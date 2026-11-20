//! The messages between the 隔空传送 daemon (shabnam-dropd, its own user "shabnam-drop") and the desktop shell
//! (shabnam-shell, root), over a unix socket: one JSON object per line, a fixed set of messages.
//! And, further down, the messages of the second, narrower socket on which a program of the
//! desktop's unprivileged user (shabnam-send) hands over files to be sent ("submit").
//!
//! This one file is compiled into both programs (the shell includes it by path), so the two
//! sides cannot drift apart. It needs serde_json and nothing else.
//!
//! Neither side trusts the other's text: everything read here is cut to a fixed length and
//! stripped of control characters before it is handed on, and unknown or malformed lines are
//! dropped (`None`). The shell never sees anything from the network that did not pass through
//! `parse_from_daemon`.

use serde_json::{json, Value};

/// The user the daemon runs as. Nothing else runs as it — in particular not the browser, which
/// must be assumed hostile — so "the peer on the socket has this uid" means "it is the daemon".
/// Both the shell and `shabnam-send` check exactly that before they believe a word.
pub const DAEMON_USER: &str = "shabnam-drop";
/// The daemon's runtime directory: made by root, owned by the daemon's user, mode 0700.
pub const RUNTIME_DIR: &str = "/run/shabnam-drop";
/// Name of the socket inside it.
pub const SOCKET_NAME: &str = "shabnam-drop.sock";

pub fn socket_path() -> String {
    format!("{RUNTIME_DIR}/{SOCKET_NAME}")
}

/// The desktop's unprivileged user: the file manager, the browser and every other application
/// run as it. It is the only user (besides root) that may hand files to the daemon for sending —
/// and because the browser is among its programs, nothing it says is believed: it can offer
/// files it could read anyway, never choose where they go.
pub const DESKTOP_USER: &str = "shabnam";
/// Where that user reaches the daemon: a directory of the daemon's user with the desktop user's
/// group and mode 0750 (made by `shabnam-drop start`), holding the socket below.
pub const SUBMIT_DIR: &str = "/run/shabnam-drop-submit";
pub const SUBMIT_SOCKET: &str = "submit.sock";

pub fn submit_path() -> String {
    format!("{SUBMIT_DIR}/{SUBMIT_SOCKET}")
}

/// The shell's "wake" socket (root's directory, the desktop user's group, mode 0750). A
/// connection to it — nothing is read from it — asks the shell to start the daemon for sending
/// when it is not running; the daemon cannot be started by anybody but root.
pub const WAKE_DIR: &str = "/run/shabnam-send";
pub const WAKE_SOCKET: &str = "wake.sock";

pub fn wake_path() -> String {
    format!("{WAKE_DIR}/{WAKE_SOCKET}")
}
/// No line is longer than this; a peer that sends more is disconnected.
pub const MAX_LINE: usize = 8192;
const MAX_NAME: usize = 120;
/// How many file names of a send request are passed to the picker (the rest is a number).
pub const SHOWN_NAMES: usize = 8;
const MAX_TEXT: usize = 600;
const MAX_ID: usize = 64;
const MAX_DEVICES: usize = 32;
const MAX_PIN: usize = 32;

/// Why the daemon turned an offer down by itself.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Refusal {
    /// Larger than the limit for one transfer.
    TooBig,
    /// It would not leave enough free space on the disk.
    NoSpace,
    /// More files than the limit for one transfer.
    TooMany,
}

impl Refusal {
    fn word(self) -> &'static str {
        match self {
            Refusal::TooBig => "too-big",
            Refusal::NoSpace => "no-space",
            Refusal::TooMany => "too-many",
        }
    }
    fn from_word(word: &str) -> Option<Refusal> {
        [Refusal::TooBig, Refusal::NoSpace, Refusal::TooMany].into_iter().find(|r| r.word() == word)
    }
}

/// What kind of device the other side says it is (shown next to its name, nothing more).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DeviceKind {
    Mobile,
    Desktop,
    Web,
    Other,
}

impl DeviceKind {
    fn word(self) -> &'static str {
        match self {
            DeviceKind::Mobile => "mobile",
            DeviceKind::Desktop => "desktop",
            DeviceKind::Web => "web",
            DeviceKind::Other => "other",
        }
    }
    fn from_word(word: &str) -> DeviceKind {
        [DeviceKind::Mobile, DeviceKind::Desktop, DeviceKind::Web].into_iter().find(|k| k.word() == word).unwrap_or(DeviceKind::Other)
    }
}

/// A device that answered on the network: a possible target for sending.
#[derive(Clone, PartialEq, Debug)]
pub struct Device {
    pub alias: String,
    pub fingerprint: String,
    pub host: String,
    pub port: u16,
    pub https: bool,
    pub kind: DeviceKind,
}

