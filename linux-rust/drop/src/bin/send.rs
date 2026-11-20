//! shabnam-send [--to NAME-or-ADDRESS] FILE... — sends files to a LocalSend-compatible device on
//! the home Wi-Fi. Without --to it lists the devices it found and asks which one.
//!
//! Who it runs as: the desktop runs as root, and a program that talks to other machines must
//! not. Started by root, this program opens the files first (so files under /root can be sent),
//! then gives up root for good — it becomes the user "shabnam" — and only then touches the
//! network. The other device sees the same name and certificate as the receiver (shabnam-dropd).
//!
//! Devices: when the receiver is running it is asked for the devices it knows (it owns the
//! protocol's port); otherwise this program listens on the Wi-Fi address itself for the few
//! seconds the search takes, declining anything offered to it meanwhile.

use bytes::Bytes;
use localsend::http::client::{ClientError, LsHttpClient, LsHttpClientVersion};
use localsend::http::dto::{PrepareUploadRequestDto, RegisterDto};
use localsend::http::server::v2::{PrepareUploadDecisionV2, ServerEventV2};
use localsend::model::discovery::{DeviceType, ProtocolType, PROTOCOL_VERSION_V2};
use localsend::model::transfer::{FileContent, FileDto};
use shabnam_drop::ipc::{self, Device, FromDaemon, ToDaemon};
use shabnam_drop::net::{self, Identity, Net};
use shabnam_drop::policy::{self, Lang};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const USER: &str = "shabnam";

/// Everything this program says, in the system's three languages (zh, ru, en).
#[derive(Clone, Copy)]
enum Say {
    Usage,
    NotAFile,
    Searching,
    NoDevices,
    NoWifi,
    Choose,
    NoSuchDevice,
    Waiting,
    Declined,
    Busy,
    PinNeeded,
    Sent,
    Failed,
    Cancelled,
    PressEnter,
}

fn say(lang: Lang, what: Say) -> &'static str {
    let (zh, ru, en) = match what {
        Say::Usage => ("用法：shabnam-send [--to 设备名或地址] 文件…", "Использование: shabnam-send [--to ИМЯ-или-АДРЕС] ФАЙЛ…", "usage: shabnam-send [--to NAME-or-ADDRESS] FILE..."),
        Say::NotAFile => ("不是普通文件，已跳过", "не обычный файл, пропущен", "not a regular file, skipped"),
        Say::Searching => ("正在查找附近的设备…", "Поиск устройств поблизости…", "Looking for nearby devices…"),
        Say::NoDevices => ("没有找到设备。请在对方设备上打开 LocalSend，并确认连着同一个 Wi-Fi。", "Устройства не найдены. Откройте LocalSend на другом устройстве в той же сети Wi-Fi.", "No devices found. Open LocalSend on the other device, on the same Wi-Fi."),
        Say::NoWifi => ("Wi-Fi 未连接。", "Wi-Fi не подключён.", "Wi-Fi is not connected."),
        Say::Choose => ("发送到哪台设备？输入编号后回车（直接回车取消）：", "На какое устройство отправить? Номер и Enter (пустая строка — отмена): ", "Send to which device? Number, then Enter (empty to cancel): "),
        Say::NoSuchDevice => ("找不到这台设备", "Устройство не найдено", "No such device"),
        Say::Waiting => ("等待对方接受…", "Ожидание подтверждения…", "Waiting for the other side to accept…"),
        Say::Declined => ("对方拒绝了。", "Получатель отклонил передачу.", "The other side declined."),
        Say::Busy => ("对方正忙。", "Получатель занят.", "The other side is busy."),
        Say::PinNeeded => ("对方要求 PIN，暂不支持。", "Получатель требует PIN — пока не поддерживается.", "The other side wants a PIN, which is not supported yet."),
        Say::Sent => ("已发送", "Отправлено", "Sent"),
        Say::Failed => ("发送失败", "Не удалось отправить", "Sending failed"),
        Say::Cancelled => ("已取消。", "Отменено.", "Cancelled."),
        Say::PressEnter => ("按回车关闭。", "Нажмите Enter, чтобы закрыть.", "Press Enter to close."),
    };
    match lang {
        Lang::Zh => zh,
        Lang::Ru => ru,
        Lang::En => en,
    }
}

