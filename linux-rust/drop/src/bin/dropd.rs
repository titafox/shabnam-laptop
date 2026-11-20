//! shabnam-dropd — the network side of 隔空传送: it receives, and it sends. Runs as its own
//! unprivileged user "shabnam-drop" (the wrapper `shabnam-drop start` drops to it), only while the
//! owner has switched receiving on or something is being sent. Nothing else runs as that user:
//! the browser and the player (user "shabnam") cannot touch this daemon's control socket,
//! certificate or process.
//!
//!   shabnam-dropd                 run (one per user); stops on `quit` from the shell or SIGTERM
//!   shabnam-dropd status          prints "on" (receiving), "send" (running only to send) or "off"
//!   shabnam-dropd ctl JSON [SEC]  testing: send one control line to the running daemon and print
//!                                 what it says for SEC seconds (default 1)
//!
//! Sending: a program of the desktop user (shabnam-send, started by the file manager) connects to
//! the submit socket in /run/shabnam-drop-submit and hands over open files (fdpass.rs). That user
//! also runs the browser, so nothing it says decides where the files go: the daemon lists the
//! devices it found to the shell, the shell (root) shows the picker, and only the shell's
//! connection can name the target — one of the listed devices (outgoing.rs). The files are read
//! from the descriptors that were handed over, which the requester opened with its own rights.
//! With receiving switched off the daemon is started for the request alone (SHABNAM_DROP_RECEIVE=0):
//! it opens the port only while a request is there (other devices answer a search on it),
//! declines every offer without asking, and exits when it has been idle for a while.
//!
//! What it does: listens on the Wi-Fi interface's address only (HTTPS, port 53317, and the
//! discovery group on that interface), speaking the LocalSend protocol through that project's own
//! core library. Every offer is checked (sender on the same network, size limits, free space) and
//! then shown by the desktop shell, which this daemon talks to over a unix socket in its 0700
//! runtime directory /run/shabnam-drop (messages: ../ipc.rs). Without an answer from the shell
//! nothing is accepted, unless the owner switched on "quick receive". Accepted files are written
//! to the downloads directory the wrapper hands over as an open descriptor (/home/shabnam/Downloads)
//! under a rewritten name (../names.rs), never over an existing file, mode 0644, and are never
//! opened or run by this program. Inside an accepted transfer each file may arrive once, a few at
//! a time, and all of them together never more than the accepted total (../policy.rs).

use bytes::Bytes;
use localsend::discovery::{DeviceChannel, DiscoveredDevice, DiscoveryEvent, HttpChannel};
use localsend::http::dto_v2::RegisterDtoV2;
use localsend::http::server::common::save::FileUploadTarget;
use localsend::http::server::v2::{PrepareUploadDecisionV2, ServerEventV2};
use localsend::http::server::PeerIp;
use localsend::model::transfer::FileDto;
use localsend::http::client::{ClientError, LsHttpClient, LsHttpClientVersion};
use localsend::http::dto::{PrepareUploadRequestDto, RegisterDto};
use localsend::model::discovery::{DeviceType, ProtocolType, PROTOCOL_VERSION_V2};
use localsend::model::transfer::FileContent;
use shabnam_drop::ipc::{self, FromDaemon, FromSubmit, SendOutcome, SubmitRefusal, ToDaemon, ToSubmit};
use shabnam_drop::net::{self, Identity, Net};
use shabnam_drop::outgoing::{self, Entry, Left, Request, Stage};
use shabnam_drop::{fdpass, names, policy};
use std::collections::HashMap;
use std::io::Write as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

const MAX_CLIENTS: usize = 4;
/// A running transfer in which nothing moved for this long is ended.
const IDLE: Duration = Duration::from_secs(120);

enum ClientEvent {
    Connected { root: bool, tx: mpsc::UnboundedSender<String> },
    Line(ToDaemon),
    Gone,
}

/// From a requester's connection on the submit socket (a thread per connection reads it).
enum SubmitEvent {
    /// It handed over these files (name, size, the open file) and is done.
    Go { root: bool, files: Vec<(String, u64, std::fs::File)>, to: Option<String>, reply: Arc<UnixStream> },
    Confirm,
    Gone,
}

/// From the task that talks to the target device.
enum SendEvent {
    PinWanted { wrong: bool },
    Accepted { count: u32, total: u64 },
    Ended { outcome: SendOutcome, sent: u32 },
}

enum Event {
    Client(u64, ClientEvent),
    FileDone { session: String, file_id: String, result: Result<String, String> },
    DevicesDue(u64),
    Submit(u64, SubmitEvent),
    SearchDone(String),
    Send(String, SendEvent),
    Stop,
}

/// The one send request there can be at a time.
struct Out {
    request: Request<Option<std::fs::File>>,
    /// The requesters' connections, to tell them how it went.
    replies: HashMap<u64, Arc<UnixStream>>,
    searching: bool,
    /// The `searching` flag last reported with the device list (None: nothing reported yet).
    listed: Option<bool>,
    /// When waiting (for the owner, for the other side) is given up.
    deadline: Instant,
    cancel: CancellationToken,
    pin_tx: Option<mpsc::Sender<String>>,
    done: Arc<AtomicU64>,
    file_no: Arc<AtomicU32>,
    count: u32,
    total: u64,
    reported: Option<(u32, u64)>,
    moved: Instant,
    /// Set when this side gave up waiting: what the task's "cancelled" then really means.
    gave_up: Option<SendOutcome>,
}

struct Pending {
    id: String,
    from: String,
    files: HashMap<String, FileDto>,
    decision: oneshot::Sender<PrepareUploadDecisionV2>,
    deadline: Instant,
}

struct Active {
    id: String,
    from: String,
    total: u64,
    done: Arc<AtomicU64>,
    reported: u64,
    moved: Instant,
    /// file id -> the name it was saved under.
    saved: HashMap<String, String>,
    /// Which accepted file may still arrive (each once, a few at a time).
    uploads: policy::Uploads,
    failed: u32,
    writers: usize,
    ended: bool,
    cancel: CancellationToken,
}

struct Daemon {
    identity: Identity,
    interface: String,
    dir: PathBuf,
    net: Option<Net>,
    quick: bool,
    /// Receiving is switched on. Off: the daemon is only here to send.
    receiving: bool,
    clients: HashMap<u64, (bool, mpsc::UnboundedSender<String>)>,
    pending: Option<Pending>,
    active: Option<Active>,
    outgoing: Option<Out>,
    requests: u64,
    /// Since when there has been nothing to send (for leaving when receiving is off).
    idle_since: Instant,
    /// Connections on the submit socket that are still alive.
    submitters: Arc<AtomicUsize>,
    parts: u32,
    events: mpsc::UnboundedSender<Event>,
    server_tx: mpsc::Sender<ServerEventV2>,
    discovery_tx: mpsc::Sender<DiscoveryEvent>,
}

fn runtime_dir() -> Result<PathBuf, String> {
    use std::os::unix::fs::MetadataExt;
    let uid = unsafe { libc::getuid() };
    let dir = ipc::RUNTIME_DIR;
    let meta = std::fs::symlink_metadata(&dir).map_err(|e| format!("runtime directory {dir}: {e}"))?;
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(format!("runtime directory {dir} must be a directory of this user with mode 0700"));
    }
    Ok(PathBuf::from(dir))
}

