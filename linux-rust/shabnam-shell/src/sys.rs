//! The shell's contact with the system: clock, battery, network, backlight, volume, the niri
//! window list, the small files under ~/.config/shabnam, and starting programs.

use crate::apps::{self, App};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

fn read_trim(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Runs a short command and returns what it printed; None if it is missing or fails.
fn output_of(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

// ---------- clock ----------

pub struct Now {
    /// "14:32"
    pub time: String,
    /// "11月20日 周五"
    pub date_zh: String,
    /// "Fri, 20 Nov"
    pub date_en: String,
    /// Seconds left in the current minute.
    pub to_next_minute: u64,
}

pub fn now() -> Now {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    unsafe { libc::localtime_r(&t, &mut tm) };
    const ZH: [&str; 7] = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];
    const EN: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let (weekday, month) = (tm.tm_wday.clamp(0, 6) as usize, tm.tm_mon.clamp(0, 11) as usize);
    Now {
        time: format!("{:02}:{:02}", tm.tm_hour, tm.tm_min),
        date_zh: format!("{}月{}日 {}", month + 1, tm.tm_mday, ZH[weekday]),
        date_en: format!("{}, {} {}", EN[weekday], tm.tm_mday, MONTHS[month]),
        to_next_minute: (60 - tm.tm_sec.clamp(0, 59)) as u64,
    }
}

// ---------- battery ----------

#[derive(Clone, PartialEq, Debug)]
pub struct Battery {
    pub percent: u32,
    pub charging: bool,
    pub full: bool,
}

pub fn battery() -> Option<Battery> {
    let mut dirs = vec!["/sys/class/power_supply/rk-bat".to_string()];
    if let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") {
        dirs.extend(entries.flatten().map(|e| e.path().to_string_lossy().into_owned()));
    }
    dirs.iter().find_map(|dir| {
        let percent = read_trim(&format!("{dir}/capacity"))?.parse::<u32>().ok()?.min(100);
        let status = read_trim(&format!("{dir}/status")).unwrap_or_default();
        Some(Battery { percent, charging: status == "Charging", full: status == "Full" })
    })
}

// ---------- network ----------

/// The interface that carries the network right now: Wi-Fi if it is up, else any other real one.
pub fn network_interface() -> Option<String> {
    let up = |name: &str| read_trim(&format!("/sys/class/net/{name}/operstate")).as_deref() == Some("up");
    if up("wlan0") {
        return Some("wlan0".into());
    }
    let mut names: Vec<String> = std::fs::read_dir("/sys/class/net")
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "lo" && !n.starts_with("usb") && !n.starts_with("dummy") && !n.starts_with("sit") && !n.starts_with("ip6"))
        .collect();
    names.sort();
    names.into_iter().find(|n| up(n))
}

#[derive(Default, Clone)]
pub struct NetDetail {
    pub interface: Option<String>,
    pub ssid: Option<String>,
    pub address: Option<String>,
}

/// Asks `iw` and `ip` (only done when the quick panel opens).
pub fn network_detail() -> NetDetail {
    let interface = network_interface();
    let mut detail = NetDetail { interface: interface.clone(), ..Default::default() };
    let Some(interface) = interface else { return detail };
    if interface.starts_with("wl") {
        detail.ssid = output_of("iw", &["dev", &interface, "link"]).and_then(|text| {
            text.lines().find_map(|l| l.trim().strip_prefix("SSID: ").map(|s| s.to_string()))
        });
    }
    detail.address = output_of("ip", &["-4", "-o", "addr", "show", "dev", &interface]).and_then(|text| {
        let mut words = text.split_whitespace();
        words.find(|w| *w == "inet")?;
        words.next().map(|a| a.split('/').next().unwrap_or(a).to_string())
    });
    detail
}

// ---------- backlight ----------

const BACKLIGHT: &str = "/sys/class/backlight/backlight";
/// The slider never goes darker than this, so the screen cannot be turned unreadable.
pub const MIN_BRIGHTNESS: u32 = 5;

pub fn brightness() -> Option<u32> {
    let max = read_trim(&format!("{BACKLIGHT}/max_brightness"))?.parse::<u32>().ok()?.max(1);
    let current = read_trim(&format!("{BACKLIGHT}/brightness"))?.parse::<u32>().ok()?;
    Some(((current * 100 + max / 2) / max).min(100))
}

pub fn set_brightness(percent: u32) {
    let percent = percent.clamp(MIN_BRIGHTNESS, 100);
    let Some(max) = read_trim(&format!("{BACKLIGHT}/max_brightness")).and_then(|m| m.parse::<u32>().ok()) else { return };
    let _ = std::fs::write(format!("{BACKLIGHT}/brightness"), format!("{}", (percent * max / 100).max(1)));
}

// ---------- volume (the shabnam-volume script) ----------

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Volume {
    pub percent: u32,
    pub muted: bool,
}

