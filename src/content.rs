use regex::Regex;
use std::path::PathBuf;
use std::collections::{BTreeMap, HashMap};
use std::thread;
use std::sync::{RwLock, Arc};
use magick_rust::MagickWand;

use crate::error::{Error, Result};
use crate::render::{FoldState, Fold, FoldInner, ART_PATH, CodeId};
use crate::node_view::NodeView;
use crate::utils;

pub type Sixel = Vec<u8>;

#[derive(PartialEq, Eq, Hash, Debug, Clone)]
pub struct NodeDim {
    pub(crate) height: usize,
    pub(crate) crop: Option<(usize, usize)>,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum ContentType {
    Math,
    Gnuplot,
    Tex,
    File,
}

impl ContentType {
    pub fn from_fence(kind: &str) -> Result<Self> {
        match kind {
            "math" => Ok(Self::Math),
            "gnuplot" => Ok(Self::Gnuplot),
            "latex" | "tex" => Ok(Self::Tex),
            _ => Err(Error::UnknownFence(kind.to_string())),
        }
    }

    pub fn generate(&self, content: String) -> Result<WrappedWand> {
        let mut path = self.path(&content);
        let missing = !path.exists();

        if missing {
            match self {
                ContentType::Math => {
                    utils::parse_equation(&content, 1.0)?;
                },
                ContentType::File => {
                    return Err(Error::FileNotFound(path))
                },
                ContentType::Tex => {
                    utils::parse_latex(&content)?;
                },
                ContentType::Gnuplot => {
                    let path = utils::generate_latex_from_gnuplot(&content)?;
                    utils::generate_svg_from_latex(&path, 1.0)?;
                },
            }
        }

        // rewrite path if ending as tex or gnuplot file
        if *self == ContentType::File {
            if path.extension().unwrap() == "tex" {
                path = utils::parse_latex_from_file(&path)?;
            }

            if path.extension().unwrap() == "plt" {
                let new_path = utils::generate_latex_from_gnuplot_file(&path)?;
                path = new_path.with_extension("svg");
            }
        }

        let wand = MagickWand::new();
        wand.set_resolution(600.0, 600.0).unwrap();

        wand.read_image(path.to_str().unwrap())
            .map_err(|_| Error::InvalidImage(path.to_str().unwrap().to_string()))?;

        //wand.set_compression_quality(5).unwrap();
        //wand.transform_image_colorspace(ColorspaceType_GRAYColorspace).unwrap();
        //wand.quantize_image(8, ColorspaceType_GRAYColorspace, 0, DitherMethod_NoDitherMethod, 0).unwrap();

        Ok(WrappedWand(wand))
    }
    
    pub fn path(&self, content: &str) -> PathBuf {
        let id = utils::hash(content);
        match self {
            ContentType::File => PathBuf::from(content),
            _ => PathBuf::from(ART_PATH).join(id).with_extension("svg"),
        }
    }
}

#[derive(Clone)]
pub struct WrappedWand(MagickWand);

impl WrappedWand {
    pub fn wand_to_sixel(self, dim: NodeDim) -> Vec<u8> {
        self.0.fit(100000, dim.height);

        if let Some(crop) = dim.crop {
            self.0.crop_image(self.0.get_image_width(), crop.0, 0, crop.1 as isize).unwrap();
        }

        self.0.write_image_blob("sixel").unwrap()
    }
}

unsafe impl Send for WrappedWand {}
unsafe impl Sync for WrappedWand {}

pub enum ContentState {
    Empty,
    Running,
    Ok(WrappedWand),
    Err(Error),
}

impl ContentState {
    pub fn new() -> Shared<ContentState> {
        Arc::new(RwLock::new(ContentState::Empty))
    }
}


type Shared<T> = Arc<RwLock<T>>;

pub struct Node {
    pub id: CodeId,
    pub range: (usize, usize),
    content: (String, ContentType),
    state: Shared<ContentState>,
    sixel_cache: Shared<HashMap<NodeDim, Sixel>>,
}

impl Node {
    pub fn new(id: CodeId, range: (usize, usize), content: &str, kind: ContentType) -> Node {
        let state = ContentState::new();
        let sixel_cache = Arc::new(RwLock::new(HashMap::new()));
        let content = (content.to_string(), kind);

        Node {
            id, range, state, sixel_cache, content
        }
    }