fn socket_path() -> Result<PathBuf, String> {
    Ok(runtime_dir()?.join(ipc::SOCKET_NAME))
}

/// The user whose Downloads directory receives the files (the desktop's unprivileged user).
const DOWNLOADS_OWNER: &str = "shabnam";

/// Where files go. Normally the wrapper (root) opened /home/shabnam/Downloads and passes the open
/// directory as descriptor SHABNAM_DROP_DIR_FD: this user cannot enter /home/shabnam and does not
/// need to. That home belongs to the browser's user, which could have put something else where
/// Downloads should be — so the descriptor is only used if it is a directory owned by that user
/// with this daemon's group (as the wrapper set it up), which rules out every directory that
/// matters to anyone else, this daemon's own state directory included.
/// Without the variable (development): ~/Downloads of whoever runs this.
fn downloads_dir() -> Result<PathBuf, String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    if let Some(fd) = std::env::var("SHABNAM_DROP_DIR_FD").ok().and_then(|v| v.parse::<u32>().ok()).filter(|fd| (3..1024).contains(fd)) {
        let dir = PathBuf::from(format!("/proc/self/fd/{fd}"));
        let meta = std::fs::metadata(&dir).map_err(|e| format!("downloads directory (descriptor {fd}): {e}"))?;
        let owner = net::user(DOWNLOADS_OWNER).map(|(uid, _, _)| uid);
        if !meta.is_dir() || Some(meta.uid()) != owner || meta.gid() != unsafe { libc::getgid() } || meta.mode() & 0o020 == 0 {
            return Err("the downloads directory is not what the wrapper should have prepared".into());
        }
        return Ok(dir);
    }
    let home = std::env::var("HOME").ok().filter(|h| h.starts_with('/')).ok_or("HOME is not set")?;
    let dir = Path::new(&home).join("Downloads");
    match std::fs::symlink_metadata(&dir) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err(format!("{} is not a directory", dir.display())),
        Err(_) => std::fs::DirBuilder::new().mode(0o755).create(&dir).map_err(|e| format!("{}: {e}", dir.display()))?,
    }
    Ok(dir)
}

/// The socket on which the desktop user hands over files. Its directory was made by the wrapper
/// for exactly this: ours, the desktop user's group, mode 0750 — so that user (and root) can
/// reach the socket and nobody can replace it. Who connects is checked again on every connection.
fn submit_listener() -> Result<std::os::unix::net::UnixListener, String> {
    use std::os::unix::fs::MetadataExt;
    let dir = ipc::SUBMIT_DIR;
    let meta = std::fs::symlink_metadata(dir).map_err(|e| format!("{dir}: {e}"))?;
    let group = net::user(ipc::DESKTOP_USER).map(|(_, gid, _)| gid);
    if !meta.is_dir() || meta.uid() != unsafe { libc::getuid() } || Some(meta.gid()) != group || meta.mode() & 0o027 != 0 {
        return Err(format!("{dir} is not what the wrapper should have prepared"));
    }
    let path = ipc::submit_path();
    let _ = std::fs::remove_file(&path);
    let old = unsafe { libc::umask(0o111) };
    let listener = std::os::unix::net::UnixListener::bind(&path);
    unsafe { libc::umask(old) };
    listener.map_err(|e| format!("{path}: {e}"))
}

/// Accepts requesters: the desktop user and root, nobody else, and not too many at once.
fn serve_submit(listener: std::os::unix::net::UnixListener, events: mpsc::UnboundedSender<Event>, alive: Arc<AtomicUsize>) {
    let desktop = net::user(ipc::DESKTOP_USER).map(|(uid, _, _)| uid).filter(|uid| *uid != 0);
    let mut next = 0u64;
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            std::thread::sleep(Duration::from_millis(200));
            continue;
        };
        let uid = net::peer_uid(&stream);
        if uid.is_none() || !(uid == Some(0) || uid == desktop) {
            continue;
        }
        if alive.fetch_add(1, Ordering::SeqCst) >= policy::MAX_SUBMITTERS {
            alive.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        next += 1;
        let (conn, events, alive) = (next, events.clone(), alive.clone());
        std::thread::spawn(move || {
            submit_conn(stream, conn, uid == Some(0), &events);
            alive.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

/// One requester: takes its files one message at a time, then waits until it goes away.
/// Every descriptor must be a regular file open for reading; anything else ends the connection.
fn submit_conn(stream: UnixStream, conn: u64, root: bool, events: &mpsc::UnboundedSender<Event>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
    let Ok(reply) = stream.try_clone().map(Arc::new) else { return };
    let say = |message: FromSubmit| (&stream).write_all(message.encode().as_bytes()).is_ok();
    let mut files = Vec::new();
    let mut bytes = 0u64;
    let to = loop {
        let Ok(Some((line, mut fds))) = fdpass::recv_line(&stream, ipc::MAX_LINE) else { return };
        match ipc::parse_to_submit(&line) {
            Some(ToSubmit::File { name }) => {
                let Some(size) = (fds.len() == 1).then(|| fdpass::readable_regular_file(&fds[0])).flatten() else {
                    say(FromSubmit::Refused(SubmitRefusal::NotAFile));
                    return;
                };
                bytes = bytes.saturating_add(size);
                if let Err(why) = policy::check_send(files.len() + 1, bytes) {
                    say(FromSubmit::Refused(why));
                    return;
                }
                files.push((names::sanitize(&name), size, std::fs::File::from(fds.pop().unwrap())));
                if !say(FromSubmit::Ok) {
                    return;
                }
            }
            Some(ToSubmit::Go { to }) if fds.is_empty() && !files.is_empty() => break to,
            _ => {
                say(FromSubmit::Refused(SubmitRefusal::Bad));
                return;
            }
        }
    };
    if events.send(Event::Submit(conn, SubmitEvent::Go { root, files, to, reply })).is_err() {
        return;
    }
    let _ = stream.set_read_timeout(None);
    while let Ok(Some((line, fds))) = fdpass::recv_line(&stream, ipc::MAX_LINE) {
        if !fds.is_empty() || ipc::parse_to_submit(&line) != Some(ToSubmit::Confirm) || events.send(Event::Submit(conn, SubmitEvent::Confirm)).is_err() {
            break;
        }
    }
    let _ = events.send(Event::Submit(conn, SubmitEvent::Gone));
}

fn tell_requester(stream: &UnixStream, message: FromSubmit) {
    let _ = (&*stream).write_all(message.encode().as_bytes());
}

/// `shabnam-dropd status`: "on" (exit 0) while receiving, "send" (exit 1) while it only runs to
/// send, "off" (exit 1) when it is not running.
fn status() -> Result<(), String> {
    use std::io::Read;
    let state = net::connect_daemon().and_then(|mut stream| {
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
        let mut lines = ipc::Lines::default();
        let mut chunk = [0u8; 2048];
        loop {
            let n = stream.read(&mut chunk).ok().filter(|n| *n > 0)?;
            for line in lines.push(&chunk[..n]).ok()? {
                if let Some(FromDaemon::State { receiving, .. }) = ipc::parse_from_daemon(&line) {
                    return Some(receiving);
                }
            }
        }
    });
    println!("{}", match state {
        Some(true) => "on",
        Some(false) => "send",
        None => "off",
    });
    if state != Some(true) {
        std::process::exit(1);
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None => run(),
        Some("status") if args.len() == 1 => status(),
        Some("ctl") if args.len() >= 2 => ctl(&args[1], args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1)),
        _ => Err("usage: shabnam-dropd [status | ctl JSON [SECONDS]]".into()),
    };
    if let Err(e) = result {
        eprintln!("shabnam-dropd: {e}");
        std::process::exit(1);
    }
}

/// Testing aid: one line to the daemon, then print its lines for a while.
fn ctl(line: &str, seconds: u64) -> Result<(), String> {
    use std::io::{Read, Write};
    let mut stream = net::connect_daemon().ok_or_else(|| format!("{}: the receiver is not running", ipc::socket_path()))?;
    if line != "-" {
        ipc::parse_to_daemon(line).ok_or("not a control message")?;
        stream.write_all(format!("{}\n", line.trim()).as_bytes()).map_err(|e| e.to_string())?;
    }
    let until = Instant::now() + Duration::from_secs(seconds.min(120));
    let mut chunk = [0u8; 4096];
    while Instant::now() < until {
        let _ = stream.set_read_timeout(Some(until.saturating_duration_since(Instant::now()).max(Duration::from_millis(10))));
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => print!("{}", String::from_utf8_lossy(&chunk[..n])),
            Err(_) => break,
        }
    }
    Ok(())
}

fn run() -> Result<(), String> {
    if unsafe { libc::getuid() } == 0 {
        return Err("refusing to run as root; start it with `shabnam-drop start`".into());
    }
    if net::user(ipc::DAEMON_USER).is_some_and(|(uid, _, _)| uid != unsafe { libc::getuid() }) {
        return Err(format!("this is to be run as the user {}; start it with `shabnam-drop start`", ipc::DAEMON_USER));
    }
    unsafe { libc::umask(0o022) };
    if std::env::var_os("SHABNAM_DROP_LOG").is_some() {
        let _ = tracing_subscriber::fmt().with_writer(std::io::stderr).try_init();
    }
    let interface = net::interface_name()?;
    let dir = downloads_dir()?;
    names::remove_stale_parts(&dir);
    let identity = Identity::load_or_generate(&net::config_dir()?, policy::alias(policy::lang()))?;
    let path = socket_path()?;
    if std::os::unix::net::UnixStream::connect(&path).is_ok() {
        return Err("already running".into());
    }
    let _ = std::fs::remove_file(&path);
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let old = unsafe { libc::umask(0o177) };
        let listener = tokio::net::UnixListener::bind(&path);
        unsafe { libc::umask(old) };
        let listener = listener.map_err(|e| format!("{}: {e}", path.display()))?;
        let (events, events_rx) = mpsc::unbounded_channel();
        let (server_tx, server_rx) = mpsc::channel(16);
        let (discovery_tx, discovery_rx) = mpsc::channel(16);
        tokio::spawn(accept_clients(listener, events.clone()));
        tokio::spawn(stop_on_signal(events.clone()));
        let submitters = Arc::new(AtomicUsize::new(0));
        match submit_listener() {
            Ok(listener) => {
                let (events, alive) = (events.clone(), submitters.clone());
                std::thread::spawn(move || serve_submit(listener, events, alive));
            }
            Err(e) => eprintln!("shabnam-dropd: sending from the desktop is not available: {e}"),
        }
        let daemon = Daemon {
            identity,
            interface,
            dir,
            net: None,
            quick: false,
            receiving: std::env::var("SHABNAM_DROP_RECEIVE").map_or(true, |v| v != "0"),
            clients: HashMap::new(),
            pending: None,
            active: None,
            outgoing: None,
            requests: 0,
            idle_since: Instant::now(),
            submitters,
            parts: 0,
            events,
            server_tx,
            discovery_tx,
        };
        daemon.serve(events_rx, server_rx, discovery_rx).await;
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(ipc::submit_path());
        Ok(())
    })
}

