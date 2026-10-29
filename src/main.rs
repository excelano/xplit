// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

mod input;
mod pane;
mod terminal;

use std::io::{self, ErrorKind, Read, Stdout, Write};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use std::{panic, process, thread};

use anyhow::Result;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use signal_hook::consts::{SIGHUP, SIGTERM, SIGWINCH};
use signal_hook::iterator::Signals;
use tui_term::widget::{Cursor, PseudoTerminal};

use input::{Arrow, Scanner, Token};
use pane::Pane;
use terminal::InputModes;

const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const ESC_TIMEOUT: Duration = Duration::from_millis(10);

pub enum Event {
    Input(Vec<u8>),
    InputClosed,
    Redraw,
    Resize,
    Exited(usize),
    Quit,
}

#[derive(Clone, Copy)]
enum Split {
    SideBySide,
    Stacked,
}

impl Split {
    fn focus_target(self, arrow: Arrow) -> Option<usize> {
        match (self, arrow) {
            (Split::SideBySide, Arrow::Left) | (Split::Stacked, Arrow::Up) => Some(0),
            (Split::SideBySide, Arrow::Right) | (Split::Stacked, Arrow::Down) => Some(1),
            _ => None,
        }
    }

    fn areas(self, area: Rect, pane_count: usize) -> (Vec<Rect>, Option<Rect>) {
        if pane_count < 2 {
            return (vec![area], None);
        }
        let constraints = [
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Fill(1),
        ];
        let layout = match self {
            Split::SideBySide => Layout::horizontal(constraints),
            Split::Stacked => Layout::vertical(constraints),
        };
        let [first, divider, second] = layout.areas(area);
        (vec![first, second], Some(divider))
    }

    fn divider_symbol(self) -> &'static str {
        match self {
            Split::SideBySide => "│",
            Split::Stacked => "─",
        }
    }
}

struct App {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    split: Split,
    panes: Vec<Pane>,
    focus: usize,
    scanner: Scanner,
    redraw_requested: Arc<AtomicBool>,
    input_modes: InputModes,
}

impl App {
    fn handle_input(&mut self, bytes: &[u8]) -> bool {
        let mut focus_changed = false;
        for token in self.scanner.feed(bytes) {
            match token {
                Token::Bytes(bytes) => self.write_focused(&bytes),
                Token::AltArrow(arrow) => {
                    match self
                        .split
                        .focus_target(arrow)
                        .filter(|_| self.panes.len() == 2)
                    {
                        Some(target) => {
                            focus_changed |= target != self.focus;
                            self.focus = target;
                        }
                        None => self.write_focused(arrow.bytes()),
                    }
                }
            }
        }
        focus_changed
    }

    fn write_focused(&mut self, bytes: &[u8]) {
        self.panes[self.focus].write(bytes);
    }

    fn close(&mut self, id: usize) {
        if let Some(index) = self.panes.iter().position(|pane| pane.id == id) {
            self.panes.remove(index).reap();
            if self.focus > index || self.focus >= self.panes.len() {
                self.focus = self.focus.saturating_sub(1);
            }
        }
    }

