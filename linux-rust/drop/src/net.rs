//! What the receiver and `shabnam-send` share: this device's identity (a self-signed
//! certificate, as the protocol wants), the Wi-Fi interface's address, and starting the protocol
//! core's HTTPS server and discovery on that one address.

use crate::policy;
use localsend::discovery::{DeviceIdentity, DiscoveryConfig, DiscoveryEvent, DiscoveryHandle, DEFAULT_DISCOVERY_TIMEOUT};
use localsend::http::dto_v2::RegisterDtoV2;
use localsend::http::server::v2::ServerEventV2;
use localsend::http::server::web::WebConfig;
use localsend::http::server::{start_with_bind, ServerConfigV2, ServerHandle, TlsConfig};
use localsend::http::state::ClientInfo;
use localsend::model::discovery::{DeviceType, ProtocolType, PROTOCOL_VERSION_V2};
use localsend::multicast::{MulticastDevice, DEFAULT_MULTICAST_GROUP, DEFAULT_PORT};
use localsend::util::interface::InterfaceFilter;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

pub const PORT: u16 = DEFAULT_PORT;
const MODEL: &str = "Shabnam";

/// The interface to use: wlan0 unless SHABNAM_DROP_IFACE names another (never the Mac cable).
pub fn interface_name() -> Result<String, String> {
    let name = std::env::var("SHABNAM_DROP_IFACE").ok().filter(|n| !n.is_empty()).unwrap_or_else(|| "wlan0".into());
    if policy::interface_allowed(&name) {
        Ok(name)
    } else {
        Err(format!("interface {name} is not allowed"))
    }
}

/// The first IPv4 address of the interface with its netmask; None while it has none.
pub fn interface_address(name: &str) -> Option<(Ipv4Addr, Ipv4Addr)> {
    if_addrs::get_if_addrs().ok()?.into_iter().find_map(|interface| match interface.addr {
        if_addrs::IfAddr::V4(v4) if interface.name == name && !v4.ip.is_loopback() => Some((v4.ip, v4.netmask)),
        _ => None,
    })
}

pub struct Identity {
    pub alias: String,
    pub cert_pem: String,
    pub key_pem: String,
    pub fingerprint: String,
}

