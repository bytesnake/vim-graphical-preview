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
use crate::node_view::{NodeView, calculate_views};

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
    state: NodeView
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
            state: NodeView::Hidden
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
    ranges: BTreeMap<usize, CodeId>,

    folds: Folds,
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
            ranges: BTreeMap::new(),
            fold_ranges: Vec::new(),
            folds: BTreeMap::new(),
            metadata: Metadata::new(),
        }
    }

    pub fn draw(&mut self, _: &str) -> Result<usize> {
        let mut pending = false;

        let relevant_nodes = self.relate_to_folds();

        for (id, node) in &mut self.blocks {
            let new_state = node.step(&self.metadata, &self.folds);

            // check if file is now available
            if node.file_available() && !node.file.is_available() {
                node.file = NodeFile::new(&Path::new(ART_PATH).join(&id).with_extension("svg"));
            }

            let img = match &node.file.file {
                Some(file) => file,
                None => {
                    if new_state != NodeView::Hidden {
                        pending = true;
                    }

                    continue
                }
            };
            let theight = node.range.1 - node.range.0 + 1;
            
            //dbg!(&node.state, &new_state);
            let data: Option<(Vec<u8>, usize)> = match (&node.state, &new_state) {
                (NodeView::UpperBorder(_, _) | NodeView::LowerBorder(_, _) | NodeView::Hidden, NodeView::Visible(pos, _)) => {
                    img.fit(self.metadata.viewport.0 as usize * CHAR_HEIGHT, theight * CHAR_HEIGHT);
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
                node.state = new_state;
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
                    let outer_lock = self.stdout.lock();
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
        }

        Ok(if pending { 1 } else { 0 })
    }

    pub fn clear_all(&mut self, _: &str) -> Result<()> {
        for (_, node) in &mut self.blocks {
            node.state = NodeView::Hidden;
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
        let blocks = self.fence_regex.captures_iter(content)
            .map(|x| x["inner"].to_string())
            .map(|x| (x.clone(), utils::hash(&x)));

        let mut line_fences = Vec::new();
        let mut line_header = Vec::new();

        for (i, line) in content.lines().enumerate() {
            if line.starts_with("```math") {
                line_fences.push(i);
            } else if self.header_regex.is_match(line) {
                line_header.push(i+1);
            }
        }

        let mut any_changed = false;

        // create mapping (Id -> Node) from cache and new nodes
        let new_blocks = blocks.zip(line_fences)
            .map(|(a, b)| self.blocks.remove(&a.1)
                .map(|mut x| {
                    let new_range = (b, b + a.0.matches("\n").count() + 1);

                    if new_range != x.range {
                        any_changed = true
                    }

                    x.range = new_range;

                    Ok(x)
                })
                .unwrap_or_else(|| { any_changed = true; Node::new(a.0, b) })
            )
            .map(|node| node.map(|node| (node.id.clone(), node)))
            .collect::<Result<_>>();

        if let Ok(new_blocks) = new_blocks {
            self.blocks = new_blocks;

            // update lines
            let lines = new_blocks.iter().map(|(id, node)| (node.range.0, id.clone())).collect();
            self.ranges = lines;

        } else {
            new_blocks?;
        }

        let ret = RedrawState {
            should_redraw: any_changed,
            update_folding: Some(line_header),
        };

        Ok(json::to_string(&ret))
    }

    pub fn set_folds(&mut self, folds: &str) -> Result<()> {
        let folds: Folds = json::from_str(folds).unwrap();
        self.folds = folds;

        let mut offset = 0;
        for view in calculate_views(&self.metadata, &self.folds, &self.nodes, &mut offset) {
        }

        Ok(())
    }
}
