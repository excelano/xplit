// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

mod clipboard;
mod control;
mod input;
mod layout;
mod mouse;
mod pane;
mod selection;
mod terminal;

use std::env;
use std::io::{self, ErrorKind, Read, Stdout, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use std::{panic, process, thread};

use anyhow::{Result, bail};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use signal_hook::consts::{SIGHUP, SIGTERM, SIGWINCH};
use signal_hook::iterator::Signals;
use tui_term::widget::{Cursor, PseudoTerminal};

use clipboard::Target;
use control::{Reply, Request, Server, Verb};
use input::{AltKey, Scanner, Token};
use layout::{Areas, Node, Split};
use mouse::{Button, Mouse, MouseKind};
use pane::{Pane, Spawner};
use selection::{Point, Selection};
use terminal::InputModes;

const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const ESC_TIMEOUT: Duration = Duration::from_millis(10);
const CHROME_STYLE: Style = Style::new().add_modifier(Modifier::DIM);
const CTRL_C: u8 = 0x03;
const BEL: u8 = 0x07;
const AUTOSCROLL_INTERVAL: Duration = Duration::from_millis(50);
const HIDE_OUTSIDE: &str = "hide only works inside xplit";
const USAGE: &str = "usage: xplit [down | solo | tray] [command [args...]]\n       xplit hide\n       xplit -- command [args...]";

pub enum Event {
    Input(Vec<u8>),
    InputClosed,
    Redraw,
    Resize,
    Exited(usize),
    Paste(usize, Vec<u8>),
    Request(Request, Reply),
    Quit,
}

#[derive(Clone, Copy)]
enum Place {
    Root,
    Beside(usize, Split),
    Tray,
}

struct App {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    layout: Option<Node>,
    tray: Option<usize>,
    hidden: Vec<usize>,
    panes: Vec<Pane>,
    focus: usize,
    scanner: Scanner,
    input_modes: InputModes,
    area: Rect,
    areas: Areas,
    selection: Option<Selection>,
    selecting: bool,
    autoscroll: Option<Mouse>,
    title: Option<String>,
    cursor_shape: u16,
    spawner: Spawner,
}

impl App {
    fn start(&mut self, request: &Request) -> Result<()> {
        match request.verb {
            Verb::Hide => bail!(HIDE_OUTSIDE),
            Verb::Solo => self.open(request, &request.command, Place::Root),
            Verb::Split(_) | Verb::Tray => {
                self.open(request, &[], Place::Root)?;
                let first = self.focus;
                let place = match request.verb {
                    Verb::Split(split) => Place::Beside(first, split),
                    _ => Place::Tray,
                };
                self.open(request, &request.command, place)?;
                if request.command.is_empty() {
                    self.focus = first;
                }
                Ok(())
            }
        }
    }

    fn open(&mut self, request: &Request, command: &[String], place: Place) -> Result<()> {
        let area = match place {
            Place::Root => self.area,
            Place::Tray if !layout::fits_tray(self.area) => {
                bail!("the window is too short for a tray")
            }
            Place::Tray => layout::tray_halves(self.area)[2],
            Place::Beside(target, split) => {
                let Some(target_area) = self.current_areas().of(target) else {
                    bail!("that pane is gone");
                };
                if !split.fits(target_area) {
                    bail!("this pane is too small to split");
                }
                split.halves(target_area)[2]
            }
        };
        let pane = self
            .spawner
            .spawn(grid_size(area), &request.cwd, &request.env, command)?;
        let id = pane.id;
        self.panes.push(pane);
        match place {
            Place::Root => self.layout = Some(Node::Pane(id)),
            Place::Beside(target, split) => {
                if let Some(layout) = &mut self.layout {
                    layout.split(target, split, id);
                }
            }
            Place::Tray => self.tray = Some(id),
        }
        self.focus = id;
        Ok(())
    }

    fn handle_request(&mut self, request: Request) -> Result<(), String> {
        match request.verb {
            Verb::Solo => Err("already running inside xplit".to_string()),
            Verb::Hide => self.hide(request.pane.unwrap_or(self.focus)),
            Verb::Tray => match self.tray {
                Some(_) if !request.command.is_empty() => {
                    Err("this screen already has a tray".to_string())
                }
                Some(tray) if self.hidden.contains(&tray) => {
                    self.restore(tray);
                    Ok(())
                }
                Some(tray) => {
                    self.focus = tray;
                    Ok(())
                }
                None => self
                    .open(&request, &request.command, Place::Tray)
                    .map_err(|error| format!("{error:#}")),
            },
            Verb::Split(split) => {
                let target = request
                    .pane
                    .filter(|id| self.panes.iter().any(|pane| pane.id == *id))
                    .unwrap_or(self.focus);
                if self.tray == Some(target) {
                    return Err("the tray cannot be split".to_string());
                }
                self.selection = None;
                self.open(&request, &request.command, Place::Beside(target, split))
                    .map_err(|error| format!("{error:#}"))
            }
        }
    }

    fn hide(&mut self, id: usize) -> Result<(), String> {
        let main_visible = self
            .panes
            .iter()
            .filter(|pane| {
                pane.id != id && Some(pane.id) != self.tray && !self.hidden.contains(&pane.id)
            })
            .count();
        if self.tray != Some(id) && main_visible == 0 {
            return Err("cannot hide the only visible pane".to_string());
        }
        let hidden_area = self.current_areas().of(id);
        self.hidden.push(id);
        if self.selection.is_some_and(|selection| selection.pane == id) {
            self.selection = None;
        }
        if self.focus == id {
            self.focus_where(hidden_area);
        }
        Ok(())
    }

    fn restore(&mut self, id: usize) {
        let Some(group) = self
            .current_areas()
            .sliver_holding(id)
            .map(|sliver| sliver.panes.clone())
        else {
            return;
        };
        let latest = self
            .hidden
            .iter()
            .rev()
            .find(|hidden| group.contains(hidden))
            .copied();
        self.hidden.retain(|hidden| !group.contains(hidden));
        self.focus = latest.unwrap_or(id);
    }

    fn focus_where(&mut self, area: Option<Rect>) {
        let areas = self.current_areas();
        let successor = area
            .and_then(|area| areas.at(area.as_position()))
            .or_else(|| areas.panes.first().copied());
        self.focus = successor.map_or(0, |(id, _)| id);
    }

    fn current_areas(&self) -> Areas {
        layout::screen(self.layout.as_ref(), self.tray, &self.hidden, self.area)
    }

    fn focused(&mut self) -> Option<&mut Pane> {
        self.panes.iter_mut().find(|pane| pane.id == self.focus)
    }

    fn cursor_position(&self) -> Position {
        let area = self.areas.of(self.focus).unwrap_or_default();
        let (row, col) = self
            .panes
            .iter()
            .find(|pane| pane.id == self.focus)
            .map_or((0, 0), |pane| pane.parser().screen().cursor_position());
        Position::new(area.x + col, area.y + row)
    }

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
        let anchor = self.cursor_position();
        if let Some(target) = layout::neighbor(&self.areas, self.focus, key, anchor) {
            if self.hidden.contains(&target) {
                self.restore(target);
            } else {
                self.focus = target;
            }
            return;
        }
        let Some(pane) = self.focused() else {
            return;
        };
        let lines = match key {
            AltKey::PageUp => Some(pane.page()),
            AltKey::PageDown => Some(-pane.page()),
            _ => None,
        };
        if let Some(lines) = lines
            && pane.scroll(lines)
        {
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
        if kind == MouseKind::Press(Button::Left)
            && let Some(sliver) = self.areas.sliver_at(position)
        {
            let id = sliver.panes[0];
            return self.restore(id);
        }
        let Some((id, area)) = self.areas.at(position) else {
            return;
        };
        let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == id) else {
            return;
        };
        let (col, row) = (mouse.col - area.x, mouse.row - area.y);
        match kind {
            MouseKind::WheelUp | MouseKind::WheelDown => {
                pane.wheel(mouse, col, row, kind == MouseKind::WheelUp);
            }
            MouseKind::Press(Button::Right) if self.selection.is_some() => self.copy_selection(),
            MouseKind::Press(Button::Left) if mouse.ctrl() || !pane.wants_mouse() => {
                let at = Point {
                    line: pane.top_line() + usize::from(row),
                    col,
                };
                let selection =
                    Selection::new(id, at, pane.alternate_screen(), pane.output_count());
                self.selection = Some(selection);
                self.selecting = true;
                self.focus = id;
            }
            MouseKind::Press(Button::Middle) if !pane.wants_mouse() => {
                clipboard::paste_primary(id, self.spawner.events.clone());
                self.focus = id;
            }
            _ => {
                pane.forward_mouse(mouse, col, row);
                if let MouseKind::Press(_) = kind {
                    self.selection = None;
                    self.focus = id;
                }
            }
        }
    }

    fn extend_selection(&mut self, mouse: Mouse) {
        let Some(selection) = self.selection.as_mut() else {
            return;
        };
        let Some(area) = self.areas.of(selection.pane) else {
            return;
        };
        let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == selection.pane) else {
            return;
        };
        let edge = if mouse.row < area.top() {
            1
        } else if mouse.row >= area.bottom() {
            -1
        } else {
            0
        };
        self.autoscroll = (edge != 0).then_some(mouse);
        if edge != 0 {
            pane.scroll(edge);
        }
        let row = mouse.row.clamp(area.top(), area.bottom().saturating_sub(1)) - area.top();
        selection.extend(Point {
            line: pane.top_line() + usize::from(row),
            col: mouse.col.clamp(area.left(), area.right().saturating_sub(1)) - area.left(),
        });
    }

    fn check_selection(&mut self) {
        let Some(selection) = self.selection.as_mut() else {
            return;
        };
        let Some(pane) = self.panes.iter().find(|pane| pane.id == selection.pane) else {
            self.selection = None;
            return;
        };
        let output = pane.output_count();
        let lost = pane.alternate_screen() != selection.alternate
            || (output != selection.output && pane.scrollback_full());
        if lost {
            self.selection = None;
        } else {
            selection.output = output;
        }
    }

    fn finish_selection(&mut self) {
        self.selecting = false;
        self.autoscroll = None;
        if self.selection.is_some_and(|selection| selection.is_empty()) {
            self.selection = None;
        } else if let Some(text) = self.selected_text() {
            clipboard::copy(text, Target::Primary);
        }
    }

    fn selected_text(&self) -> Option<String> {
        let selection = self.selection?;
        let pane = self.panes.iter().find(|pane| pane.id == selection.pane)?;
        let cols = self.areas.of(selection.pane)?.width;
        let text = selection.text(cols, |line, start, end| pane.line_text(line, start, end));
        Some(text).filter(|text| !text.trim().is_empty())
    }

    fn copy_selection(&mut self) {
        if let Some(text) = self.selected_text() {
            clipboard::copy(text, Target::Clipboard);
        }
        self.selection = None;
    }

    fn write_focused(&mut self, bytes: &[u8]) {
        self.selection = None;
        if let Some(pane) = self.focused() {
            pane.write(bytes);
        }
    }

    fn close(&mut self, id: usize) {
        let Some(index) = self.panes.iter().position(|pane| pane.id == id) else {
            return;
        };
        let closed_area = self.areas.of(id);
        self.panes.remove(index).reap();
        self.layout = self.layout.take().and_then(|layout| layout.without(id));
        self.hidden.retain(|hidden| *hidden != id);
        if self.tray == Some(id) {
            self.tray = None;
        }
        if self.layout.is_none()
            && let Some(tray) = self.tray.take()
        {
            self.layout = Some(Node::Pane(tray));
        }
        if let Some(layout) = &self.layout
            && layout
                .leaves()
                .iter()
                .all(|leaf| self.hidden.contains(leaf))
        {
            let tray = self.tray;
            self.hidden.retain(|hidden| Some(*hidden) == tray);
        }
        self.selection = None;
        self.selecting = false;
        if self.focus == id {
            self.focus_where(closed_area);
        }
    }

    fn draw(&mut self) -> Result<()> {
        self.check_selection();
        self.spawner
            .redraw_requested
            .store(false, Ordering::Release);

        let App {
            terminal,
            layout,
            tray,
            hidden,
            panes,
            focus,
            area: stored_area,
            areas: stored_areas,
            selection,
            ..
        } = self;
        let mut input_modes = InputModes::default();
        let mut title = None;
        let mut cursor_shape = 0;
        let mut bell = false;

        terminal.draw(|frame| {
            let areas = layout::screen(layout.as_ref(), *tray, hidden, frame.area());
            for &(id, pane_area) in &areas.panes {
                let Some(pane) = panes.iter_mut().find(|pane| pane.id == id) else {
                    continue;
                };
                pane.resize(grid_size(pane_area));
                let top = pane.top_line();
                let mut parser = pane.parser();
                bell |= std::mem::take(&mut parser.callbacks_mut().bell);
                let widget = PseudoTerminal::new(parser.screen())
                    .cursor(Cursor::default().visibility(false));
                frame.render_widget(widget, pane_area);
                if let Some(selection) = selection.filter(|selection| selection.pane == id) {
                    let buf = frame.buffer_mut();
                    for position in pane_area.positions() {
                        let point = Point {
                            line: top + usize::from(position.y - pane_area.y),
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
            let buf = frame.buffer_mut();
            for &(split, divider) in &areas.dividers {
                for position in divider.positions() {
                    buf[position]
                        .set_symbol(split.divider_symbol())
                        .set_style(CHROME_STYLE);
                }
            }
            for (position, symbol) in layout::junctions(&areas.dividers) {
                buf[position].set_symbol(symbol);
            }
            for sliver in &areas.slivers {
                for position in sliver.area.positions() {
                    buf[position]
                        .set_symbol(sliver.split.sliver_symbol())
                        .set_style(CHROME_STYLE);
                }
            }
            if let Some(pane) = panes.iter().find(|pane| pane.id == *focus)
                && let Some(focused_area) = areas.of(*focus)
            {
                let parser = pane.parser();
                let screen = parser.screen();
                if !screen.hide_cursor() && screen.scrollback() == 0 {
                    let (row, col) = screen.cursor_position();
                    frame.set_cursor_position((
                        focused_area.x + col.min(focused_area.width.saturating_sub(1)),
                        focused_area.y + row.min(focused_area.height.saturating_sub(1)),
                    ));
                }
                input_modes = InputModes::of(screen);
                title = parser.callbacks().title.clone();
                cursor_shape = parser.callbacks().cursor_shape;
            }
            *stored_areas = areas;
            *stored_area = frame.area();
        })?;

        let mut outward = input_modes.transition_from(self.input_modes);
        self.input_modes = input_modes;
        if let Some(title) = title.filter(|title| Some(title) != self.title.as_ref()) {
            outward.extend_from_slice(format!("\x1b]2;{title}\x07").as_bytes());
            self.title = Some(title);
        }
        if cursor_shape != self.cursor_shape {
            outward.extend_from_slice(format!("\x1b[{cursor_shape} q").as_bytes());
            self.cursor_shape = cursor_shape;
        }
        if bell {
            outward.push(BEL);
        }
        if !outward.is_empty() {
            let backend = self.terminal.backend_mut();
            backend.write_all(&outward)?;
            backend.flush()?;
        }
        Ok(())
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if let [arg] = args.as_slice()
        && (arg == "-h" || arg == "--help")
    {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let Some((verb, command)) = control::parse_args(&args) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let cwd = match env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            eprintln!("xplit: {error}");
            return ExitCode::FAILURE;
        }
    };
    let request = Request {
        verb,
        command,
        cwd,
        env: env::vars_os().collect(),
        pane: env::var(control::PANE_VAR)
            .ok()
            .and_then(|pane| pane.parse().ok()),
    };
    if let Some(socket) = env::var_os(control::SOCKET_VAR) {
        match control::send(Path::new(&socket), &request) {
            Ok(Ok(())) => return ExitCode::SUCCESS,
            Ok(Err(message)) => {
                eprintln!("xplit: {message}");
                return ExitCode::FAILURE;
            }
            Err(_) => {}
        }
    }
    if verb == Verb::Hide {
        eprintln!("xplit: {HIDE_OUTSIDE}");
        return ExitCode::FAILURE;
    }

    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        terminal::restore();
        default_hook(info);
        process::exit(101);
    }));

    let result = run(&request);
    terminal::restore();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xplit: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(request: &Request) -> Result<()> {
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

    let server = Server::start(events.clone());
    let socket = server.as_ref().map(|server| server.path().to_path_buf());
    let area = Rect::from((Position::ORIGIN, terminal.size()?));
    let mut app = App {
        terminal,
        layout: None,
        tray: None,
        hidden: Vec::new(),
        panes: Vec::new(),
        focus: 0,
        scanner: Scanner::default(),
        input_modes: InputModes::default(),
        area,
        areas: Areas::default(),
        selection: None,
        selecting: false,
        autoscroll: None,
        title: None,
        cursor_shape: 0,
        spawner: Spawner::new(events, Arc::new(AtomicBool::new(false)), socket),
    };
    let result = app
        .start(request)
        .and_then(|()| event_loop(&mut app, receiver));
    for pane in &mut app.panes {
        pane.hang_up();
    }
    result
}

fn event_loop(app: &mut App, receiver: Receiver<Event>) -> Result<()> {
    let mut needs_draw = true;
    let mut last_draw: Option<Instant> = None;
    let mut esc_deadline: Option<Instant> = None;
    let mut autoscroll_deadline: Option<Instant> = None;

    loop {
        let now = Instant::now();
        if esc_deadline.is_some_and(|deadline| now >= deadline) {
            let held = app.scanner.flush();
            app.write_focused(&held);
            esc_deadline = None;
        }
        if autoscroll_deadline.is_some_and(|deadline| now >= deadline)
            && let Some(mouse) = app.autoscroll
        {
            app.extend_selection(mouse);
            needs_draw = true;
        }
        autoscroll_deadline = match (app.autoscroll, autoscroll_deadline) {
            (None, _) => None,
            (Some(_), Some(deadline)) if now < deadline => Some(deadline),
            (Some(_), _) => Some(now + AUTOSCROLL_INTERVAL),
        };
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
        let deadline = [esc_deadline, draw_deadline, autoscroll_deadline]
            .into_iter()
            .flatten()
            .min();
        let event = match deadline {
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
            Event::Request(request, reply) => {
                let _ = reply.send(app.handle_request(request));
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