/// $HOME/.config/shabnam-drop, created for this user only (the certificate's key lives here; HOME
/// is the daemon user's own state directory, which no other user can enter).
pub fn config_dir() -> Result<PathBuf, String> {
    use std::os::unix::fs::DirBuilderExt;
    let home = std::env::var("HOME").ok().filter(|h| h.starts_with('/')).ok_or("HOME is not set")?;
    let dir = Path::new(&home).join(".config/shabnam-drop");
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

impl Identity {
    /// Reads identity.pem (certificate and key), or makes one on first use. The file is kept so
    /// that other devices keep recognising this laptop.
    pub fn load_or_generate(dir: &Path, alias: &str) -> Result<Identity, String> {
        let path = dir.join("identity.pem");
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let blocks = pem::parse_many(&text).map_err(|e| format!("{}: {e}", path.display()))?;
                let cert = blocks.iter().find(|b| b.tag() == "CERTIFICATE").ok_or("identity.pem has no certificate")?;
                let key = blocks.iter().find(|b| b.tag().ends_with("PRIVATE KEY")).ok_or("identity.pem has no key")?;
                Ok(Identity {
                    alias: alias.to_string(),
                    fingerprint: localsend::crypto::cert::fingerprint_from_cert_der(cert.contents()),
                    cert_pem: pem::encode(cert),
                    key_pem: pem::encode(key),
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                use std::io::Write;
                use std::os::unix::fs::OpenOptionsExt;
                let cert = localsend::crypto::cert::generate_self_signed().map_err(|e| format!("cannot make a certificate: {e:#}"))?;
                let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                file.write_all(format!("{}{}", cert.certificate_pem, cert.private_key_pem).as_bytes()).map_err(|e| e.to_string())?;
                Ok(Identity { alias: alias.to_string(), cert_pem: cert.certificate_pem, key_pem: cert.private_key_pem, fingerprint: cert.fingerprint })
            }
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn register_dto(&self, port: u16) -> RegisterDtoV2 {
        RegisterDtoV2 {
            alias: self.alias.clone(),
            version: PROTOCOL_VERSION_V2.to_string(),
            device_model: Some(MODEL.to_string()),
            device_type: Some(DeviceType::Desktop),
            fingerprint: self.fingerprint.clone(),
            port,
            protocol: ProtocolType::Https,
            download: false,
        }
    }
}

/// The running HTTPS server and discovery, both tied to one address.
pub struct Net {
    pub address: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub server: Arc<ServerHandle>,
    pub discovery: Arc<DiscoveryHandle>,
    server_stop: oneshot::Sender<()>,
    discovery_stop: oneshot::Sender<()>,
}

impl Net {
    /// Listens on `address`:53317 (TCP, HTTPS with this device's certificate) and joins the
    /// discovery group on the interface that has `address` — nowhere else.
    pub async fn start(
        identity: &Identity,
        address: Ipv4Addr,
        netmask: Ipv4Addr,
        server_events: mpsc::Sender<ServerEventV2>,
        discovery_events: mpsc::Sender<DiscoveryEvent>,
    ) -> Result<Net, String> {
        let (server_stop, server_stop_rx) = oneshot::channel();
        let server = start_with_bind(
            Some(address),
            PORT,
            Some(TlsConfig { cert: identity.cert_pem.clone(), private_key: identity.key_pem.clone() }),
            ClientInfo {
                alias: identity.alias.clone(),
                version: PROTOCOL_VERSION_V2.to_string(),
                device_model: Some(MODEL.to_string()),
                device_type: Some(DeviceType::Desktop),
                token: identity.fingerprint.clone(),
            },
            None,
            Some(ServerConfigV2 { pin: None, verify_checksums: true, event_tx: server_events }),
            WebConfig::default(),
            server_stop_rx,
        )
        .await
        .map_err(|e| format!("cannot listen on {address}:{PORT}: {e:#}"))?;

        let (discovery_stop, discovery_stop_rx) = oneshot::channel();
        let discovery = localsend::discovery::start(
            DiscoveryConfig {
                group: DEFAULT_MULTICAST_GROUP,
                group_v6: None,
                port: PORT,
                // Only the interface that carries this address.
                interface_filter: InterfaceFilter { whitelist: Some(vec![address.to_string()]), blacklist: None },
                device: MulticastDevice {
                    alias: identity.alias.clone(),
                    version: PROTOCOL_VERSION_V2.to_string(),
                    device_model: Some(MODEL.to_string()),
                    device_type: Some(DeviceType::Desktop),
                    fingerprint: identity.fingerprint.clone(),
                    port: PORT,
                    protocol: ProtocolType::Https,
                    download: false,
                },
                identity: DeviceIdentity { cert_pem: identity.cert_pem.clone(), private_key_pem: identity.key_pem.clone() },
                timeout: DEFAULT_DISCOVERY_TIMEOUT,
                event_tx: Some(discovery_events),
            },
            discovery_stop_rx,
        )
        .await;
        if let Some(e) = discovery.multicast_error() {
            eprintln!("shabnam-drop: no multicast discovery: {e:#}");
        }
        Ok(Net { address, netmask, server: Arc::new(server), discovery: Arc::new(discovery), server_stop, discovery_stop })
    }

    /// Closes the ports and waits (briefly) until they are free again.
    pub async fn stop(self) {
        let _ = self.server_stop.send(());
        let _ = self.discovery_stop.send(());
        let wait = std::time::Duration::from_secs(1);
        let _ = tokio::time::timeout(wait, self.server.wait_stopped()).await;
        let _ = tokio::time::timeout(wait, self.discovery.wait_stopped()).await;
    }
}

/// (uid, gid, home) of a system user.
pub fn user(name: &str) -> Option<(u32, u32, String)> {
    let name = std::ffi::CString::new(name).ok()?;
    let entry = unsafe { libc::getpwnam(name.as_ptr()) };
    if entry.is_null() {
        return None;
    }
    unsafe { Some(((*entry).pw_uid, (*entry).pw_gid, std::ffi::CStr::from_ptr((*entry).pw_dir).to_string_lossy().into_owned())) }
}

/// The uid of the process at the other end of a unix socket, as the kernel recorded it when the
/// connection was made (it cannot be forged by the peer).
#[cfg(target_os = "linux")]
pub fn peer_uid(stream: &std::os::unix::net::UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let ok = unsafe { libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, &mut cred as *mut _ as *mut libc::c_void, &mut len) } == 0;
    (ok && len as usize == std::mem::size_of::<libc::ucred>()).then_some(cred.uid)
}

#[cfg(not(target_os = "linux"))]
pub fn peer_uid(_stream: &std::os::unix::net::UnixStream) -> Option<u32> {
    None
}

/// Connects to the running daemon — only if the other end really is the daemon's user.
pub fn connect_daemon() -> Option<std::os::unix::net::UnixStream> {
    let (daemon, _, _) = user(crate::ipc::DAEMON_USER)?;
    let stream = std::os::unix::net::UnixStream::connect(crate::ipc::socket_path()).ok()?;
    (daemon != 0 && peer_uid(&stream) == Some(daemon)).then_some(stream)
}

/// Free bytes on the file system holding `dir` (for unprivileged users).
pub fn free_space(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(path.as_ptr(), &mut stat) } == 0).then(|| stat.f_bavail as u64 * stat.f_frsize as u64)
}
