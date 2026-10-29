// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Point {
    pub row: u16,
    pub col: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct Selection {
    pub pane: usize,
    anchor: Point,
    head: Point,
}

impl Selection {
    pub fn new(pane: usize, at: Point) -> Selection {
        Selection {
            pane,
            anchor: at,
            head: at,
        }
    }

    pub fn extend(&mut self, to: Point) {
        self.head = to;
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    pub fn contains(&self, point: Point) -> bool {
        let (start, end) = self.bounds();
        start <= point && point <= end
    }

    pub fn text(&self, screen: &vt100::Screen) -> String {
        let (start, end) = self.bounds();
        let end_col = (end.col + 1).min(screen.size().1);
        let text = screen.contents_between(start.row, start.col, end.row, end_col);
        text.split('\n')
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn bounds(&self) -> (Point, Point) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(row: u16, col: u16) -> Point {
        Point { row, col }
    }

    #[test]
    fn contains_runs_in_reading_order() {
        let mut selection = Selection::new(0, point(3, 5));
        selection.extend(point(1, 2));
        assert!(selection.contains(point(1, 2)));
        assert!(selection.contains(point(2, 0)));
        assert!(selection.contains(point(2, 79)));
        assert!(selection.contains(point(3, 5)));
        assert!(!selection.contains(point(1, 1)));
        assert!(!selection.contains(point(3, 6)));
    }

    #[test]
    fn text_joins_wrapped_rows_and_trims_padding() {
        let mut parser = vt100::Parser::new(4, 10, 0);
        parser.process(b"abcdefghijklm\r\nshort     \r\nlast");
        let mut selection = Selection::new(0, point(0, 2));
        selection.extend(point(3, 1));
        assert_eq!(selection.text(parser.screen()), "cdefghijklm\nshort\nla");
    }

    #[test]
    fn single_cell_selection_reads_one_character() {
        let mut parser = vt100::Parser::new(2, 10, 0);
        parser.process(b"hello");
        let selection = Selection::new(0, point(0, 1));
        assert_eq!(selection.text(parser.screen()), "e");
    }
}