async fn stop_on_signal(events: mpsc::UnboundedSender<Event>) {
    use tokio::signal::unix::{signal, SignalKind};
    let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else { return };
    tokio::select! { _ = term.recv() => {}, _ = int.recv() => {} }
    let _ = events.send(Event::Stop);
}

/// Control connections: root (the shell) may do everything, this user may only ask for the
/// device list, anybody else is turned away (and cannot even reach the socket).
async fn accept_clients(listener: tokio::net::UnixListener, events: mpsc::UnboundedSender<Event>) {
    let own = unsafe { libc::getuid() };
    let mut next = 0u64;
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(200)).await;
            continue;
        };
        let Ok(uid) = stream.peer_cred().map(|c| c.uid()) else { continue };
        if uid != 0 && uid != own {
            continue;
        }
        next += 1;
        let id = next;
        let (mut reader, mut writer) = stream.into_split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        if events.send(Event::Client(id, ClientEvent::Connected { root: uid == 0, tx })).is_err() {
            return;
        }
        tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                if writer.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        let events = events.clone();
        tokio::spawn(async move {
            let mut lines = ipc::Lines::default();
            let mut chunk = [0u8; 2048];
            'read: loop {
                let n = match reader.read(&mut chunk).await {
                    Ok(n) if n > 0 => n,
                    _ => break,
                };
                let Ok(complete) = lines.push(&chunk[..n]) else { break };
                for line in complete {
                    if let Some(message) = ipc::parse_to_daemon(&line) {
                        if events.send(Event::Client(id, ClientEvent::Line(message))).is_err() {
                            break 'read;
                        }
                    }
                }
            }
            let _ = events.send(Event::Client(id, ClientEvent::Gone));
        });
    }
}

/// Writes one arriving file. Stops with an error when more than the announced size arrives, when
/// the stream ends early, or when the transfer is cancelled. Every chunk is first counted against
/// the whole transfer's accepted `total` (`done` is the transfer's one counter): when that is
/// exceeded the transfer is cancelled, which stops its other files too.
async fn write_part(mut chunks: mpsc::Receiver<Bytes>, file: std::fs::File, size: u64, total: u64, done: Arc<AtomicU64>, cancel: CancellationToken) -> Result<(), String> {
    let mut file = tokio::fs::File::from_std(file);
    let mut written = 0u64;
    loop {
        let chunk = tokio::select! {
            chunk = chunks.recv() => chunk,
            _ = cancel.cancelled() => return Err("cancelled".into()),
        };
        let Some(chunk) = chunk else { break };
        written += chunk.len() as u64;
        if written > size {
            return Err(format!("more than the announced {size} bytes"));
        }
        if !policy::charge(&done, chunk.len() as u64, total) {
            cancel.cancel();
            return Err(format!("more than the accepted {total} bytes in this transfer"));
        }
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
    }
    file.flush().await.map_err(|e| e.to_string())?;
    if written != size {
        return Err(format!("{written} of {size} bytes arrived"));
    }
    Ok(())
}

fn device_of(ip: PeerIp, info: RegisterDtoV2) -> DiscoveredDevice {
    DiscoveredDevice {
        alias: info.alias,
        version: info.version,
        device_model: info.device_model,
        device_type: info.device_type,
        fingerprint: info.fingerprint,
        channel: DeviceChannel::Http(HttpChannel { host: ip.to_string(), port: info.port, protocol: info.protocol }),
        download: info.download,
    }
}

