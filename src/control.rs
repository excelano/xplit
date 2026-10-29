// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use crate::Event;
use crate::layout::Split;

pub const SOCKET_VAR: &str = "XPLIT_SOCKET";
pub const PANE_VAR: &str = "XPLIT_PANE";
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
const OK: &str = "ok";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Split(Split),
    Solo,
    Hide,
}

impl Verb {
    fn name(self) -> &'static str {
        match self {
            Verb::Split(Split::SideBySide) => "right",
            Verb::Split(Split::Stacked) => "down",
            Verb::Solo => "solo",
            Verb::Hide => "hide",
        }
    }

    fn from_name(name: &str) -> Option<Verb> {
        match name {
            "right" => Some(Verb::Split(Split::SideBySide)),
            "down" => Some(Verb::Split(Split::Stacked)),
            "solo" => Some(Verb::Solo),
            "hide" => Some(Verb::Hide),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub verb: Verb,
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub pane: Option<usize>,
}

pub type Reply = Sender<Result<(), String>>;

pub fn parse_args(args: &[String]) -> Option<(Verb, Vec<String>)> {
    match args {
        [] => Some((Verb::Split(Split::SideBySide), Vec::new())),
        [first, rest @ ..] if first == "--" => {
            (!rest.is_empty()).then(|| (Verb::Split(Split::SideBySide), rest.to_vec()))
        }
        [first, ..] if first.starts_with('-') => None,
        [first] if first == "hide" => Some((Verb::Hide, Vec::new())),
        [first, ..] if first == "hide" => None,
        [first, rest @ ..] => match first.as_str() {
            "down" | "solo" => {
                let verb = Verb::from_name(first)?;
                let rest = rest.strip_prefix(&["--".to_string()]).unwrap_or(rest);
                Some((verb, rest.to_vec()))
            }
            _ => Some((Verb::Split(Split::SideBySide), args.to_vec())),
        },
    }
}

impl Request {
    fn encode(&self) -> Vec<u8> {
        let pane = self.pane.map(|pane| pane.to_string()).unwrap_or_default();
        let mut fields: Vec<Vec<u8>> = vec![
            self.verb.name().into(),
            pane.into(),
            self.cwd.as_os_str().as_bytes().to_vec(),
            self.env.len().to_string().into(),
        ];
        fields.extend(
            self.env
                .iter()
                .map(|(key, value)| [key.as_bytes(), b"=", value.as_bytes()].concat()),
        );
        fields.extend(self.command.iter().map(|arg| arg.as_bytes().to_vec()));
        fields.join(&0)
    }

    fn decode(bytes: &[u8]) -> Option<Request> {
        let mut fields = bytes.split(|&byte| byte == 0);
        let verb = Verb::from_name(std::str::from_utf8(fields.next()?).ok()?)?;
        let pane = match fields.next()? {
            b"" => None,
            pane => Some(std::str::from_utf8(pane).ok()?.parse().ok()?),
        };
        let cwd = PathBuf::from(OsStr::from_bytes(fields.next()?));
        let count: usize = std::str::from_utf8(fields.next()?).ok()?.parse().ok()?;
        let env = fields
            .by_ref()
            .take(count)
            .map(|entry| {
                let at = entry.iter().position(|&byte| byte == b'=')?;
                Some((
                    OsStr::from_bytes(&entry[..at]).to_os_string(),
                    OsStr::from_bytes(&entry[at + 1..]).to_os_string(),
                ))
            })
            .collect::<Option<Vec<_>>>()?;
        if env.len() != count {
            return None;
        }
        let command = fields
            .map(|arg| String::from_utf8(arg.to_vec()).ok())
            .collect::<Option<Vec<_>>>()?;
        Some(Request {
            verb,
            command,
            cwd,
            env,
            pane,
        })
    }
}

pub struct Server {
    path: PathBuf,
}

impl Server {
    pub fn start(events: Sender<Event>) -> Option<Server> {
        let dir = env::var_os("XDG_RUNTIME_DIR")?;
        let path = Path::new(&dir).join(format!("xplit-{}.sock", std::process::id()));
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path).ok()?;
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                serve(stream, &events);
            }
        });
        Some(Server { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn serve(mut stream: UnixStream, events: &Sender<Event>) {
    let _ = stream.set_read_timeout(Some(REPLY_TIMEOUT));
    let mut bytes = Vec::new();
    if stream.read_to_end(&mut bytes).is_err() {
        return;
    }
    let reply = match Request::decode(&bytes) {
        Some(request) => {
            let (reply, answer) = mpsc::channel();
            if events.send(Event::Request(request, reply)).is_err() {
                return;
            }
            answer
                .recv_timeout(REPLY_TIMEOUT)
                .unwrap_or_else(|_| Err("no answer from xplit".to_string()))
        }
        None => Err("malformed request".to_string()),
    };
    let text = match reply {
        Ok(()) => OK.to_string(),
        Err(message) => message,
    };
    let _ = stream.write_all(text.as_bytes());
}

pub fn send(socket: &Path, request: &Request) -> io::Result<Result<(), String>> {
    let mut stream = UnixStream::connect(socket)?;
    stream.write_all(&request.encode())?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(if reply == OK { Ok(()) } else { Err(reply) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn parses_verbs_and_commands() {
        let right = Verb::Split(Split::SideBySide);
        let down = Verb::Split(Split::Stacked);
        assert_eq!(parse_args(&[]), Some((right, vec![])));
        assert_eq!(parse_args(&args(&["down"])), Some((down, vec![])));
        assert_eq!(parse_args(&args(&["solo"])), Some((Verb::Solo, vec![])));
        assert_eq!(parse_args(&args(&["hide"])), Some((Verb::Hide, vec![])));
        assert_eq!(
            parse_args(&args(&["claude", "--resume"])),
            Some((right, args(&["claude", "--resume"])))
        );
        assert_eq!(
            parse_args(&args(&["down", "htop"])),
            Some((down, args(&["htop"])))
        );
        assert_eq!(
            parse_args(&args(&["down", "--", "solo"])),
            Some((down, args(&["solo"])))
        );
        assert_eq!(
            parse_args(&args(&["--", "down"])),
            Some((right, args(&["down"])))
        );
    }

    #[test]
    fn rejects_options_and_a_bare_separator() {
        assert_eq!(parse_args(&args(&["--help"])), None);
        assert_eq!(parse_args(&args(&["-x", "vim"])), None);
        assert_eq!(parse_args(&args(&["--"])), None);
        assert_eq!(parse_args(&args(&["hide", "vim"])), None);
    }

    #[test]
    fn request_survives_encoding() {
        let request = Request {
            verb: Verb::Split(Split::Stacked),
            command: args(&["sh", "-c", "echo 'a b'"]),
            cwd: PathBuf::from("/home/someone/a dir"),
            env: vec![
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("EMPTY".into(), "".into()),
                ("EQUALS".into(), "a=b".into()),
                (
                    OsString::from("RAW"),
                    OsStr::from_bytes(b"\xff\xfe").to_os_string(),
                ),
            ],
            pane: Some(3),
        };
        assert_eq!(Request::decode(&request.encode()), Some(request));

        let bare = Request {
            verb: Verb::Solo,
            command: vec![],
            cwd: PathBuf::from("/"),
            env: vec![],
            pane: None,
        };
        assert_eq!(Request::decode(&bare.encode()), Some(bare));
    }

    #[test]
    fn rejects_malformed_requests() {
        let fields = |parts: &[&str]| parts.join("\0").into_bytes();
        assert_eq!(Request::decode(&fields(&["sideways", "", "/", "0"])), None);
        assert_eq!(Request::decode(&fields(&["down", "x", "/", "0"])), None);
        assert_eq!(
            Request::decode(&fields(&["down", "", "/", "2", "A=1"])),
            None
        );
        assert_eq!(
            Request::decode(&fields(&["down", "", "/", "1", "NOEQUALS"])),
            None
        );
        assert_eq!(Request::decode(b"down"), None);
    }
}
