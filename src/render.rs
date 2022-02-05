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
const CHAR_HEIGHT: usize = 30;

pub type CodeId = String;
pub type Folds = Vec<(usize, isize)>;

#[derive(Debug, Deserialize)]
pub struct Metadata {
    pub file_range: (u64, u64),
    pub viewport: (u64, u64),
    pub cursor: u64
}

impl Metadata {
    pub fn new() -> Metadata {
        Metadata {
            file_range: (1, 1),
            viewport: (1, 1),
            cursor: 1
        }
    }
}

#[derive(PartialEq, Clone)]
pub enum FoldState {
    Folded(usize),
    Open,
}

pub struct Fold {
    offset: isize,
    line: usize,
    state: FoldState,
}

pub enum FoldInner {
    Fold(Fold),
    Node((CodeId, NodeView)),
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
    pub fn new(content: String, line: usize) -> Result<Node> {
        let id = utils::hash(&content);
        let range = (line, line + content.matches("\n").count() + 1);

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
            fence_regex: Regex::new(r"```math\n(?P<inner>[\s\S]+?)```").unwrap(),
            header_regex: Regex::new(r"^(#{1,6}.*)").unwrap(),
            blocks: BTreeMap::new(),
            strcts: BTreeMap::new(),
            metadata: Metadata::new(),
        }
    }

    pub fn draw(&mut self, _: &str) -> Result<usize> {
        let mut pending = false;

        // perform fold skipping if folded in
        let mut skip_to = None;
        for (line, fold) in &mut self.strcts {
            if let Some(skip_to) = &skip_to {
                if line <= skip_to {
                    continue;
                }
            }

            match fold {
                FoldInner::Node((id, ref mut node_view)) => {
                    let node = self.blocks.get_mut(id).unwrap();
                    pending |= Render::draw_node(&self.metadata, &self.stdout, node, node_view)?;
                },
                FoldInner::Fold(ref fold) => {
                    if let FoldState::Folded(end) =  fold.state {
                        skip_to = Some(end);
                    }
                }
            }
        }

        Ok(if pending { 1 } else { 0 })

    }

    pub fn draw_node(metadata: &Metadata, stdout: &Stdout, node: &mut Node, view: &mut NodeView) -> Result<bool> {
        // check if file is now available
        if node.file_available() && !node.file.is_available() {
           node.file = NodeFile::new(&Path::new(ART_PATH).join(&node.id).with_extension("svg"));
        }

        let img = match &node.file.file {
            Some(file) => file,
            None => {
                if view != &NodeView::Hidden {
                    return Ok(true);
                } else {
                    return Ok(false);
                }
            }
        };
        let theight = node.range.1 - node.range.0 + 1;
        
        let new_view = NodeView::new(node,  &metadata, 0);
        //dbg!(&node.state, &new_state);
        let data: Option<(Vec<u8>, usize)> = match (&view, &new_view) {
            (NodeView::UpperBorder(_, _) | NodeView::LowerBorder(_, _) | NodeView::Hidden, NodeView::Visible(pos, _)) => {
                img.fit(metadata.viewport.0 as usize * CHAR_HEIGHT, theight * CHAR_HEIGHT);
                Some((
                    img.write_image_blob("sixel").unwrap(),
                    *pos
                ))
            }, 
            (NodeView::Hidden, NodeView::UpperBorder(y, height)) => {
                // clone and crop
                let img = img.clone();
                img.fit(100000, theight * CHAR_HEIGHT);
                img.crop_image(img.get_image_width(), height * CHAR_HEIGHT, 0, (y * CHAR_HEIGHT) as isize).unwrap();
                Some((
                    img.write_image_blob("sixel").unwrap(),
                    0
                ))
            },
            (NodeView::UpperBorder(y_old, _), NodeView::UpperBorder(y, height)) if y < y_old => {
                // clone and crop
                let img = img.clone();
                img.fit(100000, theight * CHAR_HEIGHT);
                img.crop_image(img.get_image_width(), height * CHAR_HEIGHT, 0, (y * CHAR_HEIGHT) as isize).unwrap();
                Some((
                    img.write_image_blob("sixel").unwrap(),
                    0
                ))
            },
            (NodeView::Hidden, NodeView::LowerBorder(pos, height)) => {
                // clone and crop
                let img = img.clone();
                img.fit(100000, theight * CHAR_HEIGHT);
                img.crop_image(img.get_image_width(), height * CHAR_HEIGHT, 0, 0).unwrap();
                Some((
                    img.write_image_blob("sixel").unwrap(),
                    *pos
                ))
            },
            (NodeView::LowerBorder(_, height_old), NodeView::LowerBorder(pos, height)) if height_old < height => {
                // clone and crop
                let img = img.clone();
                img.fit(100000, theight * CHAR_HEIGHT);
                img.crop_image(img.get_image_width(), height * CHAR_HEIGHT, 0, 0).unwrap();
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
            let mut wbuf = format!("\x1b[s\x1b[{};5H", pos+1).into_bytes();
            for _ in 0..(node.range.1-node.range.0) {
                wbuf.extend_from_slice(b"\x1b[B\x1b[K");
            }

            wbuf.append(&mut format!("\x1b[s\x1b[{};5H", pos+1).into_bytes());
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
        eprintln!("UPDATE METADATA");
        let metadata: Metadata = json::from_str(metadata).unwrap();

        self.metadata = metadata;

        Ok(())
    }

    pub fn update_content(&mut self, content: &str) -> Result<String> {
        let mut blocks = self.fence_regex.captures_iter(content)
            .map(|x| x["inner"].to_string())
            .map(|x| (x.clone(), utils::hash(&x)));

        // collect line numbers of code fences and section headers into b tree
        let lines = content.lines().enumerate()
            .filter_map(|(id, line)| match (line.starts_with("```math"), self.header_regex.is_match(line)) {
                (true, false) => Some((id, true)),
                (false, true) => Some((id, false)),
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();

        let mut any_changed = false;

        // TODO collect into two separate lists, all nodes and structure of file
        // create mapping (Id -> Node) from cache and new nodes
        let mut nodes = BTreeMap::new();
        let mut strct = BTreeMap::new();
        for (line, is_math) in &lines {
            if *is_math {
                let (content, id) = blocks.next().unwrap();

                let new_range = (*line, *line + content.matches("\n").count() + 1);

                // try to load from existing structures
                if let Some(mut node) = self.blocks.remove(&id) {
                    if new_range != node.range {
                        any_changed = true;
                    }
                    node.range = new_range;

                    nodes.insert(id.clone(), node);
                } else {
                    any_changed = true;

                    nodes.insert(id.clone(), Node::new(content, *line)?);
                }

                strct.insert(*line, FoldInner::Node((id, NodeView::Hidden)));
            } else {
                let new_fold = Fold {
                    offset: 0,
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

    pub fn set_folds(&mut self, folds: &str) -> Result<()> {
        let folds: Folds = json::from_str(folds).unwrap();
        let mut folds = folds.into_iter();

        let mut any_changed = false;

        // loop through structs and update fold information
        for (line, elm) in &mut self.strcts {
            match elm {
                FoldInner::Fold(ref mut fold) => {
                    let (start, end) = folds.next().unwrap();
                    let prev = fold.state.clone();

                    if end == -1 {
                        fold.state = FoldState::Open;
                    } else {
                        fold.state = FoldState::Folded(end as usize);
                    }

                    if prev != fold.state {
                        any_changed = true;
                    }
                },
                _ => {}
            }
        }

        // re-calculate 
        if any_changed {
        }

        Ok(())
    }
}
