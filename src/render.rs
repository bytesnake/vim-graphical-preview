use std::io::{Write, Stdout};
use std::collections::HashMap;
use std::path::Path;
use std::thread;

use regex::Regex;
use tinyjson::JsonValue;
use magick_rust::MagickWand;

use crate::error::{Result, Error};
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
            wand.set_resolution(600.0, 600.0).unwrap();
            wand.read_image(path.to_str().unwrap()).unwrap();
            NodeFile { file: Some(wand) }
        }
    }

    pub fn is_available(&self) -> bool {
        self.file.is_some()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum NodeState {
    Hidden,
    UpperBorder(usize, usize),
    LowerBorder(usize, usize),
    Visible(usize),
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
        let range = (line, line + content.matches("\n").count());

        let path = Path::new(ART_PATH).join(&id).with_extension("svg");
        let file = NodeFile::new(&path);
        if !file.is_available() {
            thread::spawn(move || { crate::utils::parse_equation(&path, &content, 1.0)});
        }

        // create node, it's hidden bc. we want to render it next cycle
        Ok(Node {
            id, file, range, 
            state: NodeState::Hidden
        })
    }

    pub fn step(&self, metadata: &Metadata) -> NodeState {
        let distance_upper = self.range.0 as isize - metadata.file_range.0 as isize;

        let mut start = 0;
        let mut height = self.range.1 - self.range.0;

        if distance_upper < -(height as isize) {
            // if we are above the upper line, just skip
            return NodeState::Hidden;
        } else if distance_upper < 0 {
            // if we are in the upper cross-over region, calculate the visible height
            start = (-distance_upper) as usize;
            height -= start;
            return NodeState::UpperBorder(start, height);
        }

        let distance_lower = metadata.viewport.1.max(metadata.file_range.1 + 1) as isize - self.range.1 as isize;

        dbg!(start, height, distance_upper, distance_lower);

        if distance_lower <= 0 {
            return NodeState::Hidden;
        } else if (distance_lower as usize) < height {
            // remove some height if we are in the command line region
            height -= (height as isize - distance_lower) as usize;
            return NodeState::LowerBorder(start, height);
        }

        NodeState::Visible(start)
    }

    pub fn length(&self) -> usize {
        self.range.1 - self.range.0
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

    pub fn draw(&mut self) -> Result<bool> {
        let mut pending = false;

        for (_, node) in &mut self.blocks {
            let new_state = node.step(&self.metadata);

            // check if file is now available
            if self.file.is_available() {
            }
            
            let data: Option<(Vec<u8>, usize)> = match (&node.file, &node.state, &new_state) {
                (NodeFile { file: Some(img) }, _, NodeState::Visible(pos)) => {
                    img.fit(100000, node.length() * CHAR_HEIGHT);
                    Some((
                        img.write_image_blob("sixel").unwrap(),
                        *pos
                    ))
                }, 
                (NodeFile { file: Some(img) }, NodeState::Hidden, NodeState::UpperBorder(y, height)) => {
                    // clone and crop
                    let img = img.clone();
                    img.crop_image(img.get_image_width(), height * CHAR_HEIGHT, 0, (y * CHAR_HEIGHT) as isize).unwrap();
                    Some((
                        img.write_image_blob("sixel").unwrap(),
                        0
                    ))
                },
                (NodeFile { file: Some(img) }, NodeState::Hidden, NodeState::LowerBorder(pos, height)) => {
                    // clone and crop
                    let img = img.clone();
                    img.crop_image(img.get_image_width(), height * CHAR_HEIGHT, 0, 0).unwrap();
                    Some((
                        img.write_image_blob("sixel").unwrap(),
                        *pos
                    ))
                },
                (NodeFile { file: None }, _, NodeState::UpperBorder(_, _) | NodeState::LowerBorder(_,_) | NodeState::Visible(_)) => {
                    pending = true;
                    None
                },
                _ => None
            };

            node.state = new_state;

            if let Some((buf, pos)) = data {
                self.stdout.write(&format!("\x1b[s\x1b[{};0H", pos+1).as_bytes()).unwrap();
                self.stdout.write(&buf).unwrap();
                self.stdout.write(b"\x1b[u").unwrap();
            }
        }

        Ok(pending)
    }

    pub fn clear_all(&mut self, _: &str) -> Result<()> {
        //self.stdout.write(b"\x1b[2J").unwrap();

        Ok(())
    }

    pub fn update_metadata(&mut self, metadata: &str) -> Result<bool> {
        let metadata: JsonValue = metadata.parse().unwrap();
        let metadata = Metadata::from_json(metadata);

        self.metadata = metadata;
        self.draw()
    }

    pub fn update_content(&mut self, content: &str) -> Result<bool> {
        let blocks = self.code_regex.captures_iter(content)
            .map(|x| x["inner"].to_string())
            .map(|x| (x.clone(), utils::hash(&x)));

        let new_lines = content.split("\n").enumerate().filter(|(_, content)| content == &"```math").map(|(idx, _)| idx+1);

        let mut any_changed = false;

        // create mapping (Id -> Node) from cache and new nodes
        let new_blocks = blocks.zip(new_lines)
            .map(|(a, b)| self.blocks.remove(&a.1).map(|x| Ok(x))
                .unwrap_or_else(|| { any_changed = true; Node::new(a.0, b) }))
            .map(|node| node.map(|node| (node.id.clone(), node)))
            .collect::<Result<_>>();

        if let Ok(new_blocks) = new_blocks {
            self.blocks = new_blocks;
        } else {
            new_blocks?;
        }

        Ok(any_changed)
    }
}