impl Device {
    fn to_json(&self) -> Value {
        json!({"alias": self.alias, "fp": self.fingerprint, "host": self.host, "port": self.port, "https": self.https, "kind": self.kind.word()})
    }

    /// None for a device with a strange host or fingerprint: such a one is never shown or used.
    fn from_json(d: &Value) -> Option<Device> {
        let fingerprint = d.get("fp")?.as_str()?;
        let host = d.get("host")?.as_str()?;
        (plain(fingerprint, "") && plain(host, ".:%")).then_some(())?;
        Some(Device {
            alias: text_of(d, "alias", MAX_NAME)?,
            fingerprint: fingerprint.to_string(),
            host: host.to_string(),
            port: u16::try_from(number(d, "port")?).ok().filter(|p| *p != 0)?,
            https: d.get("https")?.as_bool()?,
            kind: DeviceKind::from_word(d.get("kind").and_then(|k| k.as_str()).unwrap_or("")),
        })
    }
}

/// Letters and digits (plus the characters in `extra`), not empty, at most 128 of them.
fn plain(text: &str, extra: &str) -> bool {
    !text.is_empty() && text.len() <= 128 && text.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
}

/// A fingerprint in a form a person can compare: the first 16 characters, in groups of four.
pub fn short_fingerprint(fingerprint: &str) -> String {
    let head: Vec<char> = fingerprint.chars().filter(char::is_ascii_alphanumeric).take(16).collect();
    head.chunks(4).map(|group| group.iter().collect::<String>()).collect::<Vec<_>>().join(" ")
}

/// How a send ended.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SendOutcome {
    /// Every file arrived.
    Sent,
    /// The other side said no.
    Declined,
    /// The other side is in another transfer.
    Busy,
    /// Stopped on this side (the owner, or the requesting program went away).
    Cancelled,
    /// The other side wants a PIN and none (or no right one) could be given.
    PinNeeded,
    /// No connection to the device.
    Unreachable,
    /// A file could not be read to its announced end.
    FileChanged,
    /// The other side never answered the request.
    NoAnswer,
    /// `shabnam-send --to`: no such device was found.
    NoSuchDevice,
    /// Anything else (it broke off half-way, the other side answered with an error).
    Failed,
}

impl SendOutcome {
    const ALL: [SendOutcome; 10] = [
        SendOutcome::Sent,
        SendOutcome::Declined,
        SendOutcome::Busy,
        SendOutcome::Cancelled,
        SendOutcome::PinNeeded,
        SendOutcome::Unreachable,
        SendOutcome::FileChanged,
        SendOutcome::NoAnswer,
        SendOutcome::NoSuchDevice,
        SendOutcome::Failed,
    ];
    fn word(self) -> &'static str {
        match self {
            SendOutcome::Sent => "sent",
            SendOutcome::Declined => "declined",
            SendOutcome::Busy => "busy",
            SendOutcome::Cancelled => "cancelled",
            SendOutcome::PinNeeded => "pin",
            SendOutcome::Unreachable => "unreachable",
            SendOutcome::FileChanged => "file-changed",
            SendOutcome::NoAnswer => "no-answer",
            SendOutcome::NoSuchDevice => "no-such-device",
            SendOutcome::Failed => "failed",
        }
    }
    fn from_word(word: &str) -> Option<SendOutcome> {
        SendOutcome::ALL.into_iter().find(|o| o.word() == word)
    }
}

/// Shell to daemon. Only root's connection is obeyed.
#[derive(Clone, PartialEq, Debug)]
pub enum ToDaemon {
    /// The owner's answer to the offer with this id.
    Answer { id: String, accept: bool },
    /// "Quick receive": accept offers without asking.
    Quick(bool),
    /// Stop the transfer that is running.
    Cancel { id: String },
    /// Ask for the devices seen on the network.
    Devices,
    /// Stop everything: the daemon closes its ports and exits.
    Quit,
    /// Switch receiving on or off. Off: every offer is declined without asking, and the daemon
    /// exits once it has nothing left to send.
    Receive(bool),
    /// The owner chose, in the shell's picker, where the files of send request `id` go: one of
    /// the devices the daemon listed for that request, named by all of what was shown —
    /// and `revision` says which set of files the owner was looking at when choosing. The daemon
    /// takes the choice only if that is still the set.
    SendTo { id: String, revision: u32, fingerprint: String, host: String, port: u16 },
    /// The owner has started to use the picker of request `id`: from now on nothing is added to it.
    SendTouch { id: String },
    /// The PIN the owner typed for the device that asked for one.
    SendPin { id: String, pin: String },
    /// Search the network again for the picker of request `id`.
    SendRefresh { id: String },
    /// Give up send request `id`, whatever stage it is in.
    SendCancel { id: String },
}

