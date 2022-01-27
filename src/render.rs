use std::io::{Write, Stdout};
use std::collections::HashMap;
use std::path::Path;
use std::thread;
use std::fs::File;
use std::os::unix::io::FromRawFd;

use regex::Regex;
use tinyjson::JsonValue;
use magick_rust::MagickWand;

use crate::error::Result;
use crate::utils;

const ART_PATH: &'static str = "/tmp/nvim_arts/";
const CHAR_HEIGHT: usize = 30;
type CodeId = String;

#[derive(Debug)]
pub struct Metadata {
    file_range: (u64, u64),
    viewport: (u64, u64),
    cursor: u64
}

impl Metadata {
    pub fn from_json(data: JsonValue) -> Metadata {
        fn dec(val: &JsonValue) -> u64 {
            let num: &f64 = val.get().unwrap();
            *num as u64
        }

        Metadata {
            file_range: (dec(&data["start"]), dec(&data["end"])),
            viewport: (dec(&data["width"]), dec(&data["height"])),
            cursor: dec(&data["cursor"]),
        }
    }

    pub fn new() -> Metadata {
        Metadata {
            file_range: (1, 1),
            viewport: (1, 1),
            cursor: 1
        }
    }
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

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NodeState {
    Hidden,
    UpperBorder(usize, usize),
    LowerBorder(usize, usize),
    Visible(usize, usize),
}

pub struct Node {
    id: CodeId,
    file: NodeFile,
    range: (usize, usize),
    state: NodeState
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
            state: NodeState::Hidden
        })
    }

    pub fn step(&self, metadata: &Metadata) -> NodeState {
        let distance_upper = self.range.0 as isize - metadata.file_range.0 as isize;

        let start;
        let mut height = self.range.1 - self.range.0 + 1;

        if distance_upper <= -(height as isize) {
            // if we are above the upper line, just skip
            return NodeState::Hidden;
        } else if distance_upper < 0 {
            // if we are in the upper cross-over region, calculate the visible height
            start = (-distance_upper) as usize;
            height -= start;
            return NodeState::UpperBorder(start, height);
        }

        let distance_lower = (metadata.viewport.1).max(metadata.file_range.1) as isize - self.range.0 as isize + 1;

        if distance_lower <= 0 {
            return NodeState::Hidden;
        } else if (distance_lower as usize) < height {
            // remove some height if we are in the command line region
            height -= (height as isize - distance_lower) as usize;
            start = metadata.viewport.1 as usize - distance_lower as usize;
            return NodeState::LowerBorder(start, height);
        }

        NodeState::Visible(distance_upper as usize, height)
    }

    pub fn file_available(&self) -> bool {
        Path::new(ART_PATH).join(&self.id).with_extension("svg").exists()
    }
}

pub struct Render {
    stdout: Stdout,
    code_regex: Regex,
    blocks: HashMap<CodeId, Node>,
    metadata: Metadata
}

impl Render {
    pub fn new() -> Render {
        if !Path::new(ART_PATH).exists() {
            std::fs::create_dir(ART_PATH).unwrap();
        }

        Render {
            stdout: std::io::stdout(),
            code_regex: Regex::new(r"```math\n(?P<inner>[\s\S]+?)```").unwrap(),
            blocks: HashMap::new(),
            metadata: Metadata::new(),
        }
    }

    pub fn draw(&mut self, _: &str) -> Result<usize> {
        let mut pending = false;

        for (id, node) in &mut self.blocks {
            let new_state = node.step(&self.metadata);

            // check if file is now available
            if node.file_available() && !node.file.is_available() {
                node.file = NodeFile::new(&Path::new(ART_PATH).join(&id).with_extension("svg"));
            }

            let img = match &node.file.file {
                Some(file) => file,
                None => {
                    if new_state != NodeState::Hidden {
                        pending = true;
                    }

                    continue
                }
            };
            let theight = node.range.1 - node.range.0 + 1;
            
            //dbg!(&node.state, &new_state);
            let data: Option<(Vec<u8>, usize)> = match (&node.state, &new_state) {
                (NodeState::UpperBorder(_, _) | NodeState::LowerBorder(_, _) | NodeState::Hidden, NodeState::Visible(pos, _)) => {
                    img.fit(self.metadata.viewport.0 as usize * CHAR_HEIGHT, theight * CHAR_HEIGHT);
                    Some((
                        img.write_image_blob("sixel").unwrap(),
                        *pos
                    ))
                }, 
                (NodeState::Hidden, NodeState::UpperBorder(y, height)) => {
                    // clone and crop
                    let img = img.clone();
                    img.fit(100000, theight * CHAR_HEIGHT);
                    img.crop_image(img.get_image_width(), height * CHAR_HEIGHT, 0, (y * CHAR_HEIGHT) as isize).unwrap();
                    Some((
                        img.write_image_blob("sixel").unwrap(),
                        0
                    ))
                },
                (NodeState::UpperBorder(y_old, _), NodeState::UpperBorder(y, height)) if y < y_old => {
                    // clone and crop
                    let img = img.clone();
                    img.fit(100000, theight * CHAR_HEIGHT);
                    img.crop_image(img.get_image_width(), height * CHAR_HEIGHT, 0, (y * CHAR_HEIGHT) as isize).unwrap();
                    Some((
                        img.write_image_blob("sixel").unwrap(),
                        0
                    ))
                },
                (NodeState::Hidden, NodeState::LowerBorder(pos, height)) => {
                    // clone and crop
                    let img = img.clone();
                    img.fit(100000, theight * CHAR_HEIGHT);
                    img.crop_image(img.get_image_width(), height * CHAR_HEIGHT, 0, 0).unwrap();
                    Some((
                        img.write_image_blob("sixel").unwrap(),
                        *pos
                    ))
                },
                (NodeState::LowerBorder(_, height_old), NodeState::LowerBorder(pos, height)) if height_old < height => {
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
                let mut wbuf = format!("\x1b[s\x1b[{};0H", pos+1).into_bytes();
                for _ in 0..(node.range.1-node.range.0) {
                    wbuf.extend_from_slice(b"\x1b[B\x1b[K");
                }

                wbuf.append(&mut format!("\x1b[s\x1b[{};0H", pos+1).into_bytes());
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
            node.state = NodeState::Hidden;
        }
        //self.stdout.write(b"\x1b[2J").unwrap();

        Ok(())
    }

    pub fn update_metadata(&mut self, metadata: &str) -> Result<()> {
        let metadata: JsonValue = metadata.parse().unwrap();
        let metadata = Metadata::from_json(metadata);

        self.metadata = metadata;

        Ok(())
    }

    pub fn update_content(&mut self, content: &str) -> Result<usize> {
        let blocks = self.code_regex.captures_iter(content)
            .map(|x| x["inner"].to_string())
            .map(|x| (x.clone(), utils::hash(&x)));

        let new_lines = content.split("\n").enumerate().filter(|(_, content)| content == &"```math").map(|(idx, _)| idx+1);

        let mut any_changed = false;

        // create mapping (Id -> Node) from cache and new nodes
        let new_blocks = blocks.zip(new_lines)
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
        } else {
            new_blocks?;
        }

        Ok(if any_changed { 1 } else { 0 })
    }
}
