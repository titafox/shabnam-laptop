//! shabnam-dropd — the receiving side of 隔空传送. Runs as the unprivileged user "shabnam" (the
//! wrapper `shabnam-drop start` drops to it), only while the owner has switched receiving on.
//!
//!   shabnam-dropd                 run (one per user); stops on `quit` from the shell or SIGTERM
//!   shabnam-dropd ctl JSON [SEC]  testing: send one control line to the running daemon and print
//!                                 what it says for SEC seconds (default 1)
//!
//! What it does: listens on the Wi-Fi interface's address only (HTTPS, port 53317, and the
//! discovery group on that interface), speaking the LocalSend protocol through that project's own
//! core library. Every offer is checked (sender on the same network, size limits, free space) and
//! then shown by the desktop shell, which this daemon talks to over a unix socket in its 0700
//! runtime directory (messages: ../ipc.rs). Without an answer from the shell nothing is accepted,
//! unless the owner switched on "quick receive". Accepted files are written to ~/Downloads under a
//! rewritten name (../names.rs), never over an existing file, mode 0644, and are never opened or
//! run by this program.

use bytes::Bytes;
use localsend::discovery::{DeviceChannel, DiscoveredDevice, DiscoveryEvent, HttpChannel};
use localsend::http::dto_v2::RegisterDtoV2;
use localsend::http::server::common::save::FileUploadTarget;
use localsend::http::server::v2::{PrepareUploadDecisionV2, ServerEventV2};
use localsend::http::server::PeerIp;
use localsend::model::transfer::FileDto;
use shabnam_drop::ipc::{self, FromDaemon, ToDaemon};
use shabnam_drop::net::{self, Identity, Net};
use shabnam_drop::{names, policy};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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

enum Event {
    Client(u64, ClientEvent),
    FileDone { session: String, file_id: String, result: Result<String, String> },
    DevicesDue(u64),
    Stop,
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
    clients: HashMap<u64, (bool, mpsc::UnboundedSender<String>)>,
    pending: Option<Pending>,
    active: Option<Active>,
    parts: u32,
    events: mpsc::UnboundedSender<Event>,
    server_tx: mpsc::Sender<ServerEventV2>,
    discovery_tx: mpsc::Sender<DiscoveryEvent>,
}

fn runtime_dir() -> Result<PathBuf, String> {
    use std::os::unix::fs::MetadataExt;
    let uid = unsafe { libc::getuid() };
    let dir = std::env::var("XDG_RUNTIME_DIR").ok().filter(|d| d.starts_with('/')).unwrap_or_else(|| format!("/run/user/{uid}"));
    let meta = std::fs::symlink_metadata(&dir).map_err(|e| format!("runtime directory {dir}: {e}"))?;
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(format!("runtime directory {dir} must be a directory of this user with mode 0700"));
    }
    Ok(PathBuf::from(dir))
}

fn socket_path() -> Result<PathBuf, String> {
    Ok(runtime_dir()?.join(ipc::SOCKET_NAME))
}

/// ~/Downloads as a real directory (created on first use; a link in its place is refused).
fn downloads_dir() -> Result<PathBuf, String> {
    use std::os::unix::fs::DirBuilderExt;
    let home = std::env::var("HOME").ok().filter(|h| h.starts_with('/')).ok_or("HOME is not set")?;
    let dir = Path::new(&home).join("Downloads");
    match std::fs::symlink_metadata(&dir) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err(format!("{} is not a directory", dir.display())),
        Err(_) => std::fs::DirBuilder::new().mode(0o755).create(&dir).map_err(|e| format!("{}: {e}", dir.display()))?,
    }
    Ok(dir)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None => run(),
        Some("ctl") if args.len() >= 2 => ctl(&args[1], args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1)),
        _ => Err("usage: shabnam-dropd [ctl JSON [SECONDS]]".into()),
    };
    if let Err(e) = result {
        eprintln!("shabnam-dropd: {e}");
        std::process::exit(1);
    }
}

/// Testing aid: one line to the daemon, then print its lines for a while.
fn ctl(line: &str, seconds: u64) -> Result<(), String> {
    use std::io::{Read, Write};
    let uid = unsafe { libc::getuid() };
    let path = match uid {
        0 => PathBuf::from(format!("/run/user/{}/{}", std::env::var("SHABNAM_DROP_UID").unwrap_or_else(|_| "700".into()), ipc::SOCKET_NAME)),
        _ => socket_path()?,
    };
    let mut stream = std::os::unix::net::UnixStream::connect(&path).map_err(|e| format!("{}: {e}", path.display()))?;
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
        let daemon = Daemon {
            identity,
            interface,
            dir,
            net: None,
            quick: false,
            clients: HashMap::new(),
            pending: None,
            active: None,
            parts: 0,
            events,
            server_tx,
            discovery_tx,
        };
        daemon.serve(events_rx, server_rx, discovery_rx).await;
        let _ = std::fs::remove_file(&path);
        Ok(())
    })
}