/// None when the volume script is not installed (the quick panel then shows no volume row).
pub fn volume() -> Option<Volume> {
    if !apps::program_exists("shabnam-volume", &path_var()) {
        return None;
    }
    // The script's own state file tells level and mute apart; `get` alone prints 0 while muted.
    if let Ok(text) = std::fs::read_to_string("/var/lib/shabnam/volume") {
        let field = |name: &str| text.lines().find_map(|l| l.trim().strip_prefix(name)?.parse::<u32>().ok());
        if let Some(percent) = field("vol=") {
            return Some(Volume { percent: percent.min(100), muted: field("muted=") == Some(1) });
        }
    }
    let percent = output_of("shabnam-volume", &["get"])?.trim().parse::<u32>().ok()?;
    Some(Volume { percent: percent.min(100), muted: false })
}

pub fn set_volume(percent: u32) {
    let _ = output_of("shabnam-volume", &["set", &percent.min(100).to_string()]);
}

pub fn toggle_mute() {
    let _ = output_of("shabnam-volume", &["mute"]);
}

/// The level the volume command reports right now (0 while muted or when sound is not running).
/// Starts a process: call it off the event loop.
pub fn volume_now() -> Option<u32> {
    Some(output_of("shabnam-volume", &["get"])?.trim().parse::<u32>().ok()?.min(100))
}

/// The backlight level as the brightness command reports it, else as read from the kernel.
/// Starts a process: call it off the event loop.
pub fn brightness_now() -> Option<u32> {
    output_of("shabnam-brightness", &["get"]).and_then(|t| t.trim().parse::<u32>().ok()).map(|p| p.min(100)).or_else(brightness)
}

// ---------- niri ----------

#[derive(Clone, PartialEq, Debug)]
pub struct Window {
    pub id: u64,
    pub app_id: String,
    pub title: String,
    pub focused: bool,
    /// When it last had the focus (compositor clock, nanoseconds).
    pub focus_time: u128,
}

fn niri_socket() -> Option<UnixStream> {
    let path = std::env::var_os("NIRI_SOCKET")?;
    let stream = UnixStream::connect(path).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
    stream.set_write_timeout(Some(Duration::from_millis(500))).ok()?;
    Some(stream)
}

/// One request, one JSON reply line.
fn niri_request(request: &str) -> Option<serde_json::Value> {
    let mut stream = niri_socket()?;
    stream.write_all(request.as_bytes()).ok()?;
    stream.write_all(b"\n").ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    serde_json::from_str::<serde_json::Value>(&line).ok()?.get("Ok").cloned()
}

/// Text a program chose for itself (app id, title), made fit for showing: no control
/// characters, and no longer than `most` characters.
fn tidy(text: &str, most: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(most).collect()
}

/// The open windows, oldest first; None when niri cannot be asked.
pub fn windows() -> Option<Vec<Window>> {
    let reply = niri_request("\"Windows\"")?;
    let mut list: Vec<Window> = reply
        .get("Windows")?
        .as_array()?
        .iter()
        .filter_map(|w| {
            let stamp = w.get("focus_timestamp");
            let part = |name: &str| stamp.and_then(|s| s.get(name)).and_then(|v| v.as_u64()).unwrap_or(0) as u128;
            Some(Window {
                id: w.get("id")?.as_u64()?,
                app_id: tidy(w.get("app_id").and_then(|v| v.as_str()).unwrap_or(""), 128),
                title: tidy(w.get("title").and_then(|v| v.as_str()).unwrap_or(""), 200),
                focused: w.get("is_focused").and_then(|v| v.as_bool()).unwrap_or(false),
                focus_time: part("secs") * 1_000_000_000 + part("nanos"),
            })
        })
        .collect();
    list.sort_by_key(|w| w.id);
    Some(list)
}

pub fn focus_window(id: u64) {
    let _ = niri_request(&format!("{{\"Action\":{{\"FocusWindow\":{{\"id\":{id}}}}}}}"));
}

pub fn close_window(id: u64) {
    let _ = niri_request(&format!("{{\"Action\":{{\"CloseWindow\":{{\"id\":{id}}}}}}}"));
}

/// A connection on which niri keeps reporting changes, one JSON line each.
pub fn niri_events() -> Option<UnixStream> {
    let mut stream = niri_socket()?;
    stream.write_all(b"\"EventStream\"\n").ok()?;
    stream.set_nonblocking(true).ok()?;
    Some(stream)
}

// ---------- ~/.config/shabnam ----------

fn config_path(name: &str) -> String {
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty()).unwrap_or_else(|| "/root".into());
    format!("{home}/.config/shabnam/{name}")
}

/// A file of one entry per line; None when it does not exist.
pub fn read_list(name: &str) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(config_path(name)).ok()?;
    Some(text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with('#')).collect())
}

pub fn write_list(name: &str, entries: &[String]) {
    let path = config_path(name);
    if let Some(dir) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let temporary = format!("{path}.tmp");
    if std::fs::write(&temporary, entries.join("\n") + "\n").is_ok() {
        let _ = std::fs::rename(&temporary, &path);
    }
}

// ---------- starting programs ----------

