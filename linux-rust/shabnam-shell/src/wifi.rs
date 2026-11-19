//! Wi-Fi, through `wpa_cli -i wlan0` and nothing else (wpa_supplicant runs with update_config=1,
//! control socket /run/wpa_supplicant). The parsing and checking functions are pure; the talking
//! happens on a worker thread so the panel never waits for a scan or a connection attempt.
//! Depends on std only, so `rustc --test src/wifi.rs` checks it on any machine.
//!
//! Network names and passwords reach wpa_cli as separate arguments of a directly started process
//! (no shell). They are never printed, logged or written to a file here; wpa_supplicant's own
//! `save_config` is the only thing that stores them.

use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const IFACE: &str = "wlan0";
/// How long a connection attempt may take before it counts as failed.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const ADDRESS_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Security {
    Open,
    /// WPA/WPA2 personal (also mixed WPA2/WPA3 networks, which accept the same passphrase).
    Psk,
    /// WPA3 personal only.
    Sae,
    /// Enterprise (802.1X), WEP, OWE: not offered by the panel.
    Unsupported,
}

/// One network seen in a scan (the strongest access point of that name).
#[derive(Clone, PartialEq, Debug)]
pub struct Net {
    pub ssid: Vec<u8>,
    /// dBm, e.g. -65.
    pub signal: i32,
    pub security: Security,
}

/// One network wpa_supplicant has a configuration for.
#[derive(Clone, PartialEq, Debug)]
pub struct Known {
    pub id: u32,
    pub ssid: Vec<u8>,
    pub current: bool,
    /// wpa_supplicant gave up on it for now (what a wrong password leads to).
    pub temp_disabled: bool,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Status {
    /// wpa_state: COMPLETED, SCANNING, 4WAY_HANDSHAKE, DISCONNECTED ...
    pub state: String,
    pub ssid: Option<Vec<u8>>,
    pub id: Option<u32>,
    pub ip: Option<String>,
}

impl Status {
    pub fn connected(&self) -> bool {
        self.state == "COMPLETED"
    }
}

/// Undoes the escaping wpa_supplicant applies to network names in its text output
/// (`\\`, `\"`, `\n`, `\r`, `\t`, `\e`, `\xHH`); the result is the raw name, up to 32 bytes.
pub fn decode_ssid(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 >= bytes.len() {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        i += 1;
        match bytes[i] {
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'e' => out.push(0x1b),
            b'x' => {
                let digits: Vec<u8> = bytes[i + 1..].iter().take(2).copied().take_while(u8::is_ascii_hexdigit).collect();
                let value = std::str::from_utf8(&digits).ok().and_then(|d| u8::from_str_radix(d, 16).ok());
                match value {
                    Some(v) => {
                        out.push(v);
                        i += digits.len();
                    }
                    None => out.extend(b"\\x"),
                }
            }
            other => out.push(other), // `\\` and `\"`
        }
        i += 1;
    }
    out
}

/// The name as wpa_supplicant takes it without any quoting: hexadecimal digits.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A name for the screen: invalid UTF-8 and control characters are replaced.
pub fn display_ssid(ssid: &[u8]) -> String {
    String::from_utf8_lossy(ssid).chars().map(|c| if c.is_control() { '\u{fffd}' } else { c }).collect()
}

pub fn security_of(flags: &str) -> Security {
    if flags.contains("PSK") {
        Security::Psk
    } else if flags.contains("EAP") || flags.contains("WEP") || flags.contains("OWE") {
        Security::Unsupported
    } else if flags.contains("SAE") {
        Security::Sae
    } else {
        Security::Open
    }
}

/// `scan_results`: "bssid / frequency / signal level / flags / ssid", tab separated.
/// Hidden networks (empty name) are left out; of several access points with one name the
/// strongest stays. Strongest first.
pub fn parse_scan(text: &str) -> Vec<Net> {
    let mut nets: Vec<Net> = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.splitn(5, '\t').collect();
        if fields.len() < 5 {
            continue;
        }
        let Ok(signal) = fields[2].trim().parse::<i32>() else { continue };
        let ssid = decode_ssid(fields[4]);
        if !valid_ssid(&ssid) || ssid.iter().all(|b| *b == 0) {
            continue;
        }
        let net = Net { ssid, signal, security: security_of(fields[3]) };
        match nets.iter_mut().find(|n| n.ssid == net.ssid) {
            Some(seen) if seen.signal >= net.signal => {}
            Some(seen) => *seen = net,
            None => nets.push(net),
        }
    }
    nets.sort_by(|a, b| b.signal.cmp(&a.signal).then_with(|| a.ssid.cmp(&b.ssid)));
    nets
}

