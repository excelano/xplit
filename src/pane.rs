// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::io::{ErrorKind, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use anyhow::Result;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::Event;

pub struct Pane {
    pub id: usize,
    size: (u16, u16),
    parser: Arc<Mutex<vt100::Parser>>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

impl Pane {
    pub fn spawn(
        id: usize,
        size: (u16, u16),
        events: Sender<Event>,
        redraw_requested: Arc<AtomicBool>,
    ) -> Result<Pane> {
        let pair = native_pty_system().openpty(pty_size(size))?;

        let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/bash".into());
        let mut command = CommandBuilder::new(shell);
        command.cwd(std::env::current_dir()?);
        command.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);

        let parser = Arc::new(Mutex::new(vt100::Parser::new(size.0, size.1, 0)));
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