struct Outgoing {
    name: String,
    size: u64,
    file: std::fs::File,
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

/// Becomes the unprivileged user for good. Called before any network code runs.
fn drop_root() -> Result<(), String> {
    if unsafe { libc::geteuid() } != 0 {
        return Ok(());
    }
    let name = std::ffi::CString::new(USER).unwrap();
    let entry = unsafe { libc::getpwnam(name.as_ptr()) };
    if entry.is_null() {
        return Err(format!("user {USER} is missing"));
    }
    let (uid, gid) = unsafe { ((*entry).pw_uid, (*entry).pw_gid) };
    let home = unsafe { std::ffi::CStr::from_ptr((*entry).pw_dir) }.to_string_lossy().into_owned();
    if uid == 0 {
        return Err(format!("user {USER} is root"));
    }
    let done = unsafe { libc::setgroups(1, &gid) == 0 && libc::setgid(gid) == 0 && libc::setuid(uid) == 0 };
    // It must not be possible to get root back.
    if !done || unsafe { libc::setuid(0) } == 0 || unsafe { libc::getuid() } != uid {
        return Err("could not give up root".into());
    }
    std::env::set_var("HOME", &home);
    std::env::set_var("USER", USER);
    std::env::set_var("XDG_RUNTIME_DIR", format!("/run/user/{uid}"));
    for name in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "LD_LIBRARY_PATH", "LD_PRELOAD"] {
        std::env::remove_var(name);
    }
    let _ = std::env::set_current_dir(&home);
    Ok(())
}

fn main() {
    let lang = policy::lang();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // In a terminal window of its own (from the file manager) the result must stay readable.
    let hold = args.first().is_some_and(|a| a == "--hold");
    if hold {
        args.remove(0);
    }
    let mut to = None;
    if args.first().is_some_and(|a| a == "--to") && args.len() >= 2 {
        to = Some(args.remove(1));
        args.remove(0);
    }
    let code = match run(lang, to, args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("shabnam-send: {e}");
            1
        }
    };
    if hold {
        println!("\n{}", say(lang, Say::PressEnter));
        let _ = std::io::stdin().read_line(&mut String::new());
    }
    std::process::exit(code);
}

fn run(lang: Lang, to: Option<String>, paths: Vec<String>) -> Result<(), String> {
    unsafe { libc::umask(0o077) };
    if paths.is_empty() || paths.iter().any(|p| p.starts_with("--")) {
        return Err(say(lang, Say::Usage).into());
    }
    // As whoever started us: open what is to be sent. Only regular files, opened once; what is
    // sent later is read from these open files, not looked up by name again.
    let mut files = Vec::new();
    for path in &paths {
        let file = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
        let meta = file.metadata().map_err(|e| format!("{path}: {e}"))?;
        if !meta.is_file() {
            eprintln!("{path}: {}", say(lang, Say::NotAFile));
            continue;
        }
        let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
        files.push(Outgoing { name, size: meta.len(), file });
    }
    if files.is_empty() {
        return Err(say(lang, Say::Usage).into());
    }
    drop_root()?;
    if std::env::var_os("SHABNAM_DROP_LOG").is_some() {
        let _ = tracing_subscriber::fmt().with_writer(std::io::stderr).try_init();
    }
    let identity = Identity::load_or_generate(&net::config_dir()?, policy::alias(lang))?;
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().map_err(|e| e.to_string())?;
    runtime.block_on(send(lang, identity, to, files))
}

/// Asks the running receiver for its device list; None when it is not running.
fn devices_from_daemon() -> Option<Vec<Device>> {
    let uid = unsafe { libc::getuid() };
    let dir = std::env::var("XDG_RUNTIME_DIR").ok().filter(|d| d.starts_with('/')).unwrap_or_else(|| format!("/run/user/{uid}"));
    let mut stream = std::os::unix::net::UnixStream::connect(format!("{dir}/{}", ipc::SOCKET_NAME)).ok()?;
    stream.write_all(ToDaemon::Devices.encode().as_bytes()).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(4))).ok()?;
    let mut lines = ipc::Lines::default();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).ok().filter(|n| *n > 0)?;
        for line in lines.push(&chunk[..n]).ok()? {
            if let Some(FromDaemon::Devices(list)) = ipc::parse_from_daemon(&line) {
                return Some(list);
            }
        }
    }
}