/// `list_networks`: "network id / ssid / bssid / flags", tab separated.
pub fn parse_networks(text: &str) -> Vec<Known> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let id = fields.first()?.trim().parse::<u32>().ok()?;
            let flags = fields.get(3).copied().unwrap_or("");
            Some(Known {
                id,
                ssid: decode_ssid(fields.get(1)?),
                current: flags.contains("[CURRENT]"),
                temp_disabled: flags.contains("[TEMP-DISABLED]"),
            })
        })
        .collect()
}

/// `status`: key=value lines.
pub fn parse_status(text: &str) -> Status {
    let mut status = Status::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        match key {
            "wpa_state" => status.state = value.trim().to_string(),
            "ssid" => status.ssid = Some(decode_ssid(value)),
            "id" => status.id = value.trim().parse().ok(),
            "ip_address" => status.ip = Some(value.trim().to_string()).filter(|v| !v.is_empty()),
            _ => {}
        }
    }
    status
}

/// 0..=4 bars for a signal level in dBm.
pub fn bars(signal: i32) -> u32 {
    match signal {
        s if s >= -55 => 4,
        s if s >= -67 => 3,
        s if s >= -78 => 2,
        _ => 1,
    }
}

pub fn valid_ssid(ssid: &[u8]) -> bool {
    !ssid.is_empty() && ssid.len() <= 32
}

/// A WPA passphrase: 8 to 63 bytes, no control characters, and no double quote (it is handed to
/// wpa_supplicant between double quotes, which must not be closed early).
pub fn valid_passphrase(password: &str) -> bool {
    (8..=63).contains(&password.len()) && !password.chars().any(|c| c == '"' || c.is_control())
}

// ---------- the worker ----------

/// What to connect to. (Deliberately without Debug: it can hold a password.)
pub enum Target {
    Known(u32),
    New { ssid: Vec<u8>, security: Security, password: Option<String> },
}

pub enum Request {
    /// Read status, known networks and the last scan results.
    Refresh,
    /// Start a scan and report results as they come in.
    Scan,
    Connect(Target),
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Snapshot {
    pub status: Status,
    pub known: Vec<Known>,
    pub nets: Vec<Net>,
}

#[derive(Debug)]
pub enum Event {
    Snapshot(Snapshot),
    ScanDone,
    /// The link is up; an address is being asked for.
    Associated,
    /// The connection is complete; the address if one arrived in time.
    Address(Option<String>),
    /// Wrong password, out of reach, timed out. The previous network was selected again.
    Failed,
}

/// Runs wpa_cli with the given words; what it printed, or None when it could not be run.
fn cli(args: &[&str]) -> Option<String> {
    let out = Command::new("wpa_cli").arg("-i").arg(IFACE).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// For the commands that answer OK or FAIL.
fn ok(args: &[&str]) -> bool {
    cli(args).is_some_and(|reply| reply.trim() == "OK")
}

fn status() -> Status {
    cli(&["status"]).map(|t| parse_status(&t)).unwrap_or_default()
}

fn networks() -> Vec<Known> {
    cli(&["list_networks"]).map(|t| parse_networks(&t)).unwrap_or_default()
}

fn snapshot() -> Snapshot {
    Snapshot { status: status(), known: networks(), nets: cli(&["scan_results"]).map(|t| parse_scan(&t)).unwrap_or_default() }
}

/// Creates the configuration for a network not connected to before; its id, or None (and no
/// configuration left behind) when wpa_supplicant refuses any part of it.
fn add_network(ssid: &[u8], security: Security, password: Option<&str>) -> Option<u32> {
    if !valid_ssid(ssid) || security == Security::Unsupported {
        return None;
    }
    let password = match (security, password) {
        (Security::Open, _) => None,
        (_, Some(p)) if valid_passphrase(p) => Some(format!("\"{p}\"")),
        _ => return None,
    };
    let id: u32 = cli(&["add_network"])?.lines().last()?.trim().parse().ok()?;
    let n = id.to_string();
    let set = |name: &str, value: &str| ok(&["set_network", &n, name, value]);
    let mut good = set("ssid", &hex(ssid));
    good = good
        && match (security, &password) {
            (Security::Open, _) => set("key_mgmt", "NONE"),
            (Security::Psk, Some(p)) => set("psk", p),
            (Security::Sae, Some(p)) => set("key_mgmt", "SAE") && set("ieee80211w", "2") && set("sae_password", p),
            _ => false,
        };
    good = good && ok(&["enable_network", &n]);
    if !good {
        let _ = ok(&["remove_network", &n]);
        return None;
    }
    Some(id)
}

/// Waits until network `id` is fully connected. False on a wrong key (wpa_supplicant then marks
/// the network TEMP-DISABLED) or when the time is up.
fn wait_connected(id: u32, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(500));
        let now = status();
        if now.connected() && now.id == Some(id) {
            return true;
        }
        if networks().iter().any(|k| k.id == id && k.temp_disabled) {
            return false;
        }
    }
    false
}