impl Daemon {
    async fn serve(mut self, mut events: mpsc::UnboundedReceiver<Event>, mut server: mpsc::Receiver<ServerEventV2>, mut discovery: mpsc::Receiver<DiscoveryEvent>) {
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut ticks = 0u64;
        loop {
            tokio::select! {
                Some(event) = events.recv() => {
                    if !self.on_event(event).await {
                        break;
                    }
                }
                Some(event) = server.recv() => self.on_server(event).await,
                // The discovery store keeps what matters; its events are only drained.
                Some(_) = discovery.recv() => {}
                _ = tick.tick() => {
                    if ticks % 8 == 0 {
                        self.check_address().await;
                    }
                    ticks += 1;
                    self.on_tick().await;
                    // Only here to send, and nothing to send for a while: leave.
                    let idle = !self.receiving && self.outgoing.is_none() && self.submitters.load(Ordering::SeqCst) == 0;
                    if idle && self.idle_since.elapsed() > Duration::from_secs(policy::SEND_IDLE_SECONDS) {
                        break;
                    }
                }
            }
        }
        self.drop_pending();
        self.abort_active().await;
        self.finish_send(SendOutcome::Cancelled, 0).await;
        if let Some(net) = self.net.take() {
            net.stop().await;
        }
    }

    fn state(&self) -> FromDaemon {
        FromDaemon::State { alias: self.identity.alias.clone(), address: self.net.as_ref().map(|n| n.address.to_string()), quick: self.quick, receiving: self.receiving }
    }

    /// To every connected shell.
    fn tell(&self, message: FromDaemon) {
        let line = message.encode();
        for (root, tx) in self.clients.values() {
            if *root {
                let _ = tx.send(line.clone());
            }
        }
    }

    fn shell_connected(&self) -> bool {
        self.clients.values().any(|(root, _)| *root)
    }

    /// Follows the interface: (re)starts listening when its address appears or changes, stops
    /// when it is gone — and when nothing needs the network (receiving off, nothing to send).
    async fn check_address(&mut self) {
        let wanted = policy::network_wanted(self.receiving, self.outgoing.is_some());
        let now = if wanted { net::interface_address(&self.interface) } else { None };
        if self.net.as_ref().map(|n| (n.address, n.netmask)) == now {
            return;
        }
        self.drop_pending();
        self.abort_active().await;
        if let Some(net) = self.net.take() {
            net.stop().await;
        }
        if let Some((address, netmask)) = now {
            match Net::start(&self.identity, address, netmask, self.server_tx.clone(), self.discovery_tx.clone()).await {
                Ok(net) => {
                    let discovery = net.discovery.clone();
                    tokio::spawn(async move { discovery.announce().await });
                    self.net = Some(net);
                }
                Err(e) => eprintln!("shabnam-dropd: {e}"),
            }
        }
        self.tell(self.state());
        self.start_search();
    }

