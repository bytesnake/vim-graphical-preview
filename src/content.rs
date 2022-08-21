use regex::Regex;
use std::path::{Path, PathBuf};
use std::collections::BTreeMap;
use std::thread;
use std::sync::{Mutex, Arc};
use magick_rust::{MagickWand, bindings::{ColorspaceType_GRAYColorspace, DitherMethod_NoDitherMethod}};

use crate::error::{Error, Result};
use crate::render::{FoldState, Fold, FoldInner, ART_PATH, CodeId};
use crate::node_view::NodeView;
use crate::utils;

#[derive(Debug)]
pub enum ContentType {
    Math,
    Gnuplot,
    Tex,
    File,
    Header,
}

impl ContentType {
    pub fn generate(&self, content: String) -> Result<MagickWand> {
        let path = self.path(&content);
        let missing = !path.exists();

        if missing {
            match self {
                ContentType::Math => {
                    utils::parse_equation(&content, 1.0)?;
                },
                ContentType::File => {
                    return Err(Error::FileNotFound(path))
                },
                _  => panic!("Not supported {:?}", self),
            }
        }

        use std::time::Instant;
        let now = Instant::now();

        let wand = MagickWand::new();
        wand.set_resolution(600.0, 600.0).unwrap();

        wand.read_image(path.to_str().unwrap()).unwrap();
        //wand.set_compression_quality(5).unwrap();
        //wand.transform_image_colorspace(ColorspaceType_GRAYColorspace).unwrap();
        //wand.quantize_image(8, ColorspaceType_GRAYColorspace, 0, DitherMethod_NoDitherMethod, 0).unwrap();

        let dur = now.elapsed();
        //println!("{:?}", dur);

        Ok(wand)
    }

    pub fn path(&self, content: &str) -> PathBuf {
        let id = utils::hash(content);
        match self {
            ContentType::Math => PathBuf::from(ART_PATH).join(id).with_extension("svg"),
            ContentType::File => PathBuf::from(content),
            _ => panic!("Not supported {:?}", self),
        }
    }
}

pub enum NodeFile {
    Running,
    Done(Result<MagickWand>),
}

impl NodeFile {
    pub fn new(content: &str, kind: ContentType) -> Arc<Mutex<NodeFile>> {
        let state = Arc::new(Mutex::new(NodeFile::Running));
        let state_clone = state.clone();
        let content = content.to_string();

        thread::spawn(move || {
            let res = kind.generate(content);
            *state_clone.lock().unwrap() = Self::Done(res);
        });

        state
    }
}

unsafe impl Send for NodeFile {}

pub struct Node {
    pub id: CodeId,
    file: Arc<Mutex<NodeFile>>,
    pub range: (usize, usize),
}

impl Node {
    pub fn new(id: CodeId, range: (usize, usize), content: &str, kind: ContentType) -> Node {
        let file = NodeFile::new(content, kind);

        Node {
            id, file, range
        }
    }

    pub fn available(&mut self) -> Option<Result<MagickWand>> {
        let content = std::mem::replace(&mut *self.file.lock().unwrap(), NodeFile::Running);

        match content {
            NodeFile::Running => None,
            NodeFile::Done(file) => {
                if let Ok(file) = &file {
                    *self.file.lock().unwrap() = NodeFile::Done(Ok(file.clone()));
                }

                Some(file)
            }
        }
    }
}

pub struct Content {
    math_regex: Regex,
    gnuplot_regex: Regex,
    tex_regex: Regex,
    file_regex: Regex,
    header_regex: Regex,
    newlines: Regex,
}

impl Content {
    pub fn new() -> Content {
        Content {
            math_regex: Regex::new(r"\n```math(,height=(?P<height>([\d]+)?))?[\w]*\n(?P<inner>[\s\S]+?)?```").unwrap(),
            gnuplot_regex: Regex::new(r"```gnuplot(,height=(?P<height>[\d]+?))?[\w]*\n(?P<inner>[\s\S]+?)?```").unwrap(),
            tex_regex: Regex::new(r"```tex(,height=(?P<height>[\d]+?))?[\w]*\n(?P<inner>[\s\S]+?)?```").unwrap(),
            file_regex: Regex::new(r#"\n(?P<alt>!\[[^\]]*\])\((?P<file_name>.*?)\)(?P<new_lines>\n*)"#).unwrap(),
            header_regex: Regex::new(r"\n(#{1,6}.*)").unwrap(),
            newlines: Regex::new(r"\n").unwrap(),
        }
    }

    pub fn process(&self, content: &str, mut old_nodes: BTreeMap<String, Node>) -> (BTreeMap<String, Node>, BTreeMap<usize, FoldInner>, Vec<usize>, bool) {
        // put new lines into a btree map for later
        let (_, mut new_lines) = self.newlines.find_iter(content)
            .map(|x| x.start())
            .fold((1, BTreeMap::new()), |(mut nr, mut map): (usize, BTreeMap<usize, usize>), idx| {
                nr += 1;
                map.insert(idx, nr);

                (nr, map)
            });
        new_lines.insert(1, 1);

        let folds = self.header_regex.find_iter(content)
            .filter_map(|x| new_lines.get(&x.start()))
            .copied()
            .collect::<Vec<_>>();

        let mut nodes = BTreeMap::new();
        let mut any_changed = false;
        let maths = self.math_regex.captures_iter(content)
            .map(|x| {
                let content = x.name("inner").map_or("", |x| x.as_str()).to_string();
                let height = x.name("height")
                    .and_then(|x| x.as_str().parse::<usize>().ok())
                    .unwrap_or_else(|| content.matches('\n').count() + 1);
                let line = new_lines.get(&x.get(0).unwrap().start()).unwrap();
                let id = utils::hash(&content);

                (height, *line, content, id, ContentType::Math)
            });

        let picts = self.file_regex.captures_iter(content)
            .map(|x| {
                let file_name = x.name("file_name").unwrap().as_str().to_string();
                let height = x.name("new_lines").unwrap().as_str().len() - 1;
                let line = new_lines.get(&x.get(0).unwrap().start()).unwrap() + 1;
                let id = utils::hash(&file_name);

                (height, line, file_name, id, ContentType::File)
            });

        let maths = maths.chain(picts)
            .map(|(height, line, content, id, kind)| {
                let new_range = (line, line + height);

                // try to load from existing structures
                if let Some(mut node) = old_nodes.remove(&id) {
                    if new_range != node.range {
                        any_changed = true;
                    }
                    node.range = new_range;

                    nodes.insert(id.clone(), node);
                } else {
                    any_changed = true;

                    nodes.insert(id.clone(), Node::new(id.clone(), new_range, &content, kind));
                }

                (line, FoldInner::Node((id, NodeView::Hidden)))
            });

        let strcts = folds.iter()
            .map(|line| {
                let new_fold = Fold {
                    state: FoldState::Open,
                    line: *line,
                };
                (*line, FoldInner::Fold(new_fold))
            })
            .chain(maths)
            .collect::<BTreeMap<_, _>>();

        //dbg!(&strcts);

        (nodes, strcts, folds, any_changed)
    }

}

