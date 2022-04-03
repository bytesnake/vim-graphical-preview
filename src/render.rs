use std::io::{Write, Stdout};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::thread;
use std::fs::File;
use std::os::unix::io::FromRawFd;
use std::mem;

use regex::Regex;
use miniserde::{json, Serialize, Deserialize};
use magick_rust::MagickWand;

use crate::error::Result;
use crate::utils;
use crate::node_view::NodeView;
use crate::content::{Content, Node, NodeFile};

pub const ART_PATH: &'static str = "/tmp/nvim_arts/";

pub type CodeId = String;
pub type Folds = Vec<(usize, isize)>;

#[derive(Debug, Deserialize)]
pub struct Metadata {
    pub file_range: (u64, u64),
    pub viewport: (u64, u64),
    pub cursor: u64,
    pub winpos: (usize, usize),
    pub char_height: usize,
}

impl Metadata {
    pub fn new() -> Metadata {
        Metadata {
            file_range: (1, 1),
            viewport: (1, 1),
            cursor: 1,
            winpos: (1, 1),
            char_height: 0,
        }
    }
}

#[derive(PartialEq, Clone, Debug)]
pub enum FoldState {
    Folded(usize),
    Open,
}

#[derive(Debug)]
pub struct Fold {
    pub line: usize,
    pub state: FoldState,
}

#[derive(Debug)]
pub enum FoldInner {
    Fold(Fold),
    Node((CodeId, NodeView)),
}

