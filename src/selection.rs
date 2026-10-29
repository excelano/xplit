// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Point {
    pub line: usize,
    pub col: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct Selection {
    pub pane: usize,
    pub alternate: bool,
    pub output: u64,
    anchor: Point,
    head: Point,
}

impl Selection {
    pub fn new(pane: usize, at: Point, alternate: bool, output: u64) -> Selection {
        Selection {
            pane,
            alternate,
            output,
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

    pub fn text(
        &self,
        cols: u16,
        line_text: impl Fn(usize, u16, u16) -> Option<(String, bool)>,
    ) -> String {
        let (start, end) = self.bounds();
        let mut text = String::new();
        for line in start.line..=end.line {
            let from = if line == start.line { start.col } else { 0 };
            let to = if line == end.line { end.col + 1 } else { cols };
            let Some((content, wrapped)) = line_text(line, from, to) else {
                break;
            };
            if wrapped && line != end.line {
                text.push_str(&content);
            } else {
                text.push_str(content.trim_end());
                if line != end.line {
                    text.push('\n');
                }
            }
        }
        text
    }

    fn bounds(&self) -> (Point, Point) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(line: usize, col: u16) -> Point {
        Point { line, col }
    }

    fn selection(from: Point, to: Point) -> Selection {
        let mut selection = Selection::new(0, from, false, 0);
        selection.extend(to);
        selection
    }

    #[test]
    fn contains_runs_in_reading_order() {
        let selection = selection(point(3, 5), point(1, 2));
        assert!(selection.contains(point(1, 2)));
        assert!(selection.contains(point(2, 0)));
        assert!(selection.contains(point(2, 79)));
        assert!(selection.contains(point(3, 5)));
        assert!(!selection.contains(point(1, 1)));
        assert!(!selection.contains(point(3, 6)));
    }

    #[test]
    fn text_joins_wrapped_lines_and_trims_padding() {
        let lines = [
            ("abcdefghij", true),
            ("klm", false),
            ("short     ", false),
            ("last", false),
        ];
        let read = |line: usize, start: u16, end: u16| {
            let (text, wrapped) = lines.get(line)?;
            let end = usize::from(end).min(text.len());
            let start = usize::from(start).min(end);
            Some((text[start..end].to_string(), *wrapped))
        };
        let selection = selection(point(0, 2), point(3, 1));
        assert_eq!(selection.text(10, read), "cdefghijklm\nshort\nla");
    }

    #[test]
    fn text_stops_where_the_lines_run_out() {
        let read = |line: usize, _: u16, _: u16| (line < 2).then(|| (format!("line{line}"), false));
        let selection = selection(point(0, 0), point(5, 3));
        assert_eq!(selection.text(10, read), "line0\nline1\n");
    }
}
