use std::fs::File;
use std::io::{Read, Write, Stdout};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::thread;

use regex::Regex;
use tinyjson::JsonValue;
use sha2::{Digest, Sha256};
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

pub enum NodeFile {
    Generate,
    InMemory(MagickWand),
}

#[derive(Clone, PartialEq, Eq)]
pub enum NodeState {
    Hidden,
    UpperBorder(usize, usize),
    LowerBorder(usize, usize),
    Visible(usize),
}

impl NodeState {
    pub fn from_metadata(range: (usize, usize), obj: &Metadata) -> NodeState {
        NodeState::Hidden
    }

    pub fn should_update(&self, rhs: &NodeState) -> bool {
        self != rhs
    }
}

pub struct Node {
    id: CodeId,
    file: NodeFile,
    range: (usize, usize),
    state: NodeState
}

impl Node {
    pub fn new(content: &str, line: usize) -> Result<Node> {
        let id = utils::hash(content);
        let range = (line, line + content.matches("\n").count());

        // check if file already exists, otherwise initiate creation
        let path = Path::new(ART_PATH).join(id).with_extension("svg");
        let file = if !path.exists() {
            thread::spawn(move || { crate::utils::parse_equation(&path, content, 1.0)});

            NodeFile::Generate
        } else {
            let wand = MagickWand::new();
            wand.set_resolution(600.0, 600.0).unwrap();
            wand.read_image(path.to_str().unwrap()).unwrap();
            NodeFile::InMemory(wand)
        };

        // create node, it's hidden bc. we want to render it next cycle
        Ok(Node {
            id, file, range, 
            state: NodeState::Hidden
        })
    }

    pub fn step(&self, metadata: &Metadata) -> NodeState {
        let mut distance_upper = self.range.0 as isize - metadata.file_range.0 as isize;

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
    pub fn line_mapping(&self) -> ((usize, usize), CodeId) {
        (self.range, self.id.clone())
    }

    pub fn length(&self) -> usize {
        self.range.1 - self.range.0
    }
}

pub struct Render {
    stdout: Stdout,
    code_regex: Regex,
    blocks: HashMap<CodeId, Node>,
    block_lines: BTreeMap<(usize, usize), CodeId>,
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
            block_lines: BTreeMap::new(),
            metadata: Metadata::new(),
        }
    }

    pub fn draw(&mut self) {
        for (id, node) in self.blocks {
            let new_state = node.step(&self.metadata);
            
            let data: Option<(Vec<u8>, usize)> = match (node.file, node.state, new_state) {
                (NodeFile::InMemory(img), NodeState::Hidden, NodeState::Visible(pos)) => {
                    img.fit(100000, node.length() * CHAR_HEIGHT);
                    Some((
                        img.write_image_blob("sixel").unwrap(),
                        pos
                    ))
                }, 
                (NodeFile::InMemory(img), NodeState::Hidden, NodeState::UpperBorder(p, h) | NodeState::LowerBorder(p, h)) => {

                }
                (NodeFile::InMemory(img), NodeState::UpperBorder(p, h) | NodeState::LowerBorder(p, h), NodeState::Visible(p2)) => {
                },
                }
                _ => panic!("")
            };

            if let Some((buf, pos)) = data {
                self.stdout.write(&format!("\x1b[s\x1b[{};0H", pos+1).as_bytes()).unwrap();
                self.stdout.write(&buf).unwrap();
                self.stdout.write(b"\x1b[u").unwrap();
            }
        }

        // get images currently visible
        let visible_blocks = self.block_lines.iter().filter(|((line, length), _)| {
            (line + length) > self.metadata.file_range.0 as usize || 
                *line < self.metadata.file_range.1 as usize
        }).map(|((line, _), id)| (line, self.blocks.get(id).unwrap())).collect::<Vec<_>>();

        for (line, (img, length)) in visible_blocks {
            let mut distance_upper = *line as isize - self.metadata.file_range.0 as isize;

            let mut start = 0;
            let mut height = *length;

            if distance_upper < -(*length as isize) {
                // if we are above the upper line, just skip
                continue;
            } else if distance_upper < 0 {
                // if we are in the upper cross-over region, calculate the visible height
                start = (-distance_upper) as usize;
                height -= start;
                distance_upper = 0;
            }

            let distance_lower = self.metadata.viewport.1.max(self.metadata.file_range.1 + 1) as isize - *line as isize;

            dbg!(start, height, distance_upper, distance_lower);

            if distance_lower <= 0 {
                continue;
            } else if (distance_lower as usize) < *length {
                // remove some height if we are in the command line region
                height -= (*length as isize - distance_lower) as usize;
            }

            let buf;
            if start != 0 || height != *length {
                // clone and crop
                let img = img.clone();
                img.crop_image(img.get_image_width(), height * CHAR_HEIGHT, 0, (start * CHAR_HEIGHT) as isize).unwrap();
                buf = img.write_image_blob("sixel").unwrap();
            } else {
                img.fit(100000, *length * CHAR_HEIGHT);
                buf = img.write_image_blob("sixel").unwrap();
            }


        }
    }

    pub fn clear_all(&mut self, _: &str) -> Result<()> {
        //self.stdout.write(b"\x1b[2J").unwrap();

        Ok(())
    }

    pub fn update_metadata(&mut self, metadata: &str) -> Result<()> {
        let metadata: JsonValue = metadata.parse().unwrap();
        let metadata = Metadata::from_json(metadata);

        let redraw = metadata.file_range != self.metadata.file_range;

        self.metadata = metadata;


        if redraw {
            //dbg!(&self.metadata);
            //self.draw();
        }

        Ok(())
    }

    pub fn update_content(&mut self, content: &str) -> Result<()> {
        let blocks = self.code_regex.captures_iter(content).map(|x| x["inner"].to_string());
        let new_lines = content.split("\n").enumerate().filter(|(_, content)| content == &"```math").map(|(idx, _)| idx+1);

        // create mapping (Id -> Node) from cache and new nodes
        let new_blocks = blocks.zip(new_lines)
            .map(|(a, b)| Node::new(&a, b))
            .map(|node| node.map(|node| self.blocks.remove(&node.id).unwrap_or(node)))
            .map(|node| node.map(|node| (node.id.clone(), node)))
            .collect();

        if let Ok(new_blocks) = new_blocks {
            self.blocks = new_blocks;
            self.block_lines = new_blocks.values().map(|x| x.line_mapping()).collect();
        } else {
            new_blocks?;
        }

        //self.clear_all("");
        self.draw();

        Ok(())
    }
}
