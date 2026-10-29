// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

mod clipboard;
mod input;
mod mouse;
mod pane;
mod selection;
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

use clipboard::Target;
use input::{AltKey, Scanner, Token};
use mouse::{Button, Mouse, MouseKind};
use pane::Pane;
use selection::{Point, Selection};
use terminal::InputModes;

const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const ESC_TIMEOUT: Duration = Duration::from_millis(10);
const CHROME_STYLE: Style = Style::new().add_modifier(Modifier::DIM);
const CTRL_C: u8 = 0x03;

pub enum Event {
    Input(Vec<u8>),
    InputClosed,
    Redraw,
    Resize,
    Exited(usize),
    Paste(usize, Vec<u8>),
    Quit,
}

#[derive(Clone, Copy)]
enum Split {
    SideBySide,
    Stacked,
}

impl Split {
    fn focus_target(self, key: AltKey) -> Option<usize> {
        match (self, key) {
            (Split::SideBySide, AltKey::Left) | (Split::Stacked, AltKey::Up) => Some(0),
            (Split::SideBySide, AltKey::Right) | (Split::Stacked, AltKey::Down) => Some(1),
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
    pane_areas: Vec<Rect>,
    selection: Option<Selection>,
    selecting: bool,
    events: Sender<Event>,
}

impl App {
    fn handle_input(&mut self, bytes: &[u8]) {
        for token in self.scanner.feed(bytes) {
            match token {
                Token::Bytes(bytes) if bytes == [CTRL_C] && self.selection.is_some() => {
                    self.copy_selection();
                }
                Token::Bytes(bytes) => self.write_focused(&bytes),
                Token::AltKey(key) => self.handle_alt_key(key),
                Token::Mouse(mouse) => self.handle_mouse(mouse),
            }
        }
    }

    fn handle_alt_key(&mut self, key: AltKey) {
        if self.panes.len() == 2
            && let Some(target) = self.split.focus_target(key)
        {
            self.focus = target;
            return;
        }
        let pane = &mut self.panes[self.focus];
        let lines = match key {
            AltKey::PageUp => Some(pane.page()),
            AltKey::PageDown => Some(-pane.page()),
            _ => None,
        };
        if let Some(lines) = lines
            && pane.scroll(lines)
        {
            self.selection = None;
            return;
        }
        self.write_focused(key.bytes());
    }

    fn handle_mouse(&mut self, mouse: Mouse) {
        let kind = mouse.kind();
        if self.selecting {
            match kind {
                MouseKind::Drag(Button::Left) => return self.extend_selection(mouse),
                MouseKind::Release(Button::Left) => {
                    self.extend_selection(mouse);
                    return self.finish_selection();
                }
                _ => {}
            }
        }
        let position = Position::new(mouse.col, mouse.row);
        let Some(index) = self
            .pane_areas
            .iter()
            .position(|area| area.contains(position))
        else {
            return;
        };
        let area = self.pane_areas[index];
        let (col, row) = (mouse.col - area.x, mouse.row - area.y);
        let pane = &mut self.panes[index];
        match kind {
            MouseKind::WheelUp | MouseKind::WheelDown => {
                pane.wheel(mouse, col, row, kind == MouseKind::WheelUp);
                if self
                    .selection
                    .is_some_and(|selection| selection.pane == pane.id)
                {
                    self.selection = None;
                }
            }
            MouseKind::Press(Button::Right) if self.selection.is_some() => self.copy_selection(),
            MouseKind::Press(Button::Left) if mouse.ctrl() || !pane.wants_mouse() => {
                self.selection = Some(Selection::new(pane.id, Point { row, col }));
                self.selecting = true;
                self.focus = index;
            }
            MouseKind::Press(Button::Middle) if !pane.wants_mouse() => {
                clipboard::paste_primary(pane.id, self.events.clone());
                self.focus = index;
            }
            _ => {
                pane.forward_mouse(mouse, col, row);
                if let MouseKind::Press(_) = kind {
                    self.selection = None;
                    self.focus = index;
                }
            }
        }
    }

    fn extend_selection(&mut self, mouse: Mouse) {
        let Some(selection) = self.selection.as_mut() else {
            return;
        };
        let Some(area) = self
            .panes
            .iter()
            .position(|pane| pane.id == selection.pane)
            .and_then(|index| self.pane_areas.get(index))
        else {
            return;
        };
        selection.extend(Point {
            row: mouse.row.clamp(area.top(), area.bottom().saturating_sub(1)) - area.top(),
            col: mouse.col.clamp(area.left(), area.right().saturating_sub(1)) - area.left(),
        });
    }

    fn finish_selection(&mut self) {
        self.selecting = false;
        if self.selection.is_some_and(|selection| selection.is_empty()) {
            self.selection = None;
        } else if let Some(text) = self.selected_text() {
            clipboard::copy(text, Target::Primary);
        }
    }

    fn selected_text(&self) -> Option<String> {
        let selection = self.selection?;
        let pane = self.panes.iter().find(|pane| pane.id == selection.pane)?;
        Some(selection.text(pane.parser().screen())).filter(|text| !text.trim().is_empty())
    }

    fn copy_selection(&mut self) {
        if let Some(text) = self.selected_text() {
            clipboard::copy(text, Target::Clipboard);
        }
        self.selection = None;
    }

    fn write_focused(&mut self, bytes: &[u8]) {
        self.selection = None;
        self.panes[self.focus].write(bytes);
    }

    fn close(&mut self, id: usize) {
        if let Some(index) = self.panes.iter().position(|pane| pane.id == id) {
            self.panes.remove(index).reap();
            self.selection = None;
            self.selecting = false;
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
            pane_areas: stored_areas,
            selection,
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
                if let Some(selection) = selection.filter(|selection| selection.pane == pane.id) {
                    let buf = frame.buffer_mut();
                    for position in pane_area.positions() {
                        let point = Point {
                            row: position.y - pane_area.y,
                            col: position.x - pane_area.x,
                        };
                        if selection.contains(point) {
                            buf[position].modifier.toggle(Modifier::REVERSED);
                        }
                    }
                }
                let scrolled = parser.screen().scrollback();
                if scrolled > 0 {
                    let label = format!("+{scrolled}");
                    let width = u16::try_from(label.len()).unwrap_or(u16::MAX);
                    if width <= pane_area.width {
                        frame.buffer_mut().set_string(
                            pane_area.right() - width,
                            pane_area.y,
                            label,
                            CHROME_STYLE,
                        );
                    }
                }
            }
            if let Some(divider) = divider {
                let buf = frame.buffer_mut();
                for position in divider.positions() {
                    buf[position]
                        .set_symbol(split.divider_symbol())
                        .set_style(CHROME_STYLE);
                }
            }
            let parser = panes[*focus].parser();
            let screen = parser.screen();
            let focused_area = pane_areas[*focus];
            if !screen.hide_cursor() && screen.scrollback() == 0 {
                let (row, col) = screen.cursor_position();
                frame.set_cursor_position((
                    focused_area.x + col.min(focused_area.width.saturating_sub(1)),
                    focused_area.y + row.min(focused_area.height.saturating_sub(1)),
                ));
            }
            input_modes = InputModes::of(screen);
            drop(parser);
            *stored_areas = pane_areas;
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
        pane_areas: Vec::new(),
        selection: None,
        selecting: false,
        events,
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
                app.handle_input(&bytes);
                needs_draw = true;
                esc_deadline = app
                    .scanner
                    .has_pending()
                    .then(|| Instant::now() + ESC_TIMEOUT);
            }
            Event::Redraw => needs_draw = true,
            Event::Resize => {
                app.selection = None;
                needs_draw = true;
            }
            Event::Paste(id, bytes) => {
                if let Some(pane) = app.panes.iter_mut().find(|pane| pane.id == id) {
                    pane.paste(&bytes);
                }
                needs_draw = true;
            }
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