/// The session's private directory: $XDG_RUNTIME_DIR, or /run/user0 on this machine. Never /tmp.
pub fn runtime_dir() -> String {
    std::env::var("XDG_RUNTIME_DIR").ok().filter(|d| d.starts_with('/')).unwrap_or_else(|| "/run/user0".into())
}

/// The control socket lives in the runtime directory, so nobody else may be able to put files
/// there: it must be a directory (not a link to one), owned by this user, and writable by this
/// user only. (Group read/search is tolerated: on this machine /run/user0 is root:700 mode 750 so
/// that the unprivileged browser user can reach the Wayland socket. The control socket itself is
/// created with mode 600, so that group cannot talk to it.)
pub fn check_runtime_dir() -> Result<String, String> {
    use std::os::unix::fs::MetadataExt;
    let dir = runtime_dir();
    let meta = std::fs::symlink_metadata(&dir).map_err(|e| format!("runtime directory {dir}: {e}"))?;
    let uid = unsafe { libc::getuid() };
    if !meta.is_dir() {
        return Err(format!("runtime directory {dir} is not a directory"));
    }
    if meta.uid() != uid {
        return Err(format!("runtime directory {dir} belongs to uid {}, not to this user ({uid})", meta.uid()));
    }
    if meta.mode() & 0o022 != 0 {
        return Err(format!("runtime directory {dir} is writable by other users (mode {:o})", meta.mode() & 0o777));
    }
    Ok(dir)
}

fn path_var() -> String {
    std::env::var("PATH").unwrap_or_default()
}

/// Starts a program in its own session with no ties to the shell. It forks twice so the program
/// becomes a child of init: the shell never has to collect its exit and leaves no zombies.
/// The program is started directly from its argument list; no shell reads the command.
fn spawn_detached(argv: &[String], dir: Option<&str>) -> std::io::Result<()> {
    let Some((program, args)) = argv.split_first() else {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "empty command"));
    };
    let mut cmd = Command::new(program);
    cmd.args(args);
    let home = std::env::var("HOME").ok().filter(|h| std::path::Path::new(h).is_dir());
    let dir = dir.filter(|d| std::path::Path::new(d).is_dir()).map(str::to_string).or(home).unwrap_or_else(|| "/".into());
    cmd.current_dir(dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            match libc::fork() {
                0 => Ok(()),
                pid if pid > 0 => libc::_exit(0),
                _ => Err(std::io::Error::last_os_error()),
            }
        });
    }
    cmd.spawn()?.wait().map(|_| ())
}

/// Starts a program given as an argument list.
pub fn spawn(argv: &[&str]) -> std::io::Result<()> {
    spawn_detached(&argv.iter().map(|a| a.to_string()).collect::<Vec<_>>(), None)
}

/// The terminal command line a Terminal=true application is put behind, given which programs
/// exist: foot first (software-drawn, so it works on this machine's GPU), then rio.
/// foot is told to use the application's id as the window's app id, so the shelf can tell a
/// file-manager window from an editor window although both are foot windows.
pub fn terminal_prefix(has: impl Fn(&str) -> bool, app_id: &str) -> Vec<String> {
    let plain = !app_id.is_empty() && app_id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if has("foot") {
        let mut words = vec!["foot".to_string()];
        if plain {
            words.push(format!("--app-id={app_id}"));
        }
        words.push("-e".into());
        words
    } else if has("rio") {
        vec!["rio".into(), "-e".into()]
    } else if has("wezterm") {
        vec!["wezterm".into(), "start".into(), "--".into()]
    } else if has("alacritty") {
        vec!["alacritty".into(), "-e".into()]
    } else {
        Vec::new()
    }
}

/// Starts an application, with `extra` appended to its arguments. A Terminal=true application
/// runs inside a terminal window (see `terminal_prefix`).
pub fn launch(app: &App, extra: &[String]) -> std::io::Result<()> {
    let has = |program: &str| apps::program_exists(program, &path_var());
    let mut argv: Vec<String> = Vec::new();
    if app.terminal {
        let prefix = terminal_prefix(&has, &app.id);
        argv.extend(prefix.iter().map(|p| p.to_string()));
        // weston-terminal takes a program but no arguments for it, so it only serves bare commands.
        if prefix.is_empty() && app.argv.len() == 1 && extra.is_empty() && has("weston-terminal") {
            return spawn_detached(&["weston-terminal".to_string(), format!("--shell={}", app.argv[0])], app.dir.as_deref());
        }
    }
    argv.extend(app.argv.iter().cloned());
    argv.extend(extra.iter().cloned());
    spawn_detached(&argv, app.dir.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminals() {
        let all = |_: &str| true;
        assert_eq!(terminal_prefix(all, "yazi"), ["foot", "--app-id=yazi", "-e"]);
        assert_eq!(terminal_prefix(all, "odd id"), ["foot", "-e"]);
        assert_eq!(terminal_prefix(|p: &str| p == "rio", "Helix"), ["rio", "-e"]);
        assert!(terminal_prefix(|_: &str| false, "x").is_empty());
    }
}