    async fn on_event(&mut self, event: Event) -> bool {
        match event {
            Event::Stop => return false,
            Event::Client(id, ClientEvent::Connected { root, tx }) => {
                if self.clients.len() >= MAX_CLIENTS {
                    return true; // dropping tx closes the connection's writer
                }
                let _ = tx.send(self.state().encode());
                if root {
                    // A shell that connects late still learns about what is going on.
                    if let Some(p) = &self.pending {
                        let _ = tx.send(self.offer_of(p).encode());
                    }
                    if let Some(out) = self.outgoing.as_mut().filter(|o| !o.request.is_auto()) {
                        match out.request.stage {
                            Stage::Picking => {
                                out.listed = None; // the list follows on the next tick
                                let _ = tx.send(Self::send_request_of(out).encode());
                            }
                            Stage::PinWanted => {
                                let _ = tx.send(FromDaemon::SendPinWanted { id: out.request.id.clone(), wrong: false }.encode());
                            }
                            _ => {}
                        }
                    }
                }
                self.clients.insert(id, (root, tx));
            }
            Event::Client(id, ClientEvent::Gone) => {
                self.clients.remove(&id);
                if !self.shell_connected() {
                    self.drop_pending(); // nobody is left to answer
                    // ... nor to choose a device or type a PIN.
                    if self.outgoing.as_ref().is_some_and(|o| !o.request.is_auto() && matches!(o.request.stage, Stage::Picking | Stage::PinWanted)) {
                        self.finish_send(SendOutcome::Cancelled, 0).await;
                    }
                }
            }
            Event::Client(id, ClientEvent::Line(message)) => {
                let Some((root, tx)) = self.clients.get(&id).map(|(root, tx)| (*root, tx.clone())) else { return true };
                match (message, root) {
                    (ToDaemon::Devices, _) => {
                        if let Some(net) = &self.net {
                            let discovery = net.discovery.clone();
                            tokio::spawn(async move { discovery.announce().await });
                            let events = self.events.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_millis(1500)).await;
                                let _ = events.send(Event::DevicesDue(id));
                            });
                        } else {
                            let _ = tx.send(FromDaemon::Devices(Vec::new()).encode());
                        }
                    }
                    (_, false) => {}
                    (ToDaemon::Quit, true) => return false,
                    (ToDaemon::Quick(on), true) => {
                        self.quick = on;
                        self.tell(self.state());
                    }
                    (ToDaemon::Answer { id, accept }, true) => self.answer(&id, accept),
                    (ToDaemon::Cancel { id }, true) => {
                        if self.active.as_ref().is_some_and(|a| a.id == id) {
                            self.abort_active().await;
                        }
                    }
                    (ToDaemon::Receive(on), true) => {
                        self.receiving = on;
                        if !on {
                            // Off is off: nothing waits for an answer, nothing keeps arriving.
                            self.quick = false;
                            self.drop_pending();
                            self.abort_active().await;
                        }
                        self.idle_since = Instant::now();
                        self.check_address().await;
                        self.tell(self.state());
                    }
                    // Where the files go is said here and nowhere else: by root's connection,
                    // naming one of the devices that were listed for this request.
                    (ToDaemon::SendTo { id, fingerprint, host, port }, true) => {
                        let Some(out) = self.outgoing.as_mut().filter(|o| o.request.id == id && o.request.stage == Stage::Picking) else { return true };
                        match out.request.choose(&fingerprint, &host, port) {
                            Some(device) => self.begin_send(device).await,
                            // Not (or no longer) one of the listed devices: the request ends
                            // rather than wait for a picker that has closed.
                            None => self.finish_send(SendOutcome::Unreachable, 0).await,
                        }
                    }
                    (ToDaemon::SendPin { id, pin }, true) => {
                        if let Some(out) = self.outgoing.as_mut().filter(|o| o.request.id == id) {
                            if out.request.pin_given() {
                                out.deadline = Instant::now() + Duration::from_secs(policy::SEND_WAIT_SECONDS);
                                if let Some(pin_tx) = &out.pin_tx {
                                    let _ = pin_tx.try_send(pin);
                                }
                            }
                        }
                    }
                    (ToDaemon::SendRefresh { id }, true) => {
                        if self.outgoing.as_ref().is_some_and(|o| o.request.id == id && o.request.stage == Stage::Picking) {
                            self.start_search();
                        }
                    }
                    (ToDaemon::SendCancel { id }, true) => {
                        if self.outgoing.as_ref().is_some_and(|o| o.request.id == id) {
                            self.finish_send(SendOutcome::Cancelled, 0).await;
                        }
                    }
                }
            }
            Event::Submit(conn, event) => self.on_submit(conn, event).await,
            Event::SearchDone(id) => {
                if let Some(out) = self.outgoing.as_mut().filter(|o| o.request.id == id) {
                    out.searching = false;
                }
            }
            Event::Send(id, event) => {
                let Some(out) = self.outgoing.as_mut().filter(|o| o.request.id == id) else { return true };
                match event {
                    SendEvent::PinWanted { wrong } => {
                        if out.request.pin_wanted() {
                            out.deadline = Instant::now() + Duration::from_secs(policy::SEND_WAIT_SECONDS);
                            let message = FromDaemon::SendPinWanted { id, wrong };
                            self.tell(message);
                        } else {
                            self.finish_send(SendOutcome::PinNeeded, 0).await;
                        }
                    }
                    SendEvent::Accepted { count, total } => {
                        out.request.accepted();
                        (out.count, out.total, out.moved) = (count, total, Instant::now());
                    }
                    SendEvent::Ended { outcome, sent } => {
                        let outcome = match (outcome, out.gave_up) {
                            (SendOutcome::Cancelled, Some(real)) => real,
                            _ => outcome,
                        };
                        self.finish_send(outcome, sent).await;
                    }
                }
            }
            Event::DevicesDue(id) => {
                if let Some((_, tx)) = self.clients.get(&id) {
                    let _ = tx.send(FromDaemon::Devices(self.device_list()).encode());
                }
            }
            Event::FileDone { session, file_id, result } => {
                if let Some(active) = self.active.as_mut().filter(|a| a.id == session) {
                    active.writers = active.writers.saturating_sub(1);
                    active.moved = Instant::now();
                    active.uploads.end(&file_id, result.is_ok());
                    // A writer found the transfer over its accepted total: all of it stops.
                    let over = active.cancel.is_cancelled() && !active.ended;
                    match result {
                        Ok(name) => {
                            active.saved.insert(file_id, name);
                        }
                        Err(e) => {
                            active.failed += 1;
                            eprintln!("shabnam-dropd: a file was not saved: {e}");
                        }
                    }
                    if over {
                        self.abort_active().await;
                    }
                    self.finish_if_over();
                } else if let Ok(name) = result {
                    // Its transfer was cancelled meanwhile: the file is not kept.
                    let _ = std::fs::remove_file(self.dir.join(name));
                }
            }
        }
        true
    }

    fn offer_of(&self, pending: &Pending) -> FromDaemon {
        let mut names: Vec<&str> = pending.files.values().map(|f| f.file_name.as_str()).collect();
        names.sort();
        FromDaemon::Offer {
            id: pending.id.clone(),
            from: pending.from.clone(),
            count: pending.files.len() as u32,
            bytes: pending.files.values().map(|f| f.size).fold(0u64, u64::saturating_add),
            name: names.first().map(|n| names::sanitize(n)).unwrap_or_default(),
        }
    }

    fn answer(&mut self, id: &str, accept: bool) {
        if self.pending.as_ref().is_some_and(|p| p.id == id) {
            let pending = self.pending.take().unwrap();
            if accept {
                self.accept(pending);
            } else {
                let _ = pending.decision.send(PrepareUploadDecisionV2::Decline);
            }
        }
    }

    fn accept(&mut self, pending: Pending) {
        let total = pending.files.values().map(|f| f.size).fold(0u64, u64::saturating_add);
        let ids = pending.files.keys().cloned().collect();
        let uploads = policy::Uploads::new(pending.files.iter().map(|(id, file)| (id.clone(), file.size)));
        if pending.decision.send(PrepareUploadDecisionV2::Accept(ids)).is_err() {
            return; // the sender is gone
        }
        self.active = Some(Active {
            id: pending.id,
            from: pending.from,
            total,
            done: Arc::new(AtomicU64::new(0)),
            reported: 0,
            moved: Instant::now(),
            saved: HashMap::new(),
            uploads,
            failed: 0,
            writers: 0,
            ended: false,
            cancel: CancellationToken::new(),
        });
    }

    /// Declines the offer that is waiting, if any.
    fn drop_pending(&mut self) {
        if let Some(pending) = self.pending.take() {
            let _ = pending.decision.send(PrepareUploadDecisionV2::Decline);
            self.tell(FromDaemon::OfferGone { id: pending.id });
        }
    }

    /// Stops the running transfer: files that are complete stay, the one being written is removed.
    async fn abort_active(&mut self) {
        let Some(active) = self.active.as_mut() else { return };
        active.cancel.cancel();
        active.ended = true;
        let id = active.id.clone();
        if let Some(net) = &self.net {
            net.server.cancel_v2_session(&id).await;
        }
        // The writers report in a moment; do not wait for them.
        if let Some(active) = self.active.as_mut() {
            active.failed += active.writers as u32;
            active.writers = 0;
        }
        self.finish_if_over();
    }

    fn finish_if_over(&mut self) {
        if !self.active.as_ref().is_some_and(|a| a.ended && a.writers == 0) {
            return;
        }
        let active = self.active.take().unwrap();
        let mut names: Vec<&String> = active.saved.values().collect();
        names.sort();
        self.tell(FromDaemon::Done {
            id: active.id,
            from: active.from,
            saved: active.saved.len() as u32,
            failed: active.failed,
            name: names.first().map(|n| n.to_string()).unwrap_or_default(),
        });
    }

    /// The devices a send can go to: those that answered, on the Wi-Fi network, not this one.
    fn device_list(&self) -> Vec<ipc::Device> {
        let Some(net) = &self.net else { return Vec::new() };
        let found = net.discovery.devices();
        let list = found.iter().filter_map(|d| {
            let http = d.get_best_channel()?.http()?;
            let on_wifi = http.host.parse().is_ok_and(|ip| policy::same_network(ip, net.address, net.netmask));
            (on_wifi && d.device.fingerprint != self.identity.fingerprint).then_some(())?;
            Some(ipc::Device {
                alias: ipc::tidy(&d.device.alias, 120),
                fingerprint: d.device.fingerprint.clone(),
                host: http.host.clone(),
                port: http.port,
                https: http.protocol == ProtocolType::Https,
                kind: match d.device.device_type {
                    Some(DeviceType::Mobile) => ipc::DeviceKind::Mobile,
                    Some(DeviceType::Desktop) => ipc::DeviceKind::Desktop,
                    Some(DeviceType::Web) => ipc::DeviceKind::Web,
                    _ => ipc::DeviceKind::Other,
                },
            })
        });
        outgoing::sorted(list.collect()).into_iter().take(32).collect()
    }

    fn send_request_of(out: &Out) -> FromDaemon {
        FromDaemon::SendRequest {
            id: out.request.id.clone(),
            count: out.request.count() as u32,
            bytes: out.request.bytes(),
            name: ipc::tidy(out.request.first_name(), 120),
        }
    }

    /// A requester's connection has something to say.
    async fn on_submit(&mut self, conn: u64, event: SubmitEvent) {
        match event {
            SubmitEvent::Go { root, files, to, reply } => {
                let refuse = |why: SubmitRefusal| {
                    tell_requester(&reply, FromSubmit::Refused(why));
                    let _ = reply.shutdown(std::net::Shutdown::Both);
                };
                let entries: Vec<Entry<Option<std::fs::File>>> = files.into_iter().map(|(name, size, file)| Entry { conn, name, size, payload: Some(file) }).collect();
                // The target is the owner's to choose. Only root's own command may name one.
                if to.is_some() && !root {
                    return refuse(SubmitRefusal::NotAllowed);
                }
                if self.pending.is_some() || self.active.is_some() {
                    return refuse(SubmitRefusal::Busy);
                }
                if let Some(out) = self.outgoing.as_mut() {
                    // More files for the picker that is already up.
                    let added = if to.is_some() { Err(SubmitRefusal::Busy) } else { out.request.add(entries) };
                    if let Err(why) = added {
                        return refuse(why);
                    }
                    out.replies.insert(conn, reply.clone());
                    tell_requester(&reply, FromSubmit::Queued);
                    let message = Self::send_request_of(out);
                    return self.tell(message);
                }
                let auto = to.is_some();
                if !auto && !self.shell_connected() {
                    return refuse(SubmitRefusal::NoScreen);
                }
                self.requests += 1;
                let now = Instant::now();
                let out = Out {
                    request: Request::new(format!("o-{}-{}", std::process::id(), self.requests), conn, entries, to),
                    replies: HashMap::from([(conn, reply.clone())]),
                    searching: false,
                    listed: None,
                    deadline: now + Duration::from_secs(policy::SEND_WAIT_SECONDS),
                    cancel: CancellationToken::new(),
                    pin_tx: None,
                    done: Arc::new(AtomicU64::new(0)),
                    file_no: Arc::new(AtomicU32::new(0)),
                    count: 0,
                    total: 0,
                    reported: None,
                    moved: now,
                    gave_up: None,
                };
                if !auto {
                    tell_requester(&reply, FromSubmit::Queued);
                    self.tell(Self::send_request_of(&out));
                }
                self.outgoing = Some(out);
                // Brings the network up if only this request needs it, and looks for devices.
                self.check_address().await;
                self.start_search();
            }
            SubmitEvent::Confirm => {
                if let Some(device) = self.outgoing.as_mut().and_then(|o| o.request.confirm(conn)) {
                    self.begin_send(device).await;
                }
            }
            SubmitEvent::Gone => {
                let Some(out) = self.outgoing.as_mut() else { return };
                out.replies.remove(&conn);
                match out.request.left(conn) {
                    Left::Unchanged => {}
                    Left::Fewer => {
                        let message = Self::send_request_of(out);
                        self.tell(message);
                    }
                    Left::Empty => self.finish_send(SendOutcome::Cancelled, 0).await,
                }
            }
        }
    }

    /// Looks for devices for the request that is waiting for a target: an announcement (devices
    /// answer it on our port), and, when nothing answers, a scan of the Wi-Fi network.
    fn start_search(&mut self) {
        let Some(out) = self.outgoing.as_mut().filter(|o| o.request.stage == Stage::Picking && !o.searching) else { return };
        let Some(net) = &self.net else { return };
        out.searching = true;
        let (discovery, address, events, id) = (net.discovery.clone(), net.address, self.events.clone(), out.request.id.clone());
        tokio::spawn(async move {
            let staged = discovery.discover_staged(Vec::new(), vec![address], net::PORT, ProtocolType::Https, Duration::from_secs(2));
            let _ = tokio::time::timeout(Duration::from_secs(10), staged).await;
            let _ = events.send(Event::SearchDone(id));
        });
    }

    /// The target is settled (by the owner in the shell's picker, or by root's confirmed --to):
    /// the files start on their way. Only ever to an address on the Wi-Fi network.
    async fn begin_send(&mut self, device: ipc::Device) {
        let on_wifi = self.net.as_ref().is_some_and(|net| device.host.parse().is_ok_and(|ip| policy::same_network(ip, net.address, net.netmask)));
        let Some(out) = self.outgoing.as_mut().filter(|_| on_wifi) else {
            return self.finish_send(SendOutcome::Unreachable, 0).await;
        };
        let files: Vec<(String, u64, std::fs::File)> = out.request.files.iter_mut().filter_map(|f| Some((f.name.clone(), f.size, f.payload.take()?))).collect();
        let (pin_tx, pin_rx) = mpsc::channel(1);
        out.pin_tx = Some(pin_tx);
        out.deadline = Instant::now() + Duration::from_secs(policy::SEND_WAIT_SECONDS);
        (out.count, out.total) = (out.request.count() as u32, out.request.bytes());
        let job = SendJob {
            id: out.request.id.clone(),
            alias: self.identity.alias.clone(),
            fingerprint: self.identity.fingerprint.clone(),
            cert_pem: self.identity.cert_pem.clone(),
            key_pem: self.identity.key_pem.clone(),
            device: device.clone(),
            files,
            pin_rx,
            events: self.events.clone(),
            cancel: out.cancel.clone(),
            done: out.done.clone(),
            file_no: out.file_no.clone(),
        };
        let message = FromDaemon::SendProgress { id: out.request.id.clone(), to: device.alias, file: 0, count: out.count, done: 0, total: out.total };
        self.tell(message);
        tokio::spawn(async move {
            let (id, events) = (job.id.clone(), job.events.clone());
            let (outcome, sent) = send_files(job).await;
            let _ = events.send(Event::Send(id, SendEvent::Ended { outcome, sent }));
        });
    }

    /// The send request is over, however it went: the shell and the requesters are told, the
    /// files are closed, and the port closes again if only the request needed it.
    async fn finish_send(&mut self, outcome: SendOutcome, sent: u32) {
        let Some(out) = self.outgoing.take() else { return };
        out.cancel.cancel();
        let to = out.request.target.as_ref().map(|d| d.alias.clone()).unwrap_or_default();
        self.tell(FromDaemon::SendDone { id: out.request.id.clone(), to: to.clone(), outcome, sent });
        for reply in out.replies.values() {
            tell_requester(reply, FromSubmit::Result { outcome, sent, to: to.clone() });
            let _ = reply.shutdown(std::net::Shutdown::Both);
        }
        self.idle_since = Instant::now();
        self.check_address().await;
    }

    /// Twice a second while a send request exists: the device list for the picker, progress,
    /// and the end of waiting.
    async fn send_tick(&mut self) {
        let list = match self.outgoing.as_ref().map(|o| o.request.stage) {
            Some(Stage::Picking) => self.device_list(),
            Some(_) => Vec::new(),
            None => return,
        };
        let now = Instant::now();
        let mut say = Vec::new();
        let mut end = None;
        let Some(out) = self.outgoing.as_mut() else { return };
        let id = out.request.id.clone();
        match out.request.stage {
            Stage::Picking if out.request.is_auto() => {
                if let Some(device) = out.request.find_wanted(&list) {
                    out.deadline = now + Duration::from_secs(policy::SEND_WAIT_SECONDS);
                    for reply in out.replies.values() {
                        tell_requester(reply, FromSubmit::Target(device.clone()));
                    }
                } else if !out.searching {
                    end = Some(SendOutcome::NoSuchDevice);
                }
            }
            Stage::Picking => {
                if out.request.show(list.clone()) || out.listed != Some(out.searching) {
                    out.listed = Some(out.searching);
                    say.push(FromDaemon::SendDevices { id, searching: out.searching, list });
                }
            }
            Stage::Uploading => {
                let at = (out.file_no.load(Ordering::Relaxed), out.done.load(Ordering::Relaxed).min(out.total));
                if out.reported != Some(at) {
                    (out.reported, out.moved) = (Some(at), now);
                    let to = out.request.target.as_ref().map(|d| d.alias.clone()).unwrap_or_default();
                    say.push(FromDaemon::SendProgress { id, to, file: at.0, count: out.count, done: at.1, total: out.total });
                    for reply in out.replies.values() {
                        tell_requester(reply, FromSubmit::Progress { done: at.1, total: out.total });
                    }
                } else if out.moved.elapsed() > IDLE && out.gave_up.is_none() {
                    // Nothing moved for too long: the task is stopped and reports back.
                    out.gave_up = Some(SendOutcome::Failed);
                    out.cancel.cancel();
                }
            }
            Stage::Confirming | Stage::Waiting | Stage::PinWanted => {}
        }
        if end.is_none() && now >= out.deadline {
            match out.request.stage {
                // Nobody chose, confirmed or typed: the request is dropped.
                Stage::Picking | Stage::Confirming | Stage::PinWanted => end = Some(SendOutcome::Cancelled),
                // The other side never answered: stop asking (the task reports back).
                Stage::Waiting if out.gave_up.is_none() => {
                    out.gave_up = Some(SendOutcome::NoAnswer);
                    out.cancel.cancel();
                }
                _ => {}
            }
        }
        for message in say {
            self.tell(message);
        }
        if let Some(outcome) = end {
            self.finish_send(outcome, 0).await;
        }
    }

    async fn on_tick(&mut self) {
        self.send_tick().await;
        if self.pending.as_ref().is_some_and(|p| Instant::now() >= p.deadline) {
            self.drop_pending();
        }
        let mut stalled = false;
        if let Some(active) = self.active.as_mut() {
            let done = active.done.load(Ordering::Relaxed);
            if done != active.reported {
                active.reported = done;
                active.moved = Instant::now();
                let message = FromDaemon::Progress { id: active.id.clone(), done, total: active.total };
                self.tell(message);
            } else {
                stalled = active.moved.elapsed() > IDLE;
            }
        }
        if stalled {
            self.abort_active().await;
        }
    }

    async fn on_server(&mut self, event: ServerEventV2) {
        match event {
            ServerEventV2::Register { ip, info } => self.seen(ip, info),
            ServerEventV2::PrepareUpload { session_id, ip, info, files, decision_tx, .. } => {
                let from = ipc::tidy(&info.alias, 120);
                self.seen(ip, info);
                let Some(net) = &self.net else { return };
                let decline = |tx: oneshot::Sender<PrepareUploadDecisionV2>| {
                    let _ = tx.send(PrepareUploadDecisionV2::Decline);
                };
                // Receiving off (the port is only open for a search), or busy sending: no, and
                // nobody is asked. Then: only from the Wi-Fi network, one thing at a time.
                let welcome = policy::offers_welcome(self.receiving, self.outgoing.is_some());
                if !welcome || !policy::same_network(ip.ip, net.address, net.netmask) || self.pending.is_some() || self.active.is_some() {
                    return decline(decision_tx);
                }
                // A text message: one "file" that carries its text. Nothing is written.
                if files.len() == 1 {
                    if let Some(text) = files.values().next().and_then(|f| f.preview.as_deref()) {
                        let text = ipc::tidy(text, 600);
                        let _ = decision_tx.send(PrepareUploadDecisionV2::Accept(Default::default()));
                        return self.tell(FromDaemon::Text { from, text });
                    }
                }
                let bytes = files.values().map(|f| f.size).fold(0u64, u64::saturating_add);
                if let Err(why) = policy::check_size(files.len(), bytes, net::free_space(&self.dir).unwrap_or(0)) {
                    decline(decision_tx);
                    return self.tell(FromDaemon::Refused { from, why });
                }
                let pending = Pending { id: session_id, from, files, decision: decision_tx, deadline: Instant::now() + Duration::from_secs(policy::OFFER_SECONDS) };
                match policy::verdict(self.quick, self.shell_connected()) {
                    policy::Verdict::Accept => self.accept(pending),
                    policy::Verdict::Decline => decline(pending.decision),
                    policy::Verdict::Ask => {
                        self.tell(self.offer_of(&pending));
                        self.pending = Some(pending);
                    }
                }
            }
            ServerEventV2::PrepareUploadAborted { session_id } => {
                if self.pending.as_ref().is_some_and(|p| p.id == session_id) {
                    self.pending = None; // its answer channel is dead already
                    self.tell(FromDaemon::OfferGone { id: session_id });
                }
            }
            ServerEventV2::FileUpload { session_id, file_id, file, target_tx } => self.receive_file(session_id, file_id, file, target_tx),
            ServerEventV2::SessionEnd { session_id, .. } => {
                if let Some(active) = self.active.as_mut().filter(|a| a.id == session_id) {
                    active.ended = true;
                    self.finish_if_over();
                }
            }
            // A peer cancels a transfer we send to it by answering our upload with an error, not
            // through this server; a cancel request arriving here is about nothing of ours.
            ServerEventV2::CancelReceived { .. } => {}
            ServerEventV2::ListenerFailed { error } => {
                eprintln!("shabnam-dropd: the listener failed: {error}");
                if let Some(net) = self.net.take() {
                    net.stop().await;
                }
                self.check_address().await;
            }
        }
    }

    /// A device that reached us goes into the list `shabnam-send` asks for.
    fn seen(&self, ip: PeerIp, info: RegisterDtoV2) {
        let Some(net) = &self.net else { return };
        if info.fingerprint == self.identity.fingerprint || !policy::same_network(ip.ip, net.address, net.netmask) {
            return;
        }
        let (discovery, device) = (net.discovery.clone(), device_of(ip, info));
        tokio::spawn(async move {
            discovery.observe_inbound(device).await;
        });
    }

    /// One file of the accepted transfer starts to arrive: it goes into a hidden part file that
    /// is created here (never an existing one, never through a link) and gets its name when complete.
    fn receive_file(&mut self, session: String, file_id: String, file: FileDto, target: oneshot::Sender<FileUploadTarget>) {
        use std::os::unix::fs::OpenOptionsExt;
        let Some(active) = self.active.as_mut().filter(|a| a.id == session && !a.ended) else { return };
        // Each accepted file once, a few at a time; its size is the one that was accepted, not
        // whatever this request claims. A refused request is answered with an error (`target` dropped).
        let size = match active.uploads.begin(&file_id) {
            Ok(size) => size,
            Err(why) => {
                eprintln!("shabnam-dropd: an upload was refused: {why:?}");
                return;
            }
        };
        let total = active.total;
        self.parts += 1;
        let part = names::part_path(&self.dir, &session, self.parts);
        let opened = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o644).custom_flags(libc::O_NOFOLLOW).open(&part);
        let handle = match opened {
            Ok(handle) => handle,
            Err(e) => {
                eprintln!("shabnam-dropd: {}: {e}", part.display());
                active.uploads.end(&file_id, false);
                active.failed += 1;
                return; // dropping `target` answers the sender with an error
            }
        };
        let (binary_tx, binary_rx) = mpsc::channel::<Bytes>(16);
        let (result_tx, result_rx) = oneshot::channel();
        if target.send(FileUploadTarget::Stream { binary_tx, result_rx }).is_err() {
            let _ = std::fs::remove_file(&part);
            active.uploads.end(&file_id, false);
            active.failed += 1;
            return;
        }
        active.writers += 1;
        let (done, cancel, dir, events) = (active.done.clone(), active.cancel.clone(), self.dir.clone(), self.events.clone());
        let wanted = names::sanitize(&file.file_name);
        tokio::spawn(async move {
            let written = write_part(binary_rx, handle, size, total, done, cancel).await;
            let result = match written {
                Ok(()) => names::publish(&dir, &part, &wanted).map_err(|e| e.to_string()),
                Err(e) => {
                    let _ = std::fs::remove_file(&part);
                    Err(e)
                }
            };
            let _ = result_tx.send(result.as_ref().map(|_| ()).map_err(|e| e.clone()));
            let _ = events.send(Event::FileDone { session, file_id, result });
        });
    }
}

