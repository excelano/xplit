// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::io::{self, Stdout, Write};

use anyhow::Result;
use crossterm::cursor::Show;
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

const MOUSE_ON: &[u8] = b"\x1b[?1002h\x1b[?1006h";
const MOUSE_OFF: &[u8] = b"\x1b[?1006l\x1b[?1002l";

pub fn setup() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    stdout.write_all(MOUSE_ON)?;
    Ok(Terminal::new(CrosstermBackend::new(io::stdout()))?)
}

pub fn restore() {
    let mut stdout = io::stdout();
    let _ = stdout.write_all(&InputModes::default().transition_from(InputModes::ALL_ON));
    let _ = stdout.write_all(MOUSE_OFF);
    let _ = execute!(stdout, Show, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputModes {
    application_cursor: bool,
    application_keypad: bool,
    bracketed_paste: bool,
}

impl InputModes {
    const ALL_ON: InputModes = InputModes {
        application_cursor: true,
        application_keypad: true,
        bracketed_paste: true,
    };

    pub fn of(screen: &vt100::Screen) -> InputModes {
        InputModes {
            application_cursor: screen.application_cursor(),
            application_keypad: screen.application_keypad(),
            bracketed_paste: screen.bracketed_paste(),
        }
    }

    pub fn transition_from(self, previous: InputModes) -> Vec<u8> {
        [
            (
                self.application_cursor,
                previous.application_cursor,
                "\x1b[?1h",
                "\x1b[?1l",
            ),
            (
                self.application_keypad,
                previous.application_keypad,
                "\x1b=",
                "\x1b>",
            ),
            (
                self.bracketed_paste,
                previous.bracketed_paste,
                "\x1b[?2004h",
                "\x1b[?2004l",
            ),
        ]
        .into_iter()
        .filter(|(now, before, _, _)| now != before)
        .flat_map(|(now, _, on, off)| if now { on } else { off }.bytes())
        .collect()
    }
}