/// Daemon to shell.
#[derive(Clone, PartialEq, Debug)]
pub enum FromDaemon {
    /// Sent on connect and whenever something in it changes. `address` is None while the
    /// Wi-Fi interface has no address or nothing needs the network (then nothing is listening).
    /// `receiving` false: the daemon only runs to send, and declines whatever is offered to it.
    State { alias: String, address: Option<String>, quick: bool, receiving: bool },
    /// Somebody wants to send files; waits for `Answer`.
    Offer { id: String, from: String, count: u32, bytes: u64, name: String },
    /// The offer is no longer open (the sender gave up, or nobody answered in time).
    OfferGone { id: String },
    Progress { id: String, done: u64, total: u64 },
    /// The transfer ended. `name` is the saved name of the first file (empty if none was saved).
    Done { id: String, from: String, saved: u32, failed: u32, name: String },
    /// An offer the daemon declined by itself.
    Refused { from: String, why: Refusal },
    /// A text message (nothing is saved).
    Text { from: String, text: String },
    Devices(Vec<Device>),
    /// A program handed over files to send: show the picker. Sent again (same id, another
    /// `revision`) whenever the set of files changed, and when a choice was refused because it
    /// was made for an older set: the picker then starts over. `names` are the first
    /// `SHOWN_NAMES` of the `count` files; `by_root` says that all of them came from root's own
    /// command rather than from a program of the desktop user.
    SendRequest { id: String, revision: u32, count: u32, bytes: u64, names: Vec<String>, by_root: bool },
    /// The devices to choose from for request `id`, while its picker is up.
    SendDevices { id: String, searching: bool, list: Vec<Device> },
    /// The chosen device wants a PIN (`wrong`: the one given was not right).
    SendPinWanted { id: String, wrong: bool },
    /// `file` 0: waiting for the other side to accept; otherwise the file being sent, of `count`.
    SendProgress { id: String, to: String, file: u32, count: u32, done: u64, total: u64 },
    /// The request is over; `sent` files arrived.
    SendDone { id: String, to: String, outcome: SendOutcome, sent: u32 },
}

/// Text from the other side, made fit for showing: no control characters, at most `most`
/// characters, no leading or trailing blanks.
pub fn tidy(text: &str, most: usize) -> String {
    let cleaned: String = text.chars().map(|c| if c == '\n' || c == '\t' { ' ' } else { c }).filter(|c| !c.is_control()).collect();
    cleaned.trim().chars().take(most).collect()
}

fn text_of(value: &Value, key: &str, most: usize) -> Option<String> {
    Some(tidy(value.get(key)?.as_str()?, most))
}

/// An id is a session id the daemon made up: letters, digits and dashes only.
fn id_of(value: &Value) -> Option<String> {
    let id = value.get("id")?.as_str()?;
    (!id.is_empty() && id.len() <= MAX_ID && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).then(|| id.to_string())
}

fn number(value: &Value, key: &str) -> Option<u64> {
    value.get(key)?.as_u64()
}

fn object(line: &str) -> Option<(Value, String)> {
    if line.len() > MAX_LINE {
        return None;
    }
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    let kind = value.get("t")?.as_str()?.to_string();
    Some((value, kind))
}

impl ToDaemon {
    pub fn encode(&self) -> String {
        let value = match self {
            ToDaemon::Answer { id, accept } => json!({"t": "answer", "id": id, "accept": accept}),
            ToDaemon::Quick(on) => json!({"t": "quick", "on": on}),
            ToDaemon::Cancel { id } => json!({"t": "cancel", "id": id}),
            ToDaemon::Devices => json!({"t": "devices"}),
            ToDaemon::Quit => json!({"t": "quit"}),
            ToDaemon::Receive(on) => json!({"t": "receive", "on": on}),
            ToDaemon::SendTo { id, revision, fingerprint, host, port } => json!({"t": "send-to", "id": id, "rev": revision, "fp": fingerprint, "host": host, "port": port}),
            ToDaemon::SendTouch { id } => json!({"t": "send-touch", "id": id}),
            ToDaemon::SendPin { id, pin } => json!({"t": "send-pin", "id": id, "pin": pin}),
            ToDaemon::SendRefresh { id } => json!({"t": "send-refresh", "id": id}),
            ToDaemon::SendCancel { id } => json!({"t": "send-cancel", "id": id}),
        };
        format!("{value}\n")
    }
}

