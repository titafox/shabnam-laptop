//! The shell's line to the 隔空传送 daemon (shabnam-dropd, ../../drop). The daemon runs as its own
//! unprivileged user "shabnam-drop" and does everything that touches the network; the shell only
//! starts and stops it and exchanges the fixed messages of ipc.rs with it over a unix socket.
//! A worker thread owns the connection, so nothing here can stall the event loop.
//!
//! Also here: the "wake" socket. The daemon can only be started by root, but the program that
//! wants something sent (shabnam-send, started by the file manager) is the desktop's unprivileged
//! user. It connects to this socket — nothing is read from it — and the shell starts the daemon
//! for sending. What is to be sent, the requester then tells the daemon, not the shell.
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

pub enum Request {
    /// Is a receiver already running (the shell was restarted)? Connect if so.
    Probe,
    /// Start the receiver (if it is not running) and connect.
    Start,
    /// Switch receiving off (the daemon leaves by itself once it has nothing to send).
    Stop,
    /// Somebody wants to send: start the daemon for that, unless it is running.
    Wake,
    Send(ToDaemon),
}

pub enum Event {
    /// Receiving ended, or could not be started (`failed`).
    Off { failed: bool },
    Message(FromDaemon),
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
        // The receiver has a user of its own (ipc::DAEMON_USER) and its socket lives in a directory
        // only that user and root can write. Still: nothing is believed unless the kernel says the
        // process behind the socket is that user — never the browser's user, never a stand-in.
        let user = std::ffi::CString::new(ipc::DAEMON_USER).unwrap();
        let entry = unsafe { libc::getpwnam(user.as_ptr()) };
        if entry.is_null() {
            return false;
        }
        let daemon_uid = unsafe { (*entry).pw_uid };
        let Ok(stream) = UnixStream::connect(ipc::socket_path()) else { return false };
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let got = unsafe {
            libc::getsockopt(std::os::fd::AsRawFd::as_raw_fd(&stream), libc::SOL_SOCKET, libc::SO_PEERCRED, &mut cred as *mut libc::ucred as *mut libc::c_void, &mut len)
        };
        if got != 0 || len as usize != std::mem::size_of::<libc::ucred>() || daemon_uid == 0 || cred.uid != daemon_uid {
            return false;
        }
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
                    // It is running (perhaps only to send): receiving is switched on.
                    return self.send(&ToDaemon::Receive(true));
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
                if self.connect() {
                    // Its answer (a state with receiving off) is what turns the switch off.
                    self.send(&ToDaemon::Receive(false));
                }
                if self.stream.is_none() {
                    (self.notify)(Event::Off { failed: false });
                }
            }
            Request::Wake => {
                if self.connect() {
                    return;
                }
                if crate::sys::spawn(&["shabnam-drop", "start", "send"]).is_ok() {
                    let _ = (0..40).any(|_| {
                        std::thread::sleep(Duration::from_millis(150));
                        self.connect()
                    });
                }
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

fn ids_of(user: &str) -> Option<(u32, u32)> {
    let user = std::ffi::CString::new(user).ok()?;
    let entry = unsafe { libc::getpwnam(user.as_ptr()) };
    (!entry.is_null()).then(|| unsafe { ((*entry).pw_uid, (*entry).pw_gid) })
}

/// Makes the wake socket: /run/shabnam-send (root's, group of the desktop user, 0750) with
/// wake.sock (0660, same group) in it. None when this is not root's shell or that user is missing.
pub fn wake_listener() -> Option<std::os::unix::net::UnixListener> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    if unsafe { libc::getuid() } != 0 {
        return None;
    }
    let (_, group) = ids_of(ipc::DESKTOP_USER)?;
    let own = |path: &str, mode: u32| {
        let c_path = std::ffi::CString::new(path).ok()?;
        (unsafe { libc::lchown(c_path.as_ptr(), 0, group) } == 0).then_some(())?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).ok()
    };
    let dir = ipc::WAKE_DIR;
    match std::fs::symlink_metadata(dir) {
        Ok(meta) if meta.is_dir() && meta.uid() == 0 => {}
        Ok(_) => return None,
        Err(_) => std::fs::DirBuilder::new().mode(0o700).create(dir).ok()?,
    }
    own(dir, 0o750)?;
    let path = ipc::wake_path();
    let _ = std::fs::remove_file(&path);
    let old_mask = unsafe { libc::umask(0o177) };
    let bound = std::os::unix::net::UnixListener::bind(&path);
    unsafe { libc::umask(old_mask) };
    let listener = bound.ok()?;
    own(&path, 0o660)?;
    listener.set_nonblocking(true).ok()?;
    Some(listener)
}

/// Whether a connection to the wake socket counts: only the desktop user's (and root's).
pub fn wake_allowed(stream: &UnixStream) -> bool {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let got = unsafe {
        libc::getsockopt(std::os::fd::AsRawFd::as_raw_fd(stream), libc::SOL_SOCKET, libc::SO_PEERCRED, &mut cred as *mut libc::ucred as *mut libc::c_void, &mut len)
    };
    got == 0 && len as usize == std::mem::size_of::<libc::ucred>() && (cred.uid == 0 || ids_of(ipc::DESKTOP_USER).is_some_and(|(uid, _)| uid == cred.uid))
}