impl FoldInner {
    pub fn is_in_view(&self, metadata: &Metadata, blocks: &BTreeMap<CodeId, Node>) -> bool {
        match self {
            FoldInner::Node((id, _)) => {
                let range = blocks.get(id).unwrap().range;
        
                range.1 as u64 >= metadata.file_range.0 &&
                    range.0 as u64 <= metadata.file_range.1
            },
            FoldInner::Fold(ref fold) => 
                fold.line as u64 >= metadata.file_range.0 &&
                    fold.line as u64 <= metadata.file_range.1
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RedrawState {
    should_redraw: bool,
    update_folding: Option<Vec<usize>>,
}

pub struct Render {
    stdout: Stdout,
    blocks: BTreeMap<CodeId, Node>,
    strcts: BTreeMap<usize, FoldInner>,
    metadata: Metadata,
    content: Content,
}

impl Render {
    pub fn new() -> Render {
        if !Path::new(ART_PATH).exists() {
            std::fs::create_dir(ART_PATH).unwrap();
        }

        Render {
            stdout: std::io::stdout(),
            blocks: BTreeMap::new(),
            strcts: BTreeMap::new(),
            metadata: Metadata::new(),
            content: Content::new(),
        }
    }

    pub fn draw(&mut self, _: &str) -> Result<usize> {
        let mut pending = false;

        // mutable iterator of items, skipping things outside the viewport
        let mut items = self.strcts.iter_mut()
            .map(|(a, item)| {
                if !item.is_in_view(&self.metadata, &self.blocks) {
                    if let FoldInner::Node((_, ref mut view)) = item {
                        *view = NodeView::Hidden;
                    }
                }

                (a, item)
            })
            .filter(|(_, item)| {
                item.is_in_view(&self.metadata, &self.blocks)
            })
            .collect::<Vec<_>>();

        // initialize current item
        let mut iter = items.iter_mut();
        let mut item = match iter.next() {
            Some(x) => x,
            None => return Ok(0)
        };

        // initialize last line and top offset, so that first iteration gives offset to first item
        let mut last_line = self.metadata.file_range.0 as usize;
        let mut top_offset: isize = 0;
        let char_height = self.metadata.char_height;
    
        // perform fold skipping if folded in
        let mut skip_to = None;
        'outer: loop {
            match item.1 {
                FoldInner::Node((id, ref mut node_view)) => {
                    let node = self.blocks.get_mut(id).unwrap();

                    // calculate new offset (this can be negative at the beginning)
                    top_offset += node.range.0 as isize - last_line as isize;
                    last_line = node.range.0;

                    pending |= Render::draw_node(&self.metadata, &self.stdout, node, node_view, top_offset, char_height)?;
                },
                FoldInner::Fold(ref fold) => {
                    // offset has a header of single line
                    top_offset += fold.line as isize - last_line as isize;

                    if let FoldState::Folded(end) =  fold.state {
                        skip_to = Some(end);
                        
                        last_line = end;
                    } else {
                        last_line = fold.line;
                    }
                }
            }

            // get new items and skip until line is reached
            loop {
                item = match iter.next() {
                    Some(x) => x,
                    None => break 'outer
                };

                if let Some(skip_line) = skip_to.take() {
                    if *item.0 <= skip_line {
                        skip_to = Some(skip_line);
                        continue;
                    }
                }

                break;
            }

        }

        Ok(if pending { 1 } else { 0 })
    }
    pub fn draw_node(metadata: &Metadata, stdout: &Stdout, node: &mut Node, view: &mut NodeView, top_offset: isize, char_height: usize) -> Result<bool> {
        let new_view = NodeView::new(node,  &metadata, top_offset);

        // check if available and extract from shared mutex
        let img = match node.available() {
            Some(Ok(img)) => img,
            Some(Err(err)) => return Err(err),
            None => {
                return Ok(new_view.is_visible());
            }
        };
        let theight = node.range.1 - node.range.0;

        let data: Option<(Vec<u8>, usize)> = match (&view, &new_view) {
            (NodeView::UpperBorder(_, _) | NodeView::LowerBorder(_, _) | NodeView::Hidden, NodeView::Visible(pos, _)) => {
                // clone and fit
                img.fit(100000, theight * char_height);
                Some((
                    img.write_image_blob("sixel").unwrap(),
                    *pos
                ))
            }, 
            (NodeView::Hidden, NodeView::UpperBorder(y, height)) => {
                // clone and crop
                img.fit(100000, theight * char_height);
                img.crop_image(img.get_image_width(), height * char_height, 0, (y * char_height) as isize).unwrap();
                Some((
                    img.write_image_blob("sixel").unwrap(),
                    0
                ))
            },
            (NodeView::UpperBorder(y_old, _), NodeView::UpperBorder(y, height)) if y < y_old => {
                // clone and crop
                img.fit(100000, theight * char_height);
                img.crop_image(img.get_image_width(), height * char_height, 0, (y * char_height) as isize).unwrap();
                Some((
                    img.write_image_blob("sixel").unwrap(),
                    0
                ))
            },
            (NodeView::Hidden, NodeView::LowerBorder(pos, height)) => {
                // clone and crop
                img.fit(100000, theight * char_height);
                img.crop_image(img.get_image_width(), height * char_height, 0, 0).unwrap();
                Some((
                    img.write_image_blob("sixel").unwrap(),
                    *pos
                ))
            },
            (NodeView::LowerBorder(_, height_old), NodeView::LowerBorder(pos, height)) if height_old < height => {
                // clone and crop
                img.fit(100000, theight * char_height);
                img.crop_image(img.get_image_width(), height * char_height, 0, 0).unwrap();
                Some((
                    img.write_image_blob("sixel").unwrap(),
                    *pos
                ))
            },
            _ => None
        };

        //if node.file.is_available() {
            //*view = new_view;
        //}

        if let Some((mut buf, pos)) = data {
            //dbg!(&metadata.viewport.0, &metadata.winpos.1);
            let mut wbuf = format!("\x1b[s\x1b[{};{}H", pos + metadata.winpos.0, metadata.winpos.1).into_bytes();
            //for _ in 0..(node.range.1-node.range.0 - 1) {
            //    wbuf.extend_from_slice(b"\x1b[B\x1b[K");
            //}

            //wbuf.append(&mut format!("\x1b[{};{}H", pos + metadata.winpos.0, metadata.winpos.1).into_bytes());
            wbuf.append(&mut buf);
            //wbuf.append(&mut format!("\x1b[{};{}H", metadata.viewport.0, metadata.winpos.1).into_bytes());
            //wbuf.append(&mut format!("\x1b[?80h\x1bP100;1q\"1;1;2000;50\"1;1;2000;50\x1b[u\x1b\\").into_bytes());
            //wbuf.extend_from_slice(b"\x1b[u");
            wbuf.extend_from_slice(b"\x1b[u");

            {
                let outer_lock = stdout.lock();
                let mut stdout = unsafe { File::from_raw_fd(1) };
                let mut idx = 0;
                while idx < wbuf.len() {
                    match stdout.write(&wbuf[idx..]) {
                        Ok(n) => idx += n,
                        Err(_) => {/*eprintln!("{}", err);*/},
                    }
                }
                std::mem::forget(stdout);
                drop(outer_lock);
            }
        }

        Ok(false)
    }

    pub fn clear_all(&mut self, _: &str) -> Result<()> {
        for (_, fold) in &mut self.strcts {
            if let FoldInner::Node(ref mut node) = fold {
                node.1 = NodeView::Hidden;
            }
        }

        Ok(())
    }

    pub fn update_metadata(&mut self, metadata: &str) -> Result<()> {
        let mut metadata: Metadata = json::from_str(metadata).unwrap();
        metadata.char_height = utils::char_pixel_height();

        let rerender = metadata.viewport != self.metadata.viewport;
        if rerender {
            self.clear_all("")?;
        }

        self.metadata = metadata;

        Ok(())
    }

    pub fn update_content(&mut self, content: &str) -> Result<String> {
        let old_blocks = mem::replace(&mut self.blocks, BTreeMap::new());
        let (nodes, strcts, folds, any_changed) = self.content.process(content, old_blocks);

        self.strcts = strcts;
        self.blocks = nodes;

        let ret = RedrawState {
            should_redraw: any_changed,
            update_folding: Some(folds),
        };

        Ok(json::to_string(&ret))
    }

    pub fn set_folds(&mut self, folds: &str) -> Result<usize> {
        let folds: Folds = json::from_str(folds).unwrap();
        let mut folds = folds.into_iter();

        let mut any_changed = false;

        // loop through structs and update fold information
        let mut end_fold: Option<usize> = None;
        for (line, elm) in &mut self.strcts {
            if let Some(tmp) = &end_fold {
                if tmp < line {
                    end_fold = None;
                }
            }

            match elm {
                FoldInner::Fold(ref mut fold) => {
                    let (start, end) = folds.next().unwrap();
                    assert!(*line == start);

                    let prev = fold.state.clone();

                    if end == -1 {
                        fold.state = FoldState::Open;
                    } else {
                        fold.state = FoldState::Folded(end as usize);

                        if prev == FoldState::Open {
                            end_fold = Some(end as usize);
                        }
                    }

                    if prev != fold.state {
                        any_changed = true;
                    }
                },
                FoldInner::Node((_, ref mut view)) => {
                    if let Some(tmp) = &end_fold {
                        if line < tmp {
                            *view = NodeView::Hidden;
                        }
                    }
                }
            }
        }

        Ok(if any_changed { 1 } else { 0 })
    }
}