pub fn parse_to_daemon(line: &str) -> Option<ToDaemon> {
    let (value, kind) = object(line)?;
    Some(match kind.as_str() {
        "answer" => ToDaemon::Answer { id: id_of(&value)?, accept: value.get("accept")?.as_bool()? },
        "quick" => ToDaemon::Quick(value.get("on")?.as_bool()?),
        "cancel" => ToDaemon::Cancel { id: id_of(&value)? },
        "devices" => ToDaemon::Devices,
        "quit" => ToDaemon::Quit,
        "receive" => ToDaemon::Receive(value.get("on")?.as_bool()?),
        "send-to" => {
            let (fingerprint, host) = (value.get("fp")?.as_str()?, value.get("host")?.as_str()?);
            (plain(fingerprint, "") && plain(host, ".:%")).then_some(())?;
            ToDaemon::SendTo {
                id: id_of(&value)?,
                revision: u32::try_from(number(&value, "rev")?).ok()?,
                fingerprint: fingerprint.to_string(),
                host: host.to_string(),
                port: u16::try_from(number(&value, "port")?).ok().filter(|p| *p != 0)?,
            }
        }
        "send-pin" => ToDaemon::SendPin { id: id_of(&value)?, pin: pin_of(&value)? },
        "send-touch" => ToDaemon::SendTouch { id: id_of(&value)? },
        "send-refresh" => ToDaemon::SendRefresh { id: id_of(&value)? },
        "send-cancel" => ToDaemon::SendCancel { id: id_of(&value)? },
        _ => return None,
    })
}

/// A PIN as typed: 1 to 32 characters, none of them a control character.
fn pin_of(value: &Value) -> Option<String> {
    let pin = value.get("pin")?.as_str()?;
    (!pin.is_empty() && pin.chars().count() <= MAX_PIN && !pin.chars().any(char::is_control)).then(|| pin.to_string())
}

impl FromDaemon {
    pub fn encode(&self) -> String {
        let value = match self {
            FromDaemon::State { alias, address, quick, receiving } => {
                json!({"t": "state", "alias": alias, "address": address, "quick": quick, "receiving": receiving})
            }
            FromDaemon::Offer { id, from, count, bytes, name } => {
                json!({"t": "offer", "id": id, "from": from, "count": count, "bytes": bytes, "name": name})
            }
            FromDaemon::OfferGone { id } => json!({"t": "offer-gone", "id": id}),
            FromDaemon::Progress { id, done, total } => json!({"t": "progress", "id": id, "done": done, "total": total}),
            FromDaemon::Done { id, from, saved, failed, name } => {
                json!({"t": "done", "id": id, "from": from, "saved": saved, "failed": failed, "name": name})
            }
            FromDaemon::Refused { from, why } => json!({"t": "refused", "from": from, "why": why.word()}),
            FromDaemon::Text { from, text } => json!({"t": "text", "from": from, "text": text}),
            FromDaemon::Devices(list) => json!({"t": "devices", "list": list.iter().take(MAX_DEVICES).map(Device::to_json).collect::<Vec<_>>()}),
            FromDaemon::SendRequest { id, revision, count, bytes, names, by_root } => json!({"t": "send-request", "id": id, "rev": revision, "count": count, "bytes": bytes, "names": names, "root": by_root}),
            FromDaemon::SendDevices { id, searching, list } => {
                json!({"t": "send-devices", "id": id, "searching": searching, "list": list.iter().take(MAX_DEVICES).map(Device::to_json).collect::<Vec<_>>()})
            }
            FromDaemon::SendPinWanted { id, wrong } => json!({"t": "send-pin-wanted", "id": id, "wrong": wrong}),
            FromDaemon::SendProgress { id, to, file, count, done, total } => {
                json!({"t": "send-progress", "id": id, "to": to, "file": file, "count": count, "done": done, "total": total})
            }
            FromDaemon::SendDone { id, to, outcome, sent } => json!({"t": "send-done", "id": id, "to": to, "outcome": outcome.word(), "sent": sent}),
        };
        format!("{value}\n")
    }
}

