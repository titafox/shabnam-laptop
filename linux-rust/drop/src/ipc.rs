//! The messages between the receiving daemon (shabnam-dropd, user "shabnam") and the desktop shell
//! (shabnam-shell, root), over a unix socket: one JSON object per line, a fixed set of messages.
//!
//! This one file is compiled into both programs (the shell includes it by path), so the two
//! sides cannot drift apart. It needs serde_json and nothing else.
//!
//! Neither side trusts the other's text: everything read here is cut to a fixed length and
//! stripped of control characters before it is handed on, and unknown or malformed lines are
//! dropped (`None`). The shell never sees anything from the network that did not pass through
//! `parse_from_daemon`.

use serde_json::{json, Value};

/// Name of the socket inside the daemon user's runtime directory (/run/user/<uid>, mode 0700).
pub const SOCKET_NAME: &str = "shabnam-drop.sock";
/// No line is longer than this; a peer that sends more is disconnected.
pub const MAX_LINE: usize = 8192;
const MAX_NAME: usize = 120;
const MAX_TEXT: usize = 600;
const MAX_ID: usize = 64;
const MAX_DEVICES: usize = 32;

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

/// A device that answered on the network (for `shabnam-send`).
#[derive(Clone, PartialEq, Debug)]
pub struct Device {
    pub alias: String,
    pub fingerprint: String,
    pub host: String,
    pub port: u16,
    pub https: bool,
}

/// Shell (or `shabnam-send`) to daemon.
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
    /// Stop receiving: the daemon closes its ports and exits.
    Quit,
}

/// Daemon to shell.
#[derive(Clone, PartialEq, Debug)]
pub enum FromDaemon {
    /// Sent on connect and whenever something in it changes. `address` is None while the
    /// Wi-Fi interface has no address (then nothing is listening).
    State { alias: String, address: Option<String>, quick: bool },
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
        _ => return None,
    })
}

impl FromDaemon {
    pub fn encode(&self) -> String {
        let value = match self {
            FromDaemon::State { alias, address, quick } => json!({"t": "state", "alias": alias, "address": address, "quick": quick}),
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
            FromDaemon::Devices(list) => {
                let list: Vec<Value> = list
                    .iter()
                    .take(MAX_DEVICES)
                    .map(|d| json!({"alias": d.alias, "fp": d.fingerprint, "host": d.host, "port": d.port, "https": d.https}))
                    .collect();
                json!({"t": "devices", "list": list})
            }
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
        "devices" => FromDaemon::Devices(
            value
                .get("list")?
                .as_array()?
                .iter()
                .take(MAX_DEVICES)
                .filter_map(|d| {
                    let fingerprint = d.get("fp")?.as_str()?;
                    let host = d.get("host")?.as_str()?;
                    let plain = |text: &str, extra: &str| !text.is_empty() && text.len() <= 128 && text.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(c));
                    (plain(fingerprint, "") && plain(host, ".:%")).then_some(())?;
                    Some(Device {
                        alias: text_of(d, "alias", MAX_NAME)?,
                        fingerprint: fingerprint.to_string(),
                        host: host.to_string(),
                        port: u16::try_from(number(d, "port")?).ok().filter(|p| *p != 0)?,
                        https: d.get("https")?.as_bool()?,
                    })
                })
                .collect(),
        ),
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
        ];
        for message in to {
            let line = message.encode();
            assert!(line.ends_with('\n') && line.matches('\n').count() == 1);
            assert_eq!(parse_to_daemon(&line), Some(message));
        }
        let from = [
            FromDaemon::State { alias: "水滴".into(), address: Some("192.0.2.33".into()), quick: false },
            FromDaemon::State { alias: "Shabnam".into(), address: None, quick: true },
            FromDaemon::Offer { id: "s1".into(), from: "小米 14".into(), count: 3, bytes: 12_400_000, name: "照片 1.jpg".into() },
            FromDaemon::OfferGone { id: "s1".into() },
            FromDaemon::Progress { id: "s1".into(), done: 5, total: 10 },
            FromDaemon::Done { id: "s1".into(), from: "Mac".into(), saved: 2, failed: 1, name: "a (2).txt".into() },
            FromDaemon::Refused { from: "x".into(), why: Refusal::NoSpace },
            FromDaemon::Text { from: "Phone".into(), text: "你好 \"quoted\"".into() },
            FromDaemon::Devices(vec![Device { alias: "iPhone".into(), fingerprint: "AB12".into(), host: "192.0.2.5".into(), port: 53317, https: true }]),
        ];
        for message in from {
            assert_eq!(parse_from_daemon(&message.encode()), Some(message));
        }
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