    pub fn get_sixel(&mut self, dim: NodeDim) -> Option<Result<Sixel>> {
        let Node { sixel_cache, state, content, .. } = self;

        // first check the SIXEL blob cache
        if let Some(data) = (*sixel_cache.read().unwrap()).get(&dim) {
            return Some(Ok(data.clone()));
        }

        let state_cont = std::mem::replace(&mut *state.write().unwrap(), ContentState::Empty);

        let (res, state_cont) = match state_cont {
            ContentState::Empty => {
                let state_cloned = state.clone();
                let content = content.clone();
                thread::spawn(move || {
                    let res = content.1.generate(content.0);

                    *state_cloned.write().unwrap() = match res {
                        Ok(res) => ContentState::Ok(res),
                        Err(err) => ContentState::Err(err),
                    };
                });

                (None, ContentState::Running)
            },
            ContentState::Err(error) => 
                (Some(Err(error)), ContentState::Empty),
            ContentState::Ok(content) => {
                // start thread to calculate SIXEL blob
                let sixel_cache = sixel_cache.clone();
                let state = state.clone();

                thread::spawn(move || {
                    let res = content.clone().wand_to_sixel(dim.clone());
                    sixel_cache.write().unwrap().insert(dim, res);
                    *state.write().unwrap() = ContentState::Ok(content);
                });

                (None, ContentState::Running)
            },
            ContentState::Running => (None, ContentState::Running),
        };

        let _ = std::mem::replace(&mut *state.write().unwrap(), state_cont);

        res
    }
}

pub struct Content {
    fences_regex: Regex,
    file_regex: Regex,
    header_regex: Regex,
    newlines: Regex,
}

impl Content {
    pub fn new() -> Content {
        Content {
            fences_regex: Regex::new(r"```(?P<name>([a-z]{3,}))(,height=(?P<height>([\d]+)))?[\w]*\n(?P<inner>[\s\S]+?)?```").unwrap(),
            file_regex: Regex::new(r#"\n(?P<alt>!\[[^\]]*\])\((?P<file_name>.*?)\)(?P<new_lines>\n*)"#).unwrap(),
            header_regex: Regex::new(r"\n(#{1,6}.*)").unwrap(),
            newlines: Regex::new(r"\n").unwrap(),
        }
    }

    pub fn process(&self, content: &str, mut old_nodes: BTreeMap<String, Node>) -> Result<(BTreeMap<String, Node>, BTreeMap<usize, FoldInner>, Vec<usize>, bool)> {
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

        let maths = self.fences_regex.captures_iter(content)
            .map(|x| {
                let kind = x.name("name").unwrap().as_str();
                let content = x.name("inner").map_or("", |x| x.as_str()).to_string();
                let height = x.name("height")
                    .and_then(|x| x.as_str().parse::<usize>().ok())
                    .unwrap_or_else(|| content.matches('\n').count() + 1);
                let line = new_lines.get(&(x.get(0).unwrap().start() - 1)).unwrap();
                let id = utils::hash(&content);

                ContentType::from_fence(kind).map(|c|
                    (height, *line, content, id, c)
                )
            });

        let files = self.file_regex.captures_iter(content)
            .map(|x| {
                let file_name = x.name("file_name").unwrap().as_str().to_string();
                let height = x.name("new_lines").unwrap().as_str().len() - 1;
                let line = new_lines.get(&x.get(0).unwrap().start()).unwrap() + 1;
                let id = utils::hash(&file_name);

                Ok((height, line, file_name, id, ContentType::File))
            });

        let strcts_gen = maths.chain(files)
            .map(|x| x.map(|(height, line, content, id, kind)| {
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
            }));

        let strcts = folds.iter()
            .map(|line| {
                let new_fold = Fold {
                    state: FoldState::Open,
                    line: *line,
                };
                Ok((*line, FoldInner::Fold(new_fold)))
            })
            .chain(strcts_gen)
            .collect::<Result<BTreeMap<_, _>>>()?;

        //dbg!(&strcts);

        Ok((nodes, strcts, folds, any_changed))
    }

}