pub fn parse_from_daemon(line: &str) -> Option<FromDaemon> {
    let (value, kind) = object(line)?;
    let count = |key: &str| number(&value, key).map(|n| n.min(u32::MAX as u64) as u32);
    Some(match kind.as_str() {
        "state" => FromDaemon::State {
            alias: text_of(&value, "alias", MAX_NAME)?,
            address: value.get("address").and_then(|a| a.as_str()).map(|a| tidy(a, 45)).filter(|a| !a.is_empty()),
            quick: value.get("quick")?.as_bool()?,
            receiving: value.get("receiving")?.as_bool()?,
        },
        "offer" => FromDaemon::Offer {
            id: id_of(&value)?,
            from: text_of(&value, "from", MAX_NAME)?,
            count: count("count")?,
            bytes: number(&value, "bytes")?,
            name: text_of(&value, "name", MAX_NAME)?,
        },
        "offer-gone" => FromDaemon::OfferGone { id: id_of(&value)? },
        "progress" => FromDaemon::Progress { id: id_of(&value)?, done: number(&value, "done")?, total: number(&value, "total")? },
        "done" => FromDaemon::Done {
            id: id_of(&value)?,
            from: text_of(&value, "from", MAX_NAME)?,
            saved: count("saved")?,
            failed: count("failed")?,
            name: text_of(&value, "name", MAX_NAME)?,
        },
        "refused" => FromDaemon::Refused { from: text_of(&value, "from", MAX_NAME)?, why: Refusal::from_word(value.get("why")?.as_str()?)? },
        "text" => FromDaemon::Text { from: text_of(&value, "from", MAX_NAME)?, text: text_of(&value, "text", MAX_TEXT)? },
        "devices" => FromDaemon::Devices(devices_of(&value)?),
        "send-request" => FromDaemon::SendRequest {
            id: id_of(&value)?,
            revision: count("rev")?,
            count: count("count")?,
            bytes: number(&value, "bytes")?,
            names: value.get("names")?.as_array()?.iter().take(SHOWN_NAMES).map(|n| n.as_str().map(|n| tidy(n, MAX_NAME))).collect::<Option<Vec<_>>>()?,
            by_root: value.get("root")?.as_bool()?,
        },
        "send-devices" => FromDaemon::SendDevices { id: id_of(&value)?, searching: value.get("searching")?.as_bool()?, list: devices_of(&value)? },
        "send-pin-wanted" => FromDaemon::SendPinWanted { id: id_of(&value)?, wrong: value.get("wrong")?.as_bool()? },
        "send-progress" => FromDaemon::SendProgress {
            id: id_of(&value)?,
            to: text_of(&value, "to", MAX_NAME)?,
            file: count("file")?,
            count: count("count")?,
            done: number(&value, "done")?,
            total: number(&value, "total")?,
        },
        "send-done" => FromDaemon::SendDone {
            id: id_of(&value)?,
            to: text_of(&value, "to", MAX_NAME)?,
            outcome: SendOutcome::from_word(value.get("outcome")?.as_str()?)?,
            sent: count("sent")?,
        },
        _ => return None,
    })
}

/// The "list" of a message: at most `MAX_DEVICES`, the odd ones left out.
fn devices_of(value: &Value) -> Option<Vec<Device>> {
    Some(value.get("list")?.as_array()?.iter().take(MAX_DEVICES).filter_map(Device::from_json).collect())
}

// ---------- the submit socket: a program of the desktop user hands over files to send ----------
//
// One line per message as above. The requester sends `File` with exactly one open descriptor
// attached (SCM_RIGHTS, see fdpass.rs) and waits for `Ok` before the next; then `Go`. What is
// sent is read from those descriptors — opened by the requester with its own rights — and never
// looked up by name; the name is only what the other device is told (rewritten by names.rs).

/// Why the daemon does not take a submission.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SubmitRefusal {
    /// Something is being sent or received already.
    Busy,
    /// More files than the limit for one transfer.
    TooMany,
    /// More bytes than the limit for one transfer.
    TooBig,
    /// What was handed over is not an open, readable, regular file.
    NotAFile,
    /// Only root may name the target itself (`--to`).
    NotAllowed,
    /// Nobody could be asked where to send: the desktop shell is not connected.
    NoScreen,
    /// Not a message of this protocol, or out of order.
    Bad,
}

impl SubmitRefusal {
    const ALL: [SubmitRefusal; 7] = [
        SubmitRefusal::Busy,
        SubmitRefusal::TooMany,
        SubmitRefusal::TooBig,
        SubmitRefusal::NotAFile,
        SubmitRefusal::NotAllowed,
        SubmitRefusal::NoScreen,
        SubmitRefusal::Bad,
    ];
    fn word(self) -> &'static str {
        match self {
            SubmitRefusal::Busy => "busy",
            SubmitRefusal::TooMany => "too-many",
            SubmitRefusal::TooBig => "too-big",
            SubmitRefusal::NotAFile => "not-a-file",
            SubmitRefusal::NotAllowed => "not-allowed",
            SubmitRefusal::NoScreen => "no-screen",
            SubmitRefusal::Bad => "bad",
        }
    }
}

/// Requester to daemon.
#[derive(Clone, PartialEq, Debug)]
pub enum ToSubmit {
    /// One file: its name for the other side. The open file comes attached to this message.
    File { name: String },
    /// That was all. `to` (root's connection only): send to the device with this name or
    /// address instead of asking on the screen — after `Confirm`.
    Go { to: Option<String> },
    /// Root's connection only: yes, send to the `Target` just reported.
    Confirm,
}

