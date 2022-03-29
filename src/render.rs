use std::io::{Write, Stdout};
use std::collections::BTreeMap;
use std::path::Path;
use std::thread;
use std::fs::File;
use std::os::unix::io::FromRawFd;

use regex::Regex;
use miniserde::{json, Serialize, Deserialize};
use magick_rust::MagickWand;

use crate::error::Result;
use crate::utils;
use crate::node_view::NodeView;

const ART_PATH: &'static str = "/tmp/nvim_arts/";

pub type CodeId = String;
pub type Folds = Vec<(usize, isize)>;

#[derive(Debug, Deserialize)]
pub struct Metadata {
    pub file_range: (u64, u64),
    pub viewport: (u64, u64),
    pub cursor: u64,
    pub winpos: (usize, usize),
}

impl Metadata {
    pub fn new() -> Metadata {
        Metadata {
            file_range: (1, 1),
            viewport: (1, 1),
            cursor: 1,
            winpos: (1, 1),
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
    line: usize,
    state: FoldState,
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

pub struct NodeFile {
    file: Option<MagickWand>
}

impl NodeFile {
    pub fn new(path: &Path) -> NodeFile {
        // check if file already exists, otherwise initiate creation
        if !path.exists() {

            NodeFile { file: None }
        } else {
            let wand = MagickWand::new();
            wand.set_resolution(500.0, 500.0).unwrap();

            wand.read_image(path.to_str().unwrap()).unwrap();
            NodeFile { file: Some(wand) }
        }
    }

    pub fn is_available(&self) -> bool {
        self.file.is_some()
    }
}

pub struct Node {
    pub id: CodeId,
    file: NodeFile,
    pub range: (usize, usize),
}

impl Node {
    pub fn new(id: CodeId, content: String, range: (usize, usize)) -> Result<Node> {
        let path = Path::new(ART_PATH).join(&id).with_extension("svg");
        let file = NodeFile::new(&path);
        if !file.is_available() {
            thread::spawn(move || { 
                if let Err(err) = crate::utils::parse_equation(&path, &content, 1.0) {
                    eprintln!("{:?}", err);
                }
            });
        }

        // create node, it's hidden bc. we want to render it next cycle
        Ok(Node {
            id, file, range, 
        })
    }

    pub fn file_available(&self) -> bool {
        Path::new(ART_PATH).join(&self.id).with_extension("svg").exists()
    }
}

pub struct Render {
    stdout: Stdout,
    fence_regex: Regex,
    header_regex: Regex,

    blocks: BTreeMap<CodeId, Node>,
    strcts: BTreeMap<usize, FoldInner>,
    metadata: Metadata
}

impl Render {
    pub fn new() -> Render {
        if !Path::new(ART_PATH).exists() {
            std::fs::create_dir(ART_PATH).unwrap();
        }

        Render {
            stdout: std::io::stdout(),
            fence_regex: Regex::new(r"```math(,height=(?P<height>[\d]+?))?[\w]*\n(?P<inner>[\s\S]+?)?```").unwrap(),
            header_regex: Regex::new(r"^(#{1,6}.*)").unwrap(),
            blocks: BTreeMap::new(),
            strcts: BTreeMap::new(),
            metadata: Metadata::new(),
        }
    }

    pub fn draw(&mut self, _: &str) -> Result<usize> {
        let mut pending = false;

        // mutable iterator of items, skipping things outside the viewport
        let mut items = self.strcts.iter_mut()
            .filter(|(_, item)| item.is_in_view(&self.metadata, &self.blocks))
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
    
        let char_height = utils::char_pixel_height();

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
        // check if file is now available
        if node.file_available() && !node.file.is_available() {
           node.file = NodeFile::new(&Path::new(ART_PATH).join(&node.id).with_extension("svg"));
        }

        let new_view = NodeView::new(node,  &metadata, top_offset);
        let img = match &node.file.file {
            Some(file) => file,
            None => {
                return Ok(new_view.is_visible());
            }
        };
        let theight = node.range.1 - node.range.0;

        let data: Option<(Vec<u8>, usize)> = match (&view, &new_view) {
            (NodeView::UpperBorder(_, _) | NodeView::LowerBorder(_, _) | NodeView::Hidden, NodeView::Visible(pos, _)) => {
                // clone and fit
                let img = img.clone();
                img.fit(100000, theight * char_height);
                Some((
                    img.write_image_blob("sixel").unwrap(),
                    *pos
                ))
            }, 
            (NodeView::Hidden, NodeView::UpperBorder(y, height)) => {
                // clone and crop
                let img = img.clone();
                img.fit(100000, theight * char_height);
                img.crop_image(img.get_image_width(), height * char_height, 0, (y * char_height) as isize).unwrap();
                Some((
                    img.write_image_blob("sixel").unwrap(),
                    0
                ))
            },
            (NodeView::UpperBorder(y_old, _), NodeView::UpperBorder(y, height)) if y < y_old => {
                // clone and crop
                let img = img.clone();
                img.fit(100000, theight * char_height);
                img.crop_image(img.get_image_width(), height * char_height, 0, (y * char_height) as isize).unwrap();
                Some((
                    img.write_image_blob("sixel").unwrap(),
                    0
                ))
            },
            (NodeView::Hidden, NodeView::LowerBorder(pos, height)) => {
                // clone and crop
                let img = img.clone();
                img.fit(100000, theight * char_height);
                img.crop_image(img.get_image_width(), height * char_height, 0, 0).unwrap();
                Some((
                    img.write_image_blob("sixel").unwrap(),
                    *pos
                ))
            },
            (NodeView::LowerBorder(_, height_old), NodeView::LowerBorder(pos, height)) if height_old < height => {
                // clone and crop
                let img = img.clone();
                img.fit(100000, theight * char_height);
                img.crop_image(img.get_image_width(), height * char_height, 0, 0).unwrap();
                Some((
                    img.write_image_blob("sixel").unwrap(),
                    *pos
                ))
            },
            _ => None
        };

        if node.file.is_available() {
            *view = new_view;
        }

        if let Some((mut buf, pos)) = data {
            let mut wbuf = format!("\x1b[s\x1b[{};{}H", pos + metadata.winpos.0, metadata.winpos.1).into_bytes();
            //for _ in 0..(node.range.1-node.range.0 - 1) {
            //    wbuf.extend_from_slice(b"\x1b[B\x1b[K");
            //}

            //wbuf.append(&mut format!("\x1b[{};{}H", pos + metadata.winpos.0, metadata.winpos.1).into_bytes());
            wbuf.append(&mut buf);
            wbuf.extend_from_slice(b"\x1b[u");

            {
                let outer_lock = stdout.lock();
                let mut stdout = unsafe { File::from_raw_fd(1) };
                let mut idx = 0;
                while idx < wbuf.len() {
                    match stdout.write(&wbuf[idx..]) {
                        Ok(n) => idx += n,
                        Err(err) => {eprintln!("{}", err);},
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
        let metadata: Metadata = json::from_str(metadata).unwrap();

        self.metadata = metadata;

        Ok(())
    }

    pub fn update_content(&mut self, content: &str) -> Result<String> {
        // content of code fences starting with ```math
        let mut blocks = self.fence_regex.captures_iter(content)
            .map(|x| (x.name("height").and_then(|x| x.as_str().parse::<usize>().ok()), x.name("inner").map_or("", |x| x.as_str())))
            .map(|x| (x.0, x.1.clone(), utils::hash(&x.1)));

        // collect line numbers of code fences and section headers into b tree
        let lines = content.lines().enumerate()
            .filter_map(|(id, line)| match (line.starts_with("```math,height=") || line == "```math", self.header_regex.is_match(line)) {
                (true, false) => Some((id, true)),
                (false, true) => Some((id, false)),
                _ => None,
            })
        .map(|(line, item)| (line+1, item))
            .collect::<BTreeMap<_, _>>();

        let mut any_changed = false;

        // create mapping (Id -> Node) from cache and new nodes
        let mut nodes = BTreeMap::new();
        let mut strct = BTreeMap::new();
        for (line, is_math) in &lines {
            if *is_math {
                let (height, content, id) = blocks.next().unwrap();
                if content.is_empty() {
                    continue
                }

                let height = height.unwrap_or_else(|| content.matches("\n").count() + 1);
                let new_range = (*line, *line + height);

                // try to load from existing structures
                if let Some(mut node) = self.blocks.remove(&id) {
                    if new_range != node.range {
                        any_changed = true;
                    }
                    node.range = new_range;

                    nodes.insert(id.clone(), node);
                } else {
                    any_changed = true;

                    nodes.insert(id.clone(), Node::new(id.clone(), content.into(), new_range)?);
                }

                strct.insert(*line, FoldInner::Node((id, NodeView::Hidden)));
            } else {
                let new_fold = Fold {
                    state: FoldState::Open,
                    line: *line,
                };
                strct.insert(*line, FoldInner::Fold(new_fold));
            }
        }

        self.blocks = nodes;
        self.strcts = strct;

        let ret = RedrawState {
            should_redraw: any_changed,
            update_folding: Some(lines.into_iter().filter(|x| !x.1).map(|x| x.0).collect()),
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