fn connect(target: Target, notify: &dyn Fn(Event)) {
    let before = status();
    let previous = before.id;
    let (id, added) = match target {
        Target::Known(id) => (id, false),
        Target::New { ssid, security, password } => match add_network(&ssid, security, password.as_deref()) {
            Some(id) => (id, true),
            None => {
                notify(Event::Failed);
                return;
            }
        },
    };
    if !added && before.connected() && previous == Some(id) {
        notify(Event::Address(before.ip));
        return;
    }
    let n = id.to_string();
    if ok(&["select_network", &n]) && wait_connected(id, CONNECT_TIMEOUT) {
        // select_network disabled every other network; undo that before the configuration is
        // saved, so the other known networks stay usable.
        let _ = ok(&["enable_network", "all"]);
        if added {
            let _ = ok(&["save_config"]);
        }
        notify(Event::Associated);
        notify(Event::Snapshot(snapshot()));
        // A different network: ask the running dhcpcd for a fresh lease.
        let _ = Command::new("dhcpcd").args(["-n", IFACE]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
        let started = Instant::now();
        let mut address = None;
        while started.elapsed() < ADDRESS_TIMEOUT {
            std::thread::sleep(Duration::from_millis(500));
            // The old network's address can linger for a moment; give the rebind time to replace it.
            if started.elapsed() >= Duration::from_secs(2) {
                address = status().ip;
                if address.is_some() {
                    break;
                }
            }
        }
        notify(Event::Address(address));
    } else {
        if added {
            let _ = ok(&["remove_network", &n]);
        }
        if let Some(previous) = previous.filter(|p| *p != id) {
            let p = previous.to_string();
            if ok(&["select_network", &p]) {
                wait_connected(previous, ADDRESS_TIMEOUT);
            }
        }
        let _ = ok(&["enable_network", "all"]);
        notify(Event::Failed);
    }
    notify(Event::Snapshot(snapshot()));
}

/// Starts the worker thread. Requests go in through the returned sender; `notify` is called on
/// the worker thread with what happened.
pub fn start(notify: impl Fn(Event) + Send + 'static) -> mpsc::Sender<Request> {
    let (sender, requests) = mpsc::channel::<Request>();
    std::thread::spawn(move || {
        for request in requests {
            match request {
                Request::Refresh => notify(Event::Snapshot(snapshot())),
                Request::Scan => {
                    let _ = cli(&["scan"]);
                    for _ in 0..3 {
                        std::thread::sleep(Duration::from_millis(1500));
                        notify(Event::Snapshot(snapshot()));
                    }
                    notify(Event::ScanDone);
                }
                Request::Connect(target) => connect(target, &notify),
            }
        }
    });
    sender
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssid_text() {
        assert_eq!(decode_ssid("Home AP"), b"Home AP");
        assert_eq!(decode_ssid("a\\\"b\\\\c"), b"a\"b\\c");
        assert_eq!(decode_ssid("\\xe5\\xae\\xb6"), "家".as_bytes());
        assert_eq!(decode_ssid("tab\\there\\n"), b"tab\there\n");
        assert_eq!(decode_ssid("odd\\xzz\\"), b"odd\\xzz\\");
        assert_eq!(hex(b"a\"b"), "612262");
        assert_eq!(hex("家".as_bytes()), "e5aeb6");
        assert_eq!(display_ssid(b"ok\x07\xff"), "ok\u{fffd}\u{fffd}");
    }

    #[test]
    fn scan() {
        let text = "bssid / frequency / signal level / flags / ssid\n\
                    02:00:00:00:00:a2\t2412\t-65\t[WPA2-PSK+SAE-CCMP][SAE-H2E][ESS]\tHome AP\n\
                    02:00:00:00:00:a3\t5180\t-50\t[WPA2-PSK+SAE-CCMP][SAE-H2E][ESS]\tHome AP\n\
                    00:11:22:33:44:55\t2437\t-80\t[ESS]\tCafe\\tFree\n\
                    00:11:22:33:44:56\t2437\t-70\t[WPA2-EAP-CCMP][ESS]\teduroam\n\
                    00:11:22:33:44:57\t2437\t-71\t[WPA2-SAE-CCMP][SAE-H2E][ESS]\tw3\n\
                    00:11:22:33:44:58\t2437\t-40\t[WPA2-PSK-CCMP][ESS]\t\n\
                    00:11:22:33:44:59\t2437\t-41\t[WEP][ESS]\told\n\
                    garbage line\n";
        let nets = parse_scan(text);
        let names: Vec<String> = nets.iter().map(|n| display_ssid(&n.ssid)).collect();
        assert_eq!(names, ["old", "Home AP", "eduroam", "w3", "Cafe\u{fffd}Free"]);
        assert_eq!(nets[1].signal, -50);
        assert_eq!(nets[0].security, Security::Unsupported);
        assert_eq!(nets[1].security, Security::Psk);
        assert_eq!(nets[2].security, Security::Unsupported);
        assert_eq!(nets[3].security, Security::Sae);
        assert_eq!(nets[4].security, Security::Open);
        assert_eq!((bars(-50), bars(-65), bars(-75), bars(-90)), (4, 3, 2, 1));
    }

    #[test]
    fn networks_and_status() {
        let text = "network id / ssid / bssid / flags\n0\tHome AP\tany\t[CURRENT]\n1\thome\tany\t[DISABLED]\n2\tbad\tany\t[TEMP-DISABLED]\n3\tplain\tany\t\n";
        let known = parse_networks(text);
        assert_eq!(known.len(), 4);
        assert!(known[0].current && !known[0].temp_disabled && known[0].ssid == b"Home AP");
        assert!(!known[1].current && known[2].temp_disabled && known[3].id == 3);
        let status = parse_status("bssid=2a:20\nfreq=2412\nssid=Home AP\nid=0\nwpa_state=COMPLETED\nip_address=192.168.0.10\naddress=c8:fe\n");
        assert!(status.connected());
        assert_eq!((status.id, status.ip.as_deref(), status.ssid.as_deref()), (Some(0), Some("192.168.0.10"), Some(&b"Home AP"[..])));
        let off = parse_status("wpa_state=DISCONNECTED\naddress=c8:fe\n");
        assert!(!off.connected() && off.id.is_none() && off.ip.is_none() && off.ssid.is_none());
    }

    #[test]
    fn checks() {
        assert!(valid_passphrase("12345678"));
        assert!(valid_passphrase(&"x".repeat(63)));
        assert!(valid_passphrase("with space & 'single' \\ $(x) `y`"));
        assert!(!valid_passphrase("1234567"));
        assert!(!valid_passphrase(&"x".repeat(64)));
        assert!(!valid_passphrase("has\"quote1"));
        assert!(!valid_passphrase("line\nbreak12"));
        assert!(valid_passphrase("密码密")); // 9 bytes
        assert!(!valid_passphrase("密码")); // 6 bytes
        assert!(valid_ssid(b"a") && !valid_ssid(b"") && !valid_ssid(&[b'a'; 33]));
    }
}