async fn stop_on_signal(events: mpsc::UnboundedSender<Event>) {
    use tokio::signal::unix::{signal, SignalKind};
    let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else { return };
    tokio::select! { _ = term.recv() => {}, _ = int.recv() => {} }
    let _ = events.send(Event::Stop);
}

/// Control connections: root (the shell) may do everything, this user (`shabnam-send`) may only
/// ask for the device list, anybody else is turned away.
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
/// the stream ends early, or when the transfer is cancelled.
async fn write_part(mut chunks: mpsc::Receiver<Bytes>, file: std::fs::File, size: u64, done: Arc<AtomicU64>, cancel: CancellationToken) -> Result<(), String> {
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
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        done.fetch_add(chunk.len() as u64, Ordering::Relaxed);
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
                }
            }
        }
        self.drop_pending();
        self.abort_active().await;
        if let Some(net) = self.net.take() {
            net.stop().await;
        }
    }

    fn state(&self) -> FromDaemon {
        FromDaemon::State { alias: self.identity.alias.clone(), address: self.net.as_ref().map(|n| n.address.to_string()), quick: self.quick }
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
    /// when it is gone.
    async fn check_address(&mut self) {
        let now = net::interface_address(&self.interface);
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
                }
                self.clients.insert(id, (root, tx));
            }
            Event::Client(id, ClientEvent::Gone) => {
                self.clients.remove(&id);
                if !self.shell_connected() {
                    self.drop_pending(); // nobody is left to answer
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
                }
            }
            Event::DevicesDue(id) => {
                let list = self.net.as_ref().map(|n| n.discovery.devices()).unwrap_or_default();
                let list = list
                    .iter()
                    .filter_map(|d| {
                        let http = d.get_best_channel()?.http()?;
                        Some(ipc::Device {
                            alias: ipc::tidy(&d.device.alias, 120),
                            fingerprint: d.device.fingerprint.clone(),
                            host: http.host.clone(),
                            port: http.port,
                            https: http.protocol == localsend::model::discovery::ProtocolType::Https,
                        })
                    })
                    .collect();
                if let Some((_, tx)) = self.clients.get(&id) {
                    let _ = tx.send(FromDaemon::Devices(list).encode());
                }
            }
            Event::FileDone { session, file_id, result } => {
                if let Some(active) = self.active.as_mut().filter(|a| a.id == session) {
                    active.writers = active.writers.saturating_sub(1);
                    active.moved = Instant::now();
                    match result {
                        Ok(name) => {
                            active.saved.insert(file_id, name);
                        }
                        Err(e) => {
                            active.failed += 1;
                            eprintln!("shabnam-dropd: a file was not saved: {e}");
                        }
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

    async fn on_tick(&mut self) {
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
                if !policy::same_network(ip.ip, net.address, net.netmask) || self.pending.is_some() || self.active.is_some() {
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
            // This daemon sends nothing, so there is no transfer of ours a peer could cancel.
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
        // A second attempt at the same file (after a checksum mismatch) replaces the first.
        if let Some(old) = active.saved.remove(&file_id) {
            let _ = std::fs::remove_file(self.dir.join(old));
        }
        self.parts += 1;
        let part = names::part_path(&self.dir, &session, self.parts);
        let opened = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o644).custom_flags(libc::O_NOFOLLOW).open(&part);
        let handle = match opened {
            Ok(handle) => handle,
            Err(e) => {
                eprintln!("shabnam-dropd: {}: {e}", part.display());
                active.failed += 1;
                return; // dropping `target` answers the sender with an error
            }
        };
        let (binary_tx, binary_rx) = mpsc::channel::<Bytes>(16);
        let (result_tx, result_rx) = oneshot::channel();
        if target.send(FileUploadTarget::Stream { binary_tx, result_rx }).is_err() {
            let _ = std::fs::remove_file(&part);
            active.failed += 1;
            return;
        }
        active.writers += 1;
        let (done, cancel, dir, events) = (active.done.clone(), active.cancel.clone(), self.dir.clone(), self.events.clone());
        let wanted = names::sanitize(&file.file_name);
        tokio::spawn(async move {
            let written = write_part(binary_rx, handle, file.size, done, cancel).await;
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
