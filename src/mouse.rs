// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use vt100::{MouseProtocolEncoding, MouseProtocolMode};

const SGR_PREFIX: &[u8] = b"\x1b[<";
const MOTION: u16 = 32;
const WHEEL: u16 = 64;
const CTRL: u16 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mouse {
    pub code: u16,
    pub col: u16,
    pub row: u16,
    pub pressed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Press(Button),
    Release(Button),
    Drag(Button),
    WheelUp,
    WheelDown,
    Other,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Parse {
    Complete(Mouse, usize),
    Incomplete,
    NotMouse,
}

impl Mouse {
    pub fn kind(self) -> MouseKind {
        if self.code & WHEEL != 0 {
            return match self.code & 0b11 {
                0 => MouseKind::WheelUp,
                1 => MouseKind::WheelDown,
                _ => MouseKind::Other,
            };
        }
        let button = match self.code & 0b11 {
            0 => Button::Left,
            1 => Button::Middle,
            2 => Button::Right,
            _ => return MouseKind::Other,
        };
        match (self.code & MOTION != 0, self.pressed) {
            (true, _) => MouseKind::Drag(button),
            (false, true) => MouseKind::Press(button),
            (false, false) => MouseKind::Release(button),
        }
    }

    pub fn ctrl(self) -> bool {
        self.code & CTRL != 0
    }

    pub fn encode(
        self,
        col: u16,
        row: u16,
        mode: MouseProtocolMode,
        encoding: MouseProtocolEncoding,
    ) -> Option<Vec<u8>> {
        let kind = self.kind();
        let wanted = match mode {
            MouseProtocolMode::None => false,
            MouseProtocolMode::Press => {
                matches!(
                    kind,
                    MouseKind::Press(_) | MouseKind::WheelUp | MouseKind::WheelDown
                )
            }
            MouseProtocolMode::PressRelease => {
                !matches!(kind, MouseKind::Drag(_) | MouseKind::Other)
            }
            MouseProtocolMode::ButtonMotion | MouseProtocolMode::AnyMotion => {
                kind != MouseKind::Other
            }
        };
        if !wanted {
            return None;
        }
        let (x, y) = (u32::from(col) + 1, u32::from(row) + 1);
        if encoding == MouseProtocolEncoding::Sgr {
            let end = if self.pressed { 'M' } else { 'm' };
            return Some(format!("\x1b[<{};{x};{y}{end}", self.code).into_bytes());
        }
        let code = if self.pressed {
            self.code
        } else {
            self.code | 0b11
        };
        let mut bytes = b"\x1b[M".to_vec();
        for value in [u32::from(code), x, y].map(|value| value + 32) {
            if encoding == MouseProtocolEncoding::Utf8 {
                let mut utf8 = [0; 4];
                bytes.extend_from_slice(char::from_u32(value)?.encode_utf8(&mut utf8).as_bytes());
            } else {
                bytes.push(u8::try_from(value).ok()?);
            }
        }
        Some(bytes)
    }
}

pub fn parse_sgr(bytes: &[u8]) -> Parse {
    if bytes.len() < SGR_PREFIX.len() {
        return if SGR_PREFIX.starts_with(bytes) {
            Parse::Incomplete
        } else {
            Parse::NotMouse
        };
    }
    if !bytes.starts_with(SGR_PREFIX) {
        return Parse::NotMouse;
    }
    let mut fields = [0u16; 3];
    let mut field = 0;
    for (i, &byte) in bytes.iter().enumerate().skip(SGR_PREFIX.len()) {
        match byte {
            b'0'..=b'9' => {
                fields[field] = fields[field]
                    .saturating_mul(10)
                    .saturating_add(u16::from(byte - b'0'));
            }
            b';' if field < 2 => field += 1,
            b'M' | b'm' if field == 2 => {
                let mouse = Mouse {
                    code: fields[0],
                    col: fields[1].saturating_sub(1),
                    row: fields[2].saturating_sub(1),
                    pressed: byte == b'M',
                };
                return Parse::Complete(mouse, i + 1);
            }
            _ => return Parse::NotMouse,
        }
    }
    Parse::Incomplete
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mouse(code: u16, pressed: bool) -> Mouse {
        Mouse {
            code,
            col: 4,
            row: 2,
            pressed,
        }
    }

    #[test]
    fn parses_press_and_release() {
        assert_eq!(
            parse_sgr(b"\x1b[<0;5;3M"),
            Parse::Complete(mouse(0, true), 9)
        );
        assert_eq!(
            parse_sgr(b"\x1b[<0;5;3mrest"),
            Parse::Complete(mouse(0, false), 9)
        );
    }

    #[test]
    fn partial_report_is_incomplete() {
        let report = b"\x1b[<32;120;40M";
        for end in 1..report.len() {
            assert_eq!(parse_sgr(&report[..end]), Parse::Incomplete, "{end}");
        }
    }

    #[test]
    fn other_sequences_are_not_mouse() {
        for bytes in [
            &b"\x1b[A"[..],
            b"\x1b[<1;2X",
            b"\x1b[<1;2;3;4M",
            b"\x1b[<;M",
            b"\x1bx",
        ] {
            assert_eq!(parse_sgr(bytes), Parse::NotMouse, "{bytes:?}");
        }
    }

    #[test]
    fn classifies_kinds() {
        assert_eq!(mouse(0, true).kind(), MouseKind::Press(Button::Left));
        assert_eq!(mouse(2, false).kind(), MouseKind::Release(Button::Right));
        assert_eq!(mouse(32, true).kind(), MouseKind::Drag(Button::Left));
        assert_eq!(mouse(1, true).kind(), MouseKind::Press(Button::Middle));
        assert_eq!(mouse(64, true).kind(), MouseKind::WheelUp);
        assert_eq!(mouse(65, true).kind(), MouseKind::WheelDown);
        assert_eq!(mouse(35, true).kind(), MouseKind::Other);
        assert!(mouse(16, true).ctrl());
    }

    #[test]
    fn encodes_for_the_pane() {
        let sgr = MouseProtocolEncoding::Sgr;
        let press = mouse(0, true);
        assert_eq!(
            press.encode(9, 0, MouseProtocolMode::PressRelease, sgr),
            Some(b"\x1b[<0;10;1M".to_vec())
        );
        assert_eq!(
            mouse(0, false).encode(
                9,
                0,
                MouseProtocolMode::PressRelease,
                MouseProtocolEncoding::Default
            ),
            Some(b"\x1b[M#*!".to_vec())
        );
        assert_eq!(
            press.encode(
                230,
                0,
                MouseProtocolMode::PressRelease,
                MouseProtocolEncoding::Default
            ),
            None
        );
        assert_eq!(
            press.encode(
                200,
                0,
                MouseProtocolMode::PressRelease,
                MouseProtocolEncoding::Utf8
            ),
            Some("\x1b[M \u{e9}!".as_bytes().to_vec())
        );
    }

    #[test]
    fn respects_the_requested_mode() {
        let sgr = MouseProtocolEncoding::Sgr;
        let drag = mouse(32, true);
        let release = mouse(0, false);
        assert_eq!(
            mouse(0, true).encode(0, 0, MouseProtocolMode::None, sgr),
            None
        );
        assert_eq!(release.encode(0, 0, MouseProtocolMode::Press, sgr), None);
        assert_eq!(
            drag.encode(0, 0, MouseProtocolMode::PressRelease, sgr),
            None
        );
        assert!(
            drag.encode(0, 0, MouseProtocolMode::ButtonMotion, sgr)
                .is_some()
        );
    }
}
