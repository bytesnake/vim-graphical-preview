use crate::render::Metadata;
use crate::content::Node;

#[derive(PartialEq, Eq, Debug)]
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

        let distance_lower = metadata.viewport.0 as isize - offset;

        //dbg!(&offset, &height, &distance_lower);

        if distance_lower <= 0 {
            return NodeView::Hidden;
        } else if (distance_lower as usize) < height {
            // remove some height if we are in the command line region
            height -= (height as isize - distance_lower) as usize;
            start = offset as usize;
            return NodeView::LowerBorder(start, height);
        }

        NodeView::Visible(offset as usize, height)
    }

    pub fn is_visible(&self) -> bool {
        self != &NodeView::Hidden
    }
}
