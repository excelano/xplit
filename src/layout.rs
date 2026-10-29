// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use ratatui::layout::{Constraint, Layout, Position, Rect};

use crate::input::AltKey;

const MIN_WIDTH: u16 = 10;
const MIN_HEIGHT: u16 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Split {
    SideBySide,
    Stacked,
}

impl Split {
    pub fn halves(self, area: Rect) -> [Rect; 3] {
        let constraints = [
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Fill(1),
        ];
        match self {
            Split::SideBySide => Layout::horizontal(constraints).areas(area),
            Split::Stacked => Layout::vertical(constraints).areas(area),
        }
    }

    pub fn fits(self, area: Rect) -> bool {
        let [first, _, second] = self.halves(area);
        [first, second]
            .iter()
            .all(|half| half.width >= MIN_WIDTH && half.height >= MIN_HEIGHT)
    }

    pub fn divider_symbol(self) -> &'static str {
        match self {
            Split::SideBySide => "│",
            Split::Stacked => "─",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Pane(usize),
    Split {
        split: Split,
        first: Box<Node>,
        second: Box<Node>,
    },
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Areas {
    pub panes: Vec<(usize, Rect)>,
    pub dividers: Vec<(Split, Rect)>,
}

impl Areas {
    pub fn of(&self, id: usize) -> Option<Rect> {
        self.panes
            .iter()
            .find(|(pane, _)| *pane == id)
            .map(|(_, area)| *area)
    }

    pub fn at(&self, position: Position) -> Option<(usize, Rect)> {
        self.panes
            .iter()
            .copied()
            .find(|(_, area)| area.contains(position))
    }
}

impl Node {
    pub fn areas(&self, area: Rect) -> Areas {
        let mut areas = Areas::default();
        self.collect(area, &mut areas);
        areas
    }

    fn collect(&self, area: Rect, areas: &mut Areas) {
        match self {
            Node::Pane(id) => areas.panes.push((*id, area)),
            Node::Split {
                split,
                first,
                second,
            } => {
                let [first_area, divider, second_area] = split.halves(area);
                first.collect(first_area, areas);
                areas.dividers.push((*split, divider));
                second.collect(second_area, areas);
            }
        }
    }

    pub fn split(&mut self, target: usize, split: Split, new: usize) -> bool {
        match self {
            Node::Pane(id) if *id == target => {
                *self = Node::Split {
                    split,
                    first: Box::new(Node::Pane(target)),
                    second: Box::new(Node::Pane(new)),
                };
                true
            }
            Node::Pane(_) => false,
            Node::Split { first, second, .. } => {
                first.split(target, split, new) || second.split(target, split, new)
            }
        }
    }

    pub fn without(self, target: usize) -> Option<Node> {
        match self {
            Node::Pane(id) if id == target => None,
            Node::Pane(_) => Some(self),
            Node::Split {
                split,
                first,
                second,
            } => match (first.without(target), second.without(target)) {
                (Some(first), Some(second)) => Some(Node::Split {
                    split,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            },
        }
    }
}

pub fn neighbor(areas: &Areas, from: usize, key: AltKey, anchor: Position) -> Option<usize> {
    let origin = areas.of(from)?;
    let ahead = |area: Rect| -> Option<u16> {
        match key {
            AltKey::Left => origin.left().checked_sub(area.right()),
            AltKey::Right => area.left().checked_sub(origin.right()),
            AltKey::Up => origin.top().checked_sub(area.bottom()),
            AltKey::Down => area.top().checked_sub(origin.bottom()),
            AltKey::PageUp | AltKey::PageDown => None,
        }
    };
    let across = |area: Rect| -> (u16, u16, u16) {
        match key {
            AltKey::Left | AltKey::Right => (area.top(), area.bottom(), anchor.y),
            _ => (area.left(), area.right(), anchor.x),
        }
    };
    let (origin_start, origin_end, _) = across(origin);
    areas
        .panes
        .iter()
        .filter(|(id, _)| *id != from)
        .filter_map(|&(id, area)| {
            let gap = ahead(area)?;
            let (start, end, anchor) = across(area);
            let overlap = end.min(origin_end).saturating_sub(start.max(origin_start));
            (overlap > 0).then_some((gap, !(start..end).contains(&anchor), u16::MAX - overlap, id))
        })
        .min()
        .map(|(_, _, _, id)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> Node {
        let mut layout = Node::Pane(0);
        layout.split(0, Split::SideBySide, 1);
        layout.split(0, Split::Stacked, 2);
        layout.split(1, Split::Stacked, 3);
        layout
    }

    #[test]
    fn splits_the_target_pane_only() {
        let layout = quad();
        let areas = layout.areas(Rect::new(0, 0, 81, 21));
        assert_eq!(areas.of(0), Some(Rect::new(0, 0, 40, 10)));
        assert_eq!(areas.of(2), Some(Rect::new(0, 11, 40, 10)));
        assert_eq!(areas.of(1), Some(Rect::new(41, 0, 40, 10)));
        assert_eq!(areas.of(3), Some(Rect::new(41, 11, 40, 10)));
        assert_eq!(areas.dividers.len(), 3);
    }

    #[test]
    fn removing_a_pane_gives_its_space_to_the_sibling() {
        let layout = quad().without(2).unwrap();
        let areas = layout.areas(Rect::new(0, 0, 81, 21));
        assert_eq!(areas.of(0), Some(Rect::new(0, 0, 40, 21)));
        assert_eq!(areas.of(2), None);
        assert_eq!(Node::Pane(5).without(5), None);
    }

    #[test]
    fn neighbor_moves_geometrically() {
        let areas = quad().areas(Rect::new(0, 0, 81, 21));
        let top_left = Position::new(0, 0);
        assert_eq!(neighbor(&areas, 0, AltKey::Right, top_left), Some(1));
        assert_eq!(neighbor(&areas, 0, AltKey::Down, top_left), Some(2));
        assert_eq!(neighbor(&areas, 0, AltKey::Left, top_left), None);
        assert_eq!(neighbor(&areas, 0, AltKey::Up, top_left), None);
        assert_eq!(
            neighbor(&areas, 3, AltKey::Left, Position::new(50, 15)),
            Some(2)
        );
    }

    #[test]
    fn neighbor_prefers_the_pane_level_with_the_cursor() {
        let mut layout = Node::Pane(0);
        layout.split(0, Split::SideBySide, 1);
        layout.split(1, Split::Stacked, 2);
        let areas = layout.areas(Rect::new(0, 0, 81, 21));
        assert_eq!(
            neighbor(&areas, 0, AltKey::Right, Position::new(5, 3)),
            Some(1)
        );
        assert_eq!(
            neighbor(&areas, 0, AltKey::Right, Position::new(5, 18)),
            Some(2)
        );
    }

    #[test]
    fn refuses_splits_that_leave_unusable_halves() {
        assert!(Split::SideBySide.fits(Rect::new(0, 0, 21, 3)));
        assert!(!Split::SideBySide.fits(Rect::new(0, 0, 20, 3)));
        assert!(!Split::Stacked.fits(Rect::new(0, 0, 80, 6)));
        assert!(Split::Stacked.fits(Rect::new(0, 0, 80, 7)));
    }
}