/// Daemon to requester.
#[derive(Clone, PartialEq, Debug)]
pub enum FromSubmit {
    /// The file was taken; send the next.
    Ok,
    Refused(SubmitRefusal),
    /// The request waits for the owner to choose a device on the screen.
    Queued,
    /// Answer to `Go { to }`: the device that was found; waits for `Confirm`.
    Target(Device),
    Progress { done: u64, total: u64 },
    Result { outcome: SendOutcome, sent: u32, to: String },
}

impl ToSubmit {
    pub fn encode(&self) -> String {
        let value = match self {
            ToSubmit::File { name } => json!({"t": "file", "name": name}),
            ToSubmit::Go { to } => json!({"t": "go", "to": to}),
            ToSubmit::Confirm => json!({"t": "confirm"}),
        };
        format!("{value}\n")
    }
}

/// A file name as the requester gave it: any text up to 1000 characters (it is rewritten into a
/// harmless name before use), but text it must be.
pub fn parse_to_submit(line: &str) -> Option<ToSubmit> {
    let (value, kind) = object(line)?;
    Some(match kind.as_str() {
        "file" => ToSubmit::File { name: value.get("name")?.as_str()?.chars().take(1000).collect() },
        "go" => ToSubmit::Go {
            to: match value.get("to") {
                None | Some(Value::Null) => None,
                Some(to) => Some(tidy(to.as_str()?, MAX_NAME)).filter(|to| !to.is_empty()),
            },
        },
        "confirm" => ToSubmit::Confirm,
        _ => return None,
    })
}

impl FromSubmit {
    pub fn encode(&self) -> String {
        let value = match self {
            FromSubmit::Ok => json!({"t": "ok"}),
            FromSubmit::Refused(why) => json!({"t": "refused", "why": why.word()}),
            FromSubmit::Queued => json!({"t": "queued"}),
            FromSubmit::Target(device) => json!({"t": "target", "device": device.to_json()}),
            FromSubmit::Progress { done, total } => json!({"t": "progress", "done": done, "total": total}),
            FromSubmit::Result { outcome, sent, to } => json!({"t": "result", "outcome": outcome.word(), "sent": sent, "to": to}),
        };
        format!("{value}\n")
    }
}

pub fn parse_from_submit(line: &str) -> Option<FromSubmit> {
    let (value, kind) = object(line)?;
    Some(match kind.as_str() {
        "ok" => FromSubmit::Ok,
        "refused" => FromSubmit::Refused(SubmitRefusal::ALL.into_iter().find(|r| Some(r.word()) == value.get("why").and_then(|w| w.as_str()))?),
        "queued" => FromSubmit::Queued,
        "target" => FromSubmit::Target(Device::from_json(value.get("device")?)?),
        "progress" => FromSubmit::Progress { done: number(&value, "done")?, total: number(&value, "total")? },
        "result" => FromSubmit::Result {
            outcome: SendOutcome::from_word(value.get("outcome")?.as_str()?)?,
            sent: number(&value, "sent")?.min(u32::MAX as u64) as u32,
            to: text_of(&value, "to", MAX_NAME)?,
        },
        _ => return None,
    })
}

/// Collects bytes from a socket into lines. A line longer than `MAX_LINE` is an error: the
/// caller drops the connection.
#[derive(Default)]
pub struct Lines {
    pending: Vec<u8>,
}