/// What the task that talks to the target device needs.
struct SendJob {
    id: String,
    alias: String,
    fingerprint: String,
    cert_pem: String,
    key_pem: String,
    device: ipc::Device,
    files: Vec<(String, u64, std::fs::File)>,
    pin_rx: mpsc::Receiver<String>,
    events: mpsc::UnboundedSender<Event>,
    cancel: CancellationToken,
    done: Arc<AtomicU64>,
    file_no: Arc<AtomicU32>,
}

fn file_type(name: &str) -> &'static str {
    let extension = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "apk" => "application/vnd.android.package-archive",
        "txt" | "log" | "md" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Feeds a handed-over file to the upload: exactly the `size` bytes that were announced, read
/// from the descriptor at our own offsets (the requester shares the descriptor and could move
/// its position). A file that ends early or cannot be read fails the whole send — a short
/// upload must not pass for the whole file.
fn content_of(file: std::fs::File, size: u64, cancel: CancellationToken, unreadable: Arc<AtomicBool>) -> FileContent {
    let (tx, rx) = mpsc::channel::<Bytes>(4);
    tokio::task::spawn_blocking(move || {
        use std::os::unix::fs::FileExt;
        let mut offset = 0u64;
        while offset < size {
            let mut chunk = vec![0u8; (size - offset).min(256 * 1024) as usize];
            match file.read_at(&mut chunk, offset) {
                Ok(n) if n > 0 => {
                    chunk.truncate(n);
                    offset += n as u64;
                    if tx.blocking_send(Bytes::from(chunk)).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                _ => {
                    unreadable.store(true, Ordering::SeqCst);
                    cancel.cancel();
                    break;
                }
            }
        }
    });
    FileContent::Stream(rx)
}

/// Asks the device, waits for its yes (and for a PIN if it wants one), uploads file after file.
/// The connection accepts only the certificate the device showed when it was found.
async fn send_files(mut job: SendJob) -> (SendOutcome, u32) {
    let device = &job.device;
    let protocol = if device.https { ProtocolType::Https } else { ProtocolType::Http };
    let pinned = device.https.then(|| device.fingerprint.clone());
    let Ok(client) = LsHttpClient::new(&job.key_pem, &job.cert_pem, LsHttpClientVersion::V2, pinned, None) else { return (SendOutcome::Failed, 0) };
    let mut dtos = HashMap::new();
    let mut sources = HashMap::new();
    for (i, (name, size, file)) in job.files.drain(..).enumerate() {
        let id = format!("f{i:04}");
        dtos.insert(id.clone(), FileDto { id: id.clone(), file_name: name.clone(), size, file_type: file_type(&name).to_string(), sha256: None, preview: None, metadata: None });
        sources.insert(id, (size, file));
    }
    let mut pin: Option<String> = None;
    let prepared = loop {
        let info = RegisterDto {
            alias: job.alias.clone(),
            version: PROTOCOL_VERSION_V2.to_string(),
            device_model: Some("Shabnam".into()),
            device_type: Some(DeviceType::Desktop),
            token: job.fingerprint.clone(),
            port: net::PORT,
            protocol: ProtocolType::Https,
            has_web_interface: false,
        };
        let request = PrepareUploadRequestDto { info, files: dtos.clone() };
        match client.prepare_upload(protocol, &device.host, device.port, None, request, pin.as_deref(), job.cancel.clone()).await {
            Ok(prepared) => break prepared,
            Err(ClientError::Cancelled) => return (SendOutcome::Cancelled, 0),
            Err(ClientError::StatusCode(e)) => match e.status {
                401 => {
                    let _ = job.events.send(Event::Send(job.id.clone(), SendEvent::PinWanted { wrong: pin.is_some() }));
                    tokio::select! {
                        typed = job.pin_rx.recv() => match typed {
                            Some(typed) => pin = Some(typed),
                            None => return (SendOutcome::Cancelled, 0),
                        },
                        _ = job.cancel.cancelled() => return (SendOutcome::Cancelled, 0),
                    }
                }
                403 => return (SendOutcome::Declined, 0),
                409 => return (SendOutcome::Busy, 0),
                // Too many wrong PINs.
                429 => return (SendOutcome::PinNeeded, 0),
                status => {
                    eprintln!("shabnam-dropd: the device answered the request with HTTP {status}");
                    return (SendOutcome::Failed, 0);
                }
            },
            Err(e) if e.is_unreachable() => return (SendOutcome::Unreachable, 0),
            Err(e) => {
                eprintln!("shabnam-dropd: the request failed: {e}");
                return (SendOutcome::Failed, 0);
            }
        }
    };
    let Some(response) = prepared.response.filter(|r| !r.session_id.is_empty() && !r.files.is_empty()) else {
        return (SendOutcome::Declined, 0);
    };
    let mut ids: Vec<&String> = response.files.keys().filter(|id| sources.contains_key(*id)).collect();
    ids.sort();
    let total = ids.iter().map(|id| sources[*id].0).fold(0u64, u64::saturating_add);
    let _ = job.events.send(Event::Send(job.id.clone(), SendEvent::Accepted { count: ids.len() as u32, total }));
    let unreadable = Arc::new(AtomicBool::new(false));
    let (mut base, mut sent) = (0u64, 0u32);
    for (n, id) in ids.into_iter().enumerate() {
        let Some((size, file)) = sources.remove(id) else { continue };
        job.file_no.store(n as u32 + 1, Ordering::Relaxed);
        let done = job.done.clone();
        let progress = move |bytes: u64| done.store(base + bytes.min(size), Ordering::Relaxed);
        let content = content_of(file, size, job.cancel.clone(), unreadable.clone());
        let uploaded = client.upload(protocol, &device.host, device.port, None, &response.session_id, id, &response.files[id], content, progress, job.cancel.clone()).await;
        if let Err(e) = uploaded {
            let _ = tokio::time::timeout(Duration::from_secs(3), client.cancel(protocol, &device.host, device.port, &response.session_id)).await;
            let outcome = match e {
                _ if unreadable.load(Ordering::SeqCst) => SendOutcome::FileChanged,
                ClientError::Cancelled => SendOutcome::Cancelled,
                e if e.is_unreachable() => SendOutcome::Unreachable,
                e => {
                    eprintln!("shabnam-dropd: an upload failed: {e}");
                    SendOutcome::Failed
                }
            };
            return (outcome, sent);
        }
        base += size;
        job.done.store(base, Ordering::Relaxed);
        sent += 1;
    }
    (SendOutcome::Sent, sent)
}
