// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::env;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

use crate::Event;

#[derive(Clone, Copy)]
pub enum Target {
    Clipboard,
    Primary,
}

pub fn copy(text: String, target: Target) {
    let Some(mut command) = command(target, Direction::Copy) else {
        return;
    };
    thread::spawn(move || {
        let Ok(mut child) = command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    });
}

pub fn paste_primary(pane: usize, events: Sender<Event>) {
    let Some(mut command) = command(Target::Primary, Direction::Paste) else {
        return;
    };
    thread::spawn(move || {
        let Ok(output) = command.stdin(Stdio::null()).stderr(Stdio::null()).output() else {
            return;
        };
        if output.status.success() && !output.stdout.is_empty() {
            let _ = events.send(Event::Paste(pane, output.stdout));
        }
    });
}

#[derive(Clone, Copy, PartialEq)]
enum Direction {
    Copy,
    Paste,
}

fn command(target: Target, direction: Direction) -> Option<Command> {
    if env::var_os("WAYLAND_DISPLAY").is_some() {
        let program = match direction {
            Direction::Copy => "wl-copy",
            Direction::Paste => "wl-paste",
        };
        let mut command = Command::new(program);
        if direction == Direction::Paste {
            command.arg("--no-newline");
        }
        if let Target::Primary = target {
            command.arg("--primary");
        }
        return Some(command);
    }
    if env::var_os("DISPLAY").is_some() {
        let mut command = Command::new("xclip");
        if direction == Direction::Paste {
            command.arg("-o");
        }
        let name = match target {
            Target::Clipboard => "clipboard",
            Target::Primary => "primary",
        };
        command.args(["-selection", name]);
        return Some(command);
    }
    None
}