/// Searches by itself: listens on the Wi-Fi address while announcing, declines anything offered.
async fn devices_by_search(identity: &Identity) -> Result<Vec<Device>, String> {
    let interface = net::interface_name()?;
    let (address, netmask) = net::interface_address(&interface).ok_or("no-wifi")?;
    let (server_tx, mut server_rx) = mpsc::channel(16);
    let (discovery_tx, mut discovery_rx) = mpsc::channel(16);
    let net = Net::start(identity, address, netmask, server_tx, discovery_tx).await?;
    let discovery = net.discovery.clone();
    let own = identity.fingerprint.clone();
    let answering = tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(event) = server_rx.recv() => match event {
                    ServerEventV2::Register { ip, info } if info.fingerprint != own && policy::same_network(ip.ip, address, netmask) => {
                        let device = localsend::discovery::DiscoveredDevice {
                            alias: info.alias,
                            version: info.version,
                            device_model: info.device_model,
                            device_type: info.device_type,
                            fingerprint: info.fingerprint,
                            channel: localsend::discovery::DeviceChannel::Http(localsend::discovery::HttpChannel { host: ip.to_string(), port: info.port, protocol: info.protocol }),
                            download: info.download,
                        };
                        discovery.observe_inbound(device).await;
                    }
                    ServerEventV2::PrepareUpload { decision_tx, .. } => {
                        let _ = decision_tx.send(PrepareUploadDecisionV2::Decline);
                    }
                    _ => {}
                },
                Some(_) = discovery_rx.recv() => {}
                else => break,
            }
        }
    });
    let staged = net.discovery.discover_staged(Vec::new(), vec![address], net::PORT, ProtocolType::Https, Duration::from_secs(2));
    let _ = tokio::time::timeout(Duration::from_secs(8), staged).await;
    let list = net
        .discovery
        .devices()
        .iter()
        .filter_map(|d| {
            let http = d.get_best_channel()?.http()?;
            Some(Device {
                alias: ipc::tidy(&d.device.alias, 120),
                fingerprint: d.device.fingerprint.clone(),
                host: http.host.clone(),
                port: http.port,
                https: http.protocol == ProtocolType::Https,
            })
        })
        .collect();
    answering.abort();
    net.stop().await;
    Ok(list)
}

fn choose(lang: Lang, devices: &[Device], to: Option<&str>) -> Result<Option<Device>, String> {
    if let Some(wanted) = to {
        let found = devices.iter().find(|d| d.alias.eq_ignore_ascii_case(wanted) || d.alias == wanted || d.host == wanted);
        return found.cloned().map(Some).ok_or_else(|| format!("{}: {wanted}", say(lang, Say::NoSuchDevice)));
    }
    for (i, device) in devices.iter().enumerate() {
        println!("  {}  {}  ({})", i + 1, device.alias, device.host);
    }
    print!("{}", say(lang, Say::Choose));
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(|e| e.to_string())?;
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }
    let index = line.parse::<usize>().ok().filter(|n| (1..=devices.len()).contains(n)).ok_or_else(|| say(lang, Say::NoSuchDevice).to_string())?;
    Ok(Some(devices[index - 1].clone()))
}

/// Feeds an already open file to the upload, chunk by chunk.
fn content_of(file: std::fs::File, cancel: CancellationToken) -> FileContent {
    let (tx, rx) = mpsc::channel::<Bytes>(4);
    tokio::task::spawn_blocking(move || {
        let mut file = file;
        loop {
            let mut chunk = vec![0u8; 256 * 1024];
            match file.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    chunk.truncate(n);
                    if tx.blocking_send(Bytes::from(chunk)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    // A short upload must not pass for the whole file.
                    eprintln!("shabnam-send: {e}");
                    cancel.cancel();
                    break;
                }
            }
        }
    });
    FileContent::Stream(rx)
}

