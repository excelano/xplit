// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use anyhow::{Result, bail};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::Event;
use crate::control;
use crate::mouse::Mouse;

const SCROLLBACK_LINES: usize = 10_000;
const WHEEL_LINES: u8 = 3;

pub struct Pane {
    pub id: usize,
    size: (u16, u16),
    parser: Arc<Mutex<vt100::Parser>>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

pub struct Spawner {
    pub events: Sender<Event>,
    pub redraw_requested: Arc<AtomicBool>,
    pub socket: Option<PathBuf>,
    next_id: usize,
}

impl Spawner {
    pub fn new(
        events: Sender<Event>,
        redraw_requested: Arc<AtomicBool>,
        socket: Option<PathBuf>,
    ) -> Spawner {
        Spawner {
            events,
            redraw_requested,
            socket,
            next_id: 0,
        }
    }

    pub fn spawn(
        &mut self,
        size: (u16, u16),
        cwd: &Path,
        env: &[(OsString, OsString)],
        program: &[String],
    ) -> Result<Pane> {
        if let [name, ..] = program {
            let path = env
                .iter()
                .find(|(key, _)| key == "PATH")
                .map(|(_, value)| value.as_os_str());
            if !is_runnable(name, path, cwd) {
                bail!("{name}: command not found");
            }
        }
        let id = self.next_id;
        let pane = Pane::spawn(id, size, self.command(id, cwd, env, program), self)?;
        self.next_id += 1;
        Ok(pane)
    }

    fn command(
        &self,
        id: usize,
        cwd: &Path,
        env: &[(OsString, OsString)],
        program: &[String],
    ) -> CommandBuilder {
        let shell = env
            .iter()
            .find(|(key, _)| key == "SHELL")
            .map_or_else(|| "/bin/bash".into(), |(_, value)| value.clone());
        let mut command = match program {
            [] => CommandBuilder::new(shell),
            argv => CommandBuilder::from_argv(argv.iter().map(Into::into).collect()),
        };
        command.env_clear();
        for (key, value) in env {
            command.env(key, value);
        }
        command.cwd(cwd);
        command.env("TERM", "xterm-256color");
        match &self.socket {
            Some(socket) => {
                command.env(control::SOCKET_VAR, socket);
                command.env(control::PANE_VAR, id.to_string());
            }
            None => {
                command.env_remove(control::SOCKET_VAR);
                command.env_remove(control::PANE_VAR);
            }
        }
        command
    }
}

impl Pane {
    fn spawn(
        id: usize,
        size: (u16, u16),
        command: CommandBuilder,
        spawner: &Spawner,
    ) -> Result<Pane> {
        let pair = native_pty_system().openpty(pty_size(size))?;
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let events = spawner.events.clone();
        let redraw_requested = Arc::clone(&spawner.redraw_requested);

        let parser = Arc::new(Mutex::new(vt100::Parser::new(
            size.0,
            size.1,
            SCROLLBACK_LINES,
        )));
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let thread_parser = Arc::clone(&parser);
        thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        thread_parser.lock().unwrap().process(&buf[..n]);
                        if !redraw_requested.swap(true, Ordering::AcqRel) {
                            let _ = events.send(Event::Redraw);
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            let _ = events.send(Event::Exited(id));
        });

        Ok(Pane {
            id,
            size,
            parser,
            master: pair.master,
            writer,
            child,
        })
    }

    pub fn write(&mut self, bytes: &[u8]) {
        self.parser().screen_mut().set_scrollback(0);
        self.send(bytes);
    }

    pub fn paste(&mut self, text: &[u8]) {
        if !self.parser().screen().bracketed_paste() {
            return self.write(text);
        }
        let mut bytes = b"\x1b[200~".to_vec();
        bytes.extend(text.iter().filter(|&&byte| byte != 0x1b));
        bytes.extend_from_slice(b"\x1b[201~");
        self.write(&bytes);
    }

    fn send(&mut self, bytes: &[u8]) {
        let _ = self
            .writer
            .write_all(bytes)
            .and_then(|()| self.writer.flush());
    }

    pub fn resize(&mut self, size: (u16, u16)) {
        if size == self.size {
            return;
        }
        self.size = size;
        self.parser
            .lock()
            .unwrap()
            .screen_mut()
            .set_size(size.0, size.1);
        let _ = self.master.resize(pty_size(size));
    }

    pub fn page(&self) -> i32 {
        self.size.0.into()
    }

    pub fn scroll(&mut self, lines: i32) -> bool {
        let mut parser = self.parser();
        let screen = parser.screen_mut();
        if screen.alternate_screen() {
            return false;
        }
        screen.set_scrollback(screen.scrollback().saturating_add_signed(lines as isize));
        true
    }

    pub fn wants_mouse(&self) -> bool {
        self.parser().screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None
    }

    pub fn forward_mouse(&mut self, mouse: Mouse, col: u16, row: u16) {
        if let Some(bytes) = self.encode_mouse(mouse, col, row) {
            self.send(&bytes);
        }
    }

    pub fn wheel(&mut self, mouse: Mouse, col: u16, row: u16, up: bool) {
        if let Some(bytes) = self.encode_mouse(mouse, col, row) {
            self.send(&bytes);
            return;
        }
        let lines = if up {
            i32::from(WHEEL_LINES)
        } else {
            -i32::from(WHEEL_LINES)
        };
        if self.scroll(lines) {
            return;
        }
        let key: &[u8] = match (up, self.parser().screen().application_cursor()) {
            (true, true) => b"\x1bOA",
            (true, false) => b"\x1b[A",
            (false, true) => b"\x1bOB",
            (false, false) => b"\x1b[B",
        };
        self.send(&key.repeat(usize::from(WHEEL_LINES)));
    }

    fn encode_mouse(&self, mouse: Mouse, col: u16, row: u16) -> Option<Vec<u8>> {
        let parser = self.parser();
        let screen = parser.screen();
        mouse.encode(
            col,
            row,
            screen.mouse_protocol_mode(),
            screen.mouse_protocol_encoding(),
        )
    }

    pub fn parser(&self) -> MutexGuard<'_, vt100::Parser> {
        self.parser.lock().unwrap()
    }

    pub fn hang_up(&mut self) {
        let _ = self.child.kill();
    }

    pub fn reap(mut self) {
        let _ = self.child.wait();
    }
}

fn pty_size((rows, cols): (u16, u16)) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn is_runnable(name: &str, path: Option<&OsStr>, cwd: &Path) -> bool {
    let candidates: Vec<PathBuf> = if name.contains('/') {
        vec![cwd.join(name)]
    } else {
        path.map(|path| {
            env::split_paths(path)
                .map(|dir| cwd.join(dir).join(name))
                .collect()
        })
        .unwrap_or_default()
    };
    candidates.iter().any(|candidate| {
        candidate
            .metadata()
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    })
}