impl Lines {
    /// Adds what was read; returns the complete lines, or Err when the peer sent an overlong one.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, ()> {
        self.pending.extend_from_slice(bytes);
        let mut lines = Vec::new();
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            if line.len() > MAX_LINE {
                return Err(());
            }
            lines.push(String::from_utf8_lossy(&line[..line.len() - 1]).into_owned());
        }
        if self.pending.len() > MAX_LINE {
            return Err(());
        }
        Ok(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let to = [
            ToDaemon::Answer { id: "3f2a-01".into(), accept: true },
            ToDaemon::Quick(false),
            ToDaemon::Cancel { id: "abc".into() },
            ToDaemon::Devices,
            ToDaemon::Quit,
            ToDaemon::Receive(true),
            ToDaemon::SendTo { id: "o-1".into(), revision: 3, fingerprint: "AB12".into(), host: "192.0.2.5".into(), port: 53317 },
            ToDaemon::SendTouch { id: "o-1".into() },
            ToDaemon::SendPin { id: "o-1".into(), pin: "123456".into() },
            ToDaemon::SendRefresh { id: "o-1".into() },
            ToDaemon::SendCancel { id: "o-1".into() },
        ];
        for message in to {
            let line = message.encode();
            assert!(line.ends_with('\n') && line.matches('\n').count() == 1);
            assert_eq!(parse_to_daemon(&line), Some(message));
        }
        let from = [
            FromDaemon::State { alias: "水滴".into(), address: Some("192.0.2.33".into()), quick: false, receiving: true },
            FromDaemon::State { alias: "Shabnam".into(), address: None, quick: true, receiving: false },
            FromDaemon::Offer { id: "s1".into(), from: "小米 14".into(), count: 3, bytes: 12_400_000, name: "照片 1.jpg".into() },
            FromDaemon::OfferGone { id: "s1".into() },
            FromDaemon::Progress { id: "s1".into(), done: 5, total: 10 },
            FromDaemon::Done { id: "s1".into(), from: "Mac".into(), saved: 2, failed: 1, name: "a (2).txt".into() },
            FromDaemon::Refused { from: "x".into(), why: Refusal::NoSpace },
            FromDaemon::Text { from: "Phone".into(), text: "你好 \"quoted\"".into() },
            FromDaemon::Devices(vec![phone()]),
            FromDaemon::SendRequest { id: "o-1".into(), revision: 2, count: 2, bytes: 3_000_000, names: vec!["照片 1.jpg".into(), "b.pdf".into()], by_root: false },
            FromDaemon::SendDevices { id: "o-1".into(), searching: true, list: vec![phone(), Device { https: false, kind: DeviceKind::Other, ..phone() }] },
            FromDaemon::SendPinWanted { id: "o-1".into(), wrong: true },
            FromDaemon::SendProgress { id: "o-1".into(), to: "iPhone".into(), file: 1, count: 2, done: 5, total: 10 },
        ];
        for message in from {
            assert_eq!(parse_from_daemon(&message.encode()), Some(message));
        }
        for outcome in SendOutcome::ALL {
            let message = FromDaemon::SendDone { id: "o-1".into(), to: "iPhone".into(), outcome, sent: 1 };
            assert_eq!(parse_from_daemon(&message.encode()), Some(message));
        }
    }

    fn phone() -> Device {
        Device { alias: "iPhone".into(), fingerprint: "AB12".into(), host: "192.0.2.5".into(), port: 53317, https: true, kind: DeviceKind::Mobile }
    }

    #[test]
    fn submit_messages_round_trip() {
        for message in [ToSubmit::File { name: "照片 \"1\".jpg".into() }, ToSubmit::Go { to: None }, ToSubmit::Go { to: Some("Mac".into()) }, ToSubmit::Confirm] {
            let line = message.encode();
            assert!(line.ends_with('\n') && line.matches('\n').count() == 1);
            assert_eq!(parse_to_submit(&line), Some(message));
        }
        let mut from = vec![FromSubmit::Ok, FromSubmit::Queued, FromSubmit::Target(phone()), FromSubmit::Progress { done: 1, total: 2 }];
        from.extend(SubmitRefusal::ALL.map(FromSubmit::Refused));
        from.extend(SendOutcome::ALL.map(|outcome| FromSubmit::Result { outcome, sent: 3, to: "Mac".into() }));
        for message in from {
            assert_eq!(parse_from_submit(&message.encode()), Some(message));
        }
    }

    #[test]
    fn a_requester_cannot_say_more_than_the_protocol_allows() {
        // No message of the submit socket names a target, except root's `to` (which the daemon
        // refuses from anybody else); the shell's messages are not understood there at all.
        for line in [
            r#"{"t":"send-to","id":"o-1","fp":"AB12","host":"192.0.2.5","port":53317}"#,
            r#"{"t":"answer","id":"s1","accept":true}"#,
            r#"{"t":"receive","on":true}"#,
            r#"{"t":"quit"}"#,
            r#"{"t":"file"}"#,
            r#"{"t":"file","name":7}"#,
            r#"{"t":"go","to":7}"#,
        ] {
            assert_eq!(parse_to_submit(line), None, "{line}");
        }
        // An empty or blank target is no target; control characters never get through.
        assert_eq!(parse_to_submit(r#"{"t":"go","to":"  "}"#), Some(ToSubmit::Go { to: None }));
        assert_eq!(parse_to_submit(r#"{"t":"go"}"#), Some(ToSubmit::Go { to: None }));
        assert_eq!(parse_to_submit("{\"t\":\"go\",\"to\":\"Mac\\u001b[31m\"}"), Some(ToSubmit::Go { to: Some("Mac[31m".into()) }));
        let Some(ToSubmit::File { name }) = parse_to_submit(&ToSubmit::File { name: "x".repeat(3000) }.encode()) else { panic!() };
        assert_eq!(name.len(), 1000);
        // The shell's side: a target must look like one, a PIN like a PIN.
        for line in [
            r#"{"t":"send-to","id":"o-1","fp":"AB 12","host":"192.0.2.5","port":53317}"#,
            r#"{"t":"send-to","id":"o-1","fp":"AB12","host":"192.0.2.5/x","port":53317}"#,
            r#"{"t":"send-to","id":"o-1","fp":"AB12","host":"192.0.2.5","port":0}"#,
            r#"{"t":"send-to","id":"o-1","fp":"AB12","host":"192.0.2.5","port":70000}"#,
            r#"{"t":"send-to","id":"../x","fp":"AB12","host":"192.0.2.5","port":1}"#,
            r#"{"t":"send-pin","id":"o-1","pin":""}"#,
            r#"{"t":"send-pin","id":"o-1","pin":"1\n2"}"#,
            r#"{"t":"send-pin","id":"o-1","pin":"123456789012345678901234567890123"}"#,
            r#"{"t":"receive","on":"yes"}"#,
            r#"{"t":"send-cancel"}"#,
        ] {
            assert_eq!(parse_to_daemon(line), None, "{line}");
        }
        assert_eq!(parse_from_daemon(r#"{"t":"send-done","id":"o-1","to":"x","outcome":"great","sent":1}"#), None);
        assert_eq!(parse_from_daemon(r#"{"t":"state","alias":"x","address":null,"quick":false}"#), None, "a state without `receiving`");
        assert_eq!(short_fingerprint("AB12CD34EF56AB78FFFF0000"), "AB12 CD34 EF56 AB78");
        assert_eq!(short_fingerprint("ab:12"), "ab12");
    }

    #[test]
    fn hostile_text_is_tamed() {
        // Control characters and escape sequences from a sender's name never reach the screen.
        let line = FromDaemon::Offer { id: "s1".into(), from: "evil\u{1b}[31m\nname\u{0}".into(), count: 1, bytes: 1, name: "x".repeat(5000) }.encode();
        let Some(FromDaemon::Offer { from, name, .. }) = parse_from_daemon(&line) else { panic!() };
        assert_eq!(from, "evil[31m name");
        assert_eq!(name.chars().count(), 120);
        assert_eq!(tidy("  a\tb\r\n ", 10), "a b");
    }

    #[test]
    fn malformed_lines_are_dropped() {
        for line in [
            "",
            "not json",
            "[]",
            "{}",
            r#"{"t":"nonsense"}"#,
            r#"{"t":"answer","id":"../../x","accept":true}"#,
            r#"{"t":"answer","id":"ok"}"#,
            r#"{"t":"answer","id":"ok","accept":"yes"}"#,
            r#"{"t":"quick","on":1}"#,
            r#"{"t":"cancel","id":""}"#,
        ] {
            assert_eq!(parse_to_daemon(line), None, "{line}");
        }
        for line in [
            r#"{"t":"offer","id":"a","from":"x","count":-1,"bytes":1,"name":"n"}"#,
            r#"{"t":"offer","id":"a b","from":"x","count":1,"bytes":1,"name":"n"}"#,
            r#"{"t":"refused","from":"x","why":"because"}"#,
            r#"{"t":"progress","id":"a","done":"1","total":2}"#,
            r#"{"t":"state","alias":"x"}"#,
        ] {
            assert_eq!(parse_from_daemon(line), None, "{line}");
        }
        // A device with a strange host or fingerprint is left out, the rest stays.
        let line = r#"{"t":"devices","list":[{"alias":"a","fp":"AB","host":"1.2.3.4; rm","port":53317,"https":true},{"alias":"b","fp":"CD","host":"1.2.3.5","port":53317,"https":true},{"alias":"c","fp":"EF","host":"1.2.3.6","port":0,"https":true}]}"#;
        let Some(FromDaemon::Devices(list)) = parse_from_daemon(line) else { panic!() };
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].alias, "b");
        assert_eq!(parse_from_daemon(&format!(r#"{{"t":"text","from":"a","text":"{}"}}"#, "x".repeat(MAX_LINE))), None);
    }

    #[test]
    fn lines_are_split_and_capped() {
        let mut lines = Lines::default();
        assert_eq!(lines.push(b"{\"t\":\"qu").unwrap(), Vec::<String>::new());
        assert_eq!(lines.push(b"it\"}\n{\"t\":\"devices\"}\npart").unwrap(), vec!["{\"t\":\"quit\"}".to_string(), "{\"t\":\"devices\"}".to_string()]);
        assert!(lines.push(&vec![b'x'; MAX_LINE + 1]).is_err());
        let mut lines = Lines::default();
        let mut long = vec![b'y'; MAX_LINE + 5];
        long.push(b'\n');
        assert!(lines.push(&long).is_err());
    }
}