async fn send(lang: Lang, identity: Identity, to: Option<String>, files: Vec<Outgoing>) -> Result<(), String> {
    println!("{}", say(lang, Say::Searching));
    let devices = match tokio::task::spawn_blocking(devices_from_daemon).await.ok().flatten() {
        Some(list) => list,
        None => match devices_by_search(&identity).await {
            Ok(list) => list,
            Err(e) if e == "no-wifi" => return Err(say(lang, Say::NoWifi).into()),
            Err(e) => return Err(e),
        },
    };
    if devices.is_empty() {
        return Err(say(lang, Say::NoDevices).into());
    }
    let Some(device) = choose(lang, &devices, to.as_deref())? else {
        println!("{}", say(lang, Say::Cancelled));
        return Ok(());
    };
    let protocol = if device.https { ProtocolType::Https } else { ProtocolType::Http };
    // The certificate the device showed when it was found is the only one accepted now.
    let pinned = device.https.then(|| device.fingerprint.clone());
    let client = LsHttpClient::new(&identity.key_pem, &identity.cert_pem, LsHttpClientVersion::V2, pinned, None).map_err(|e| e.to_string())?;

    let mut dtos = HashMap::new();
    let mut sources = HashMap::new();
    for (i, file) in files.into_iter().enumerate() {
        let id = format!("f{i}");
        dtos.insert(id.clone(), FileDto { id: id.clone(), file_name: file.name.clone(), size: file.size, file_type: file_type(&file.name).to_string(), sha256: None, preview: None, metadata: None });
        sources.insert(id, file);
    }
    let cancel = CancellationToken::new();
    tokio::spawn({
        let cancel = cancel.clone();
        async move {
            let _ = tokio::signal::ctrl_c().await;
            cancel.cancel();
        }
    });
    let info = RegisterDto {
        alias: identity.alias.clone(),
        version: PROTOCOL_VERSION_V2.to_string(),
        device_model: Some("Shabnam".into()),
        device_type: Some(DeviceType::Desktop),
        token: identity.fingerprint.clone(),
        port: net::PORT,
        protocol: ProtocolType::Https,
        has_web_interface: false,
    };
    println!("{} → {}  {}", identity.alias, device.alias, say(lang, Say::Waiting));
    let prepared = client.prepare_upload(protocol, &device.host, device.port, None, PrepareUploadRequestDto { info, files: dtos.clone() }, None, cancel.clone()).await;
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(ClientError::Cancelled) => return Err(say(lang, Say::Cancelled).into()),
        Err(ClientError::StatusCode(e)) => {
            return Err(match e.status {
                401 => say(lang, Say::PinNeeded).to_string(),
                403 => say(lang, Say::Declined).to_string(),
                409 => say(lang, Say::Busy).to_string(),
                status => format!("{} (HTTP {status})", say(lang, Say::Failed)),
            })
        }
        Err(e) => return Err(format!("{}: {e}", say(lang, Say::Failed))),
    };
    let Some(response) = prepared.response.filter(|r| !r.session_id.is_empty() && !r.files.is_empty()) else {
        return Err(say(lang, Say::Declined).into());
    };
    let total: u64 = response.files.keys().filter_map(|id| dtos.get(id)).map(|f| f.size).sum();
    let sent = Arc::new(AtomicU64::new(0));
    let mut ids: Vec<&String> = response.files.keys().filter(|id| dtos.contains_key(*id)).collect();
    ids.sort();
    let mut base = 0u64;
    let mut count = 0;
    for id in ids {
        let Some(source) = sources.remove(id) else { continue };
        let (sent_now, name, size) = (sent.clone(), source.name.clone(), source.size);
        let progress = move |bytes: u64| {
            let now = base + bytes;
            // One line per 5 %: enough to see it move, readable in a log.
            let step = (total / 20).max(1);
            if now / step != sent_now.swap(now, Ordering::Relaxed) / step {
                eprint!("\r{:3}%", (now * 100).checked_div(total).unwrap_or(100).min(100));
            }
        };
        let content = content_of(source.file, cancel.clone());
        let result = client.upload(protocol, &device.host, device.port, None, &response.session_id, id, &response.files[id], content, progress, cancel.clone()).await;
        if let Err(e) = result {
            eprintln!();
            let _ = client.cancel(protocol, &device.host, device.port, &response.session_id).await;
            return Err(match e {
                ClientError::Cancelled => say(lang, Say::Cancelled).to_string(),
                e => format!("{} {name}: {e}", say(lang, Say::Failed)),
            });
        }
        base += size;
        count += 1;
    }
    eprintln!("\r100%");
    println!("{} {count} → {}", say(lang, Say::Sent), device.alias);
    Ok(())
}
