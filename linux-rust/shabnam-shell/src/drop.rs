//! The shell's line to the 隔空传送 receiver (shabnam-dropd, ../../drop). The receiver runs as the
//! unprivileged user "shabnam" and does everything that touches the network; the shell only
//! starts and stops it and exchanges the fixed messages of ipc.rs with it over a unix socket.
//! A worker thread owns the connection, so nothing here can stall the event loop.
//!
//! The receiver is not trusted: its lines are length-capped and parsed by ipc.rs (which strips
//! control characters and drops anything malformed), and nothing it says is ever used as a path
//! or a command.

#[allow(dead_code)]
#[path = "../../drop/src/ipc.rs"]
pub mod ipc;

use ipc::{FromDaemon, ToDaemon};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

/// The user the receiver runs as (see device/shabnam-drop).
const USER: &str = "shabnam";

pub enum Request {
    /// Is a receiver already running (the shell was restarted)? Connect if so.
    Probe,
    /// Start the receiver (if it is not running) and connect.
    Start,
    /// Tell it to quit.
    Stop,
    Send(ToDaemon),
}

pub enum Event {
    /// Receiving ended, or could not be started (`failed`).
    Off { failed: bool },
    Message(FromDaemon),
}

fn socket_path() -> String {
    let name = std::ffi::CString::new(USER).unwrap();
    let entry = unsafe { libc::getpwnam(name.as_ptr()) };
    let uid = if entry.is_null() { 700 } else { unsafe { (*entry).pw_uid } };
    format!("/run/user/{uid}/{}", ipc::SOCKET_NAME)
}

struct Worker<F: Fn(Event) + Send + Clone + 'static> {
    notify: F,
    stream: Option<UnixStream>,
    /// Counts connections, so that the end of an old one is not taken for the end of the new one.
    generation: Arc<AtomicU64>,
    /// The generation of the present connection, and of the last one whose reader has ended.
    current: u64,
    ended: Arc<AtomicU64>,
}

impl<F: Fn(Event) + Send + Clone + 'static> Worker<F> {
    fn connect(&mut self) -> bool {
        if self.stream.is_some() && self.ended.load(Ordering::SeqCst) != self.current {
            return true;
        }
        self.stream = None;
        let Ok(stream) = UnixStream::connect(socket_path()) else { return false };
        let Ok(mut reader) = stream.try_clone() else { return false };
        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
        let mine = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.current = mine;
        let (generation, ended, notify) = (self.generation.clone(), self.ended.clone(), self.notify.clone());
        std::thread::spawn(move || {
            let mut lines = ipc::Lines::default();
            let mut chunk = [0u8; 2048];
            'read: loop {
                let n = match reader.read(&mut chunk) {
                    Ok(n) if n > 0 => n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    _ => break,
                };
                // An overlong line: stop listening to it.
                let Ok(complete) = lines.push(&chunk[..n]) else { break };
                for line in complete {
                    if generation.load(Ordering::SeqCst) != mine {
                        break 'read;
                    }
                    if let Some(message) = ipc::parse_from_daemon(&line) {
                        notify(Event::Message(message));
                    }
                }
            }
            let _ = reader.shutdown(std::net::Shutdown::Both);
            ended.store(mine, Ordering::SeqCst);
            if generation.load(Ordering::SeqCst) == mine {
                notify(Event::Off { failed: false });
            }
        });
        self.stream = Some(stream);
        true
    }

    fn send(&mut self, message: &ToDaemon) {
        let sent = self.stream.as_mut().is_some_and(|s| s.write_all(message.encode().as_bytes()).is_ok());
        if !sent {
            self.stream = None;
        }
    }

    fn handle(&mut self, request: Request) {
        match request {
            Request::Probe => {
                self.connect();
            }
            Request::Start => {
                if self.connect() {
                    return;
                }
                let started = crate::sys::spawn(&["shabnam-drop", "start"]).is_ok();
                // It needs a moment to make its certificate on the very first start.
                let connected = started && (0..40).any(|_| {
                    std::thread::sleep(Duration::from_millis(150));
                    self.connect()
                });
                if !connected {
                    (self.notify)(Event::Off { failed: true });
                }
            }
            Request::Stop => {
                self.connect();
                self.send(&ToDaemon::Quit);
                // From here on the old connection's end is expected, not news.
                self.generation.fetch_add(1, Ordering::SeqCst);
                self.stream = None;
                (self.notify)(Event::Off { failed: false });
            }
            Request::Send(message) => self.send(&message),
        }
    }
}

/// Starts the worker thread; `notify` is called from other threads.
pub fn start(notify: impl Fn(Event) + Send + Clone + 'static) -> mpsc::Sender<Request> {
    let (requests, incoming) = mpsc::channel::<Request>();
    std::thread::spawn(move || {
        let mut worker = Worker { notify, stream: None, generation: Arc::new(AtomicU64::new(0)), current: 0, ended: Arc::new(AtomicU64::new(0)) };
        for request in incoming {
            worker.handle(request);
        }
    });
    requests
}