    fn draw(&mut self) -> Result<()> {
        self.redraw_requested.store(false, Ordering::Release);

        let App {
            terminal,
            split,
            panes,
            focus,
            ..
        } = self;
        let mut input_modes = InputModes::default();

        terminal.draw(|frame| {
            let (pane_areas, divider) = split.areas(frame.area(), panes.len());
            for (pane, pane_area) in panes.iter_mut().zip(&pane_areas) {
                pane.resize(grid_size(*pane_area));
                let parser = pane.parser();
                let widget = PseudoTerminal::new(parser.screen())
                    .cursor(Cursor::default().visibility(false));
                frame.render_widget(widget, *pane_area);
            }
            if let Some(divider) = divider {
                let buf = frame.buffer_mut();
                for position in divider.positions() {
                    buf[position]
                        .set_symbol(split.divider_symbol())
                        .set_style(Style::new().add_modifier(Modifier::DIM));
                }
            }
            let parser = panes[*focus].parser();
            let screen = parser.screen();
            let focused_area = pane_areas[*focus];
            if !screen.hide_cursor() {
                let (row, col) = screen.cursor_position();
                frame.set_cursor_position((
                    focused_area.x + col.min(focused_area.width.saturating_sub(1)),
                    focused_area.y + row.min(focused_area.height.saturating_sub(1)),
                ));
            }
            input_modes = InputModes::of(screen);
        })?;

        if input_modes != self.input_modes {
            let backend = self.terminal.backend_mut();
            backend.write_all(&input_modes.transition_from(self.input_modes))?;
            backend.flush()?;
            self.input_modes = input_modes;
        }
        Ok(())
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let split = match args.as_slice() {
        [] => Split::SideBySide,
        [arg] if arg == "down" => Split::Stacked,
        _ => {
            eprintln!("usage: xplit [down]");
            return ExitCode::from(2);
        }
    };

    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        terminal::restore();
        default_hook(info);
        process::exit(101);
    }));

    let result = run(split);
    terminal::restore();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xplit: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(split: Split) -> Result<()> {
    let (events, receiver) = mpsc::channel();
    let mut signals = Signals::new([SIGWINCH, SIGTERM, SIGHUP])?;
    let terminal = terminal::setup()?;
    spawn_stdin_reader(events.clone());
    let signal_events = events.clone();
    thread::spawn(move || {
        for signal in signals.forever() {
            let event = if signal == SIGWINCH {
                Event::Resize
            } else {
                Event::Quit
            };
            if signal_events.send(event).is_err() {
                return;
            }
        }
    });

    let redraw_requested = Arc::new(AtomicBool::new(false));
    let area = Rect::from((Position::ORIGIN, terminal.size()?));
    let (pane_areas, _) = split.areas(area, 2);
    let panes = pane_areas
        .iter()
        .enumerate()
        .map(|(id, pane_area)| {
            Pane::spawn(
                id,
                grid_size(*pane_area),
                events.clone(),
                Arc::clone(&redraw_requested),
            )
        })
        .collect::<Result<Vec<_>>>()?;

    let mut app = App {
        terminal,
        split,
        panes,
        focus: 0,
        scanner: Scanner::default(),
        redraw_requested,
        input_modes: InputModes::default(),
    };
    let result = event_loop(&mut app, receiver);
    for pane in &mut app.panes {
        pane.hang_up();
    }
    result
}

fn event_loop(app: &mut App, receiver: Receiver<Event>) -> Result<()> {
    let mut needs_draw = true;
    let mut last_draw: Option<Instant> = None;
    let mut esc_deadline: Option<Instant> = None;

    loop {
        let now = Instant::now();
        if esc_deadline.is_some_and(|deadline| now >= deadline) {
            let held = app.scanner.flush();
            app.write_focused(&held);
            esc_deadline = None;
        }
        let next_frame = last_draw.map(|at| at + FRAME_INTERVAL);
        if needs_draw && next_frame.is_none_or(|at| now >= at) {
            app.draw()?;
            last_draw = Some(now);
            needs_draw = false;
        }

        let draw_deadline = if needs_draw {
            last_draw.map(|at| at + FRAME_INTERVAL)
        } else {
            None
        };
        let event = match esc_deadline.into_iter().chain(draw_deadline).min() {
            Some(deadline) => {
                match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(event) => event,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => return Ok(()),
                }
            }
            None => match receiver.recv() {
                Ok(event) => event,
                Err(_) => return Ok(()),
            },
        };

        match event {
            Event::Input(bytes) => {
                needs_draw |= app.handle_input(&bytes);
                esc_deadline = app
                    .scanner
                    .has_pending()
                    .then(|| Instant::now() + ESC_TIMEOUT);
            }
            Event::Redraw | Event::Resize => needs_draw = true,
            Event::Exited(id) => {
                app.close(id);
                if app.panes.is_empty() {
                    return Ok(());
                }
                needs_draw = true;
            }
            Event::InputClosed | Event::Quit => return Ok(()),
        }
    }
}

fn spawn_stdin_reader(events: Sender<Event>) {
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if events.send(Event::Input(buf[..n].to_vec())).is_err() {
                        return;
                    }
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        let _ = events.send(Event::InputClosed);
    });
}

fn grid_size(area: Rect) -> (u16, u16) {
    (area.height.max(1), area.width.max(1))
}
