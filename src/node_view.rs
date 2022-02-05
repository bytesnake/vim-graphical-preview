use std::collections::BTreeMap;
use crate::render::{Metadata, Folds, Node, CodeId};

#[derive(PartialEq)]
pub enum NodeView {
    Hidden,
    UpperBorder(usize, usize),
    LowerBorder(usize, usize),
    Visible(usize, usize),
}

impl NodeView {
    pub fn new(node: &Node, metadata: &Metadata, offset: isize) -> NodeView {
        let start;
        let mut height = node.range.1 - node.range.0 + 1;

        if offset <= -(height as isize) {
            // if we are above the upper line, just skip
            return NodeView::Hidden;
        } else if offset < 0 {
            // if we are in the upper cross-over region, calculate the visible height
            start = (-offset) as usize;
            height -= start;
            return NodeView::UpperBorder(start, height);
        }

        let distance_lower = (metadata.viewport.1).max(metadata.file_range.1) as isize - node.range.0 as isize + 1;

        if distance_lower <= 0 {
            return NodeView::Hidden;
        } else if (distance_lower as usize) < height {
            // remove some height if we are in the command line region
            height -= (height as isize - distance_lower) as usize;
            start = metadata.viewport.1 as usize - distance_lower as usize;
            return NodeView::LowerBorder(start, height);
        }

        NodeView::Visible(offset as usize, height)
    }
}

/*
pub fn calculate_views(metadata: &Metadata, folds: &Vec<Fold>, nodes: &BTreeMap<CodeId, Node>, mut offset: &mut usize) -> Vec<(CodeId, NodeView)> {
    let mut node_views = Vec::new();

    let mut last_top = 0;

    // step over nodes and folds, select 
    loop {
        match (folds.first(), nodes.first_entry()) {
            (Some(a), Some(b)) if a.range.0 > b.range.0 => {
                
            },
            (None, Some(b)) => {},
            (Some(a), None) => {},
            _ => break,
        }
    }

    for elms in &fold.inner {
        match elms {
            FoldInner::Node(ref id) => {
                let node: &Node = nodes.get(id).unwrap();

                // calculate range to last item and add to range
                let range = node.range.0 - last_top;
                *offset += range as isize;

                // create new node with offset
                let view = NodeView::new(node, metadata, *offset);
                node_views.push((node.id.clone(), view));
            },
            FoldInner::Fold(fold) => {
                // skip fold if not in view
                if fold.range.1 < metadata.file_range.0 as usize && fold.range.0 > metadata.file_range.1 as usize {
                    continue;
                }

                if fold.folded {
                    *offset += 1;
                } else {
                    node_views.append(&mut calculate_views(metadata, fold, nodes, &mut offset));
                }
            },
        }
    }

}

pub fn calculate_views(metadata: &Metadata, fold: &Fold, nodes: &BTreeMap<CodeId, Node>, mut offset: &mut isize) -> Vec<(CodeId, NodeView)> {
    let mut node_views = Vec::new();

    let mut last_top = 0;

    for elms in &fold.inner {
        match elms {
            FoldInner::Node(ref id) => {
                let node: &Node = nodes.get(id).unwrap();

                // calculate range to last item and add to range
                let range = node.range.0 - last_top;
                *offset += range as isize;

                // create new node with offset
                let view = NodeView::new(node, metadata, *offset);
                node_views.push((node.id.clone(), view));
            },
            FoldInner::Fold(fold) => {
                // skip fold if not in view
                if fold.range.1 < metadata.file_range.0 as usize && fold.range.0 > metadata.file_range.1 as usize {
                    continue;
                }

                if fold.folded {
                    *offset += 1;
                } else {
                    node_views.append(&mut calculate_views(metadata, fold, nodes, &mut offset));
                }
            },
        }
    }

    // add remaining heigth to offset
    *offset += (fold.range.1 - last_top) as isize;

    node_views
}


impl Fold {
    pub fn from_lines(nodes: &Vec<Node>, lines: &[usize, usize, usize]) -> Fold {
        for line in lines {

        }
    }
}*/
