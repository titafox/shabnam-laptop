//! The rules the receiver applies before anything touches the disk, kept free of I/O so they
//! can be tested: who may send, how much, and what happens to an offer.

use crate::ipc::{Refusal, SubmitRefusal};
use std::net::{IpAddr, Ipv4Addr};

/// Most files in one transfer.
pub const MAX_FILES: usize = 500;
/// Most bytes in one transfer (8 GiB).
pub const MAX_TOTAL: u64 = 8 << 30;
/// Free space that must remain on the disk after a transfer (1 GiB).
pub const RESERVE: u64 = 1 << 30;
/// How long an offer waits for the owner's answer before it is declined.
pub const OFFER_SECONDS: u64 = 60;

/// The interface the receiver may listen on. Never the cable to the Mac (usb*), never loopback.
pub fn interface_allowed(name: &str) -> bool {
    !name.is_empty() && name != "lo" && !name.starts_with("usb") && !name.starts_with("rndis") && !name.starts_with("ncm")
}

/// True when `peer` is on the same IPv4 network as the interface (`address` with `netmask`).
/// Anything else — another subnet routed to us, IPv6 — is not a device on the home Wi-Fi.
pub fn same_network(peer: IpAddr, address: Ipv4Addr, netmask: Ipv4Addr) -> bool {
    let IpAddr::V4(peer) = peer else { return false };
    let mask = u32::from(netmask);
    mask != 0 && (u32::from(peer) & mask) == (u32::from(address) & mask)
}

/// The size check on an offer: number of files, declared bytes, and what the disk can take.
pub fn check_size(files: usize, bytes: u64, free: u64) -> Result<(), Refusal> {
    if files > MAX_FILES {
        Err(Refusal::TooMany)
    } else if bytes > MAX_TOTAL {
        Err(Refusal::TooBig)
    } else if bytes.saturating_add(RESERVE) > free {
        Err(Refusal::NoSpace)
    } else {
        Ok(())
    }
}

/// The same limits for what leaves: number of files and bytes of one send request.
pub fn check_send(files: usize, bytes: u64) -> Result<(), SubmitRefusal> {
    if files > MAX_FILES {
        Err(SubmitRefusal::TooMany)
    } else if bytes > MAX_TOTAL {
        Err(SubmitRefusal::TooBig)
    } else {
        Ok(())
    }
}

/// How long a send request waits for the owner (to choose a device, to type a PIN) and for
/// the other side's answer before it is given up.
pub const SEND_WAIT_SECONDS: u64 = 300;
/// How long a daemon that is only there for sending stays when nobody asks it for anything.
pub const SEND_IDLE_SECONDS: u64 = 20;
/// Most programs that may be handing over files at the same time (the file manager starts one
/// per selected file).
pub const MAX_SUBMITTERS: usize = 64;

/// Whether an offer from the network may be considered at all: only while receiving is switched
/// on, and not while this device is itself sending. "Off" means nobody can send to us — also
/// during the moments the port is open because the owner is looking for a device to send to.
pub fn offers_welcome(receiving: bool, sending: bool) -> bool {
    receiving && !sending
}

/// Whether the network side (port and discovery) has to be up.
pub fn network_wanted(receiving: bool, sending: bool) -> bool {
    receiving || sending
}

/// Most files of one transfer that may arrive at the same time.
pub const MAX_CONCURRENT: usize = 4;

/// Why an upload request inside an accepted transfer is turned away.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum UploadDenied {
    /// Not one of the files the owner accepted.
    Unknown,
    /// This file is arriving right now.
    InFlight,
    /// This file has arrived already.
    Completed,
    /// Too many files are arriving at once.
    TooMany,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum UploadState {
    Waiting,
    InFlight,
    Completed,
}

/// The files of one accepted transfer: each may be in flight once, and may complete once.
/// (A file whose upload failed may be tried again; the bytes it wrote stay counted, see `charge`.)
#[derive(Debug)]
pub struct Uploads {
    files: std::collections::HashMap<String, (u64, UploadState)>,
}

impl Uploads {
    /// `files`: id and announced size of every accepted file.
    pub fn new(files: impl IntoIterator<Item = (String, u64)>) -> Uploads {
        Uploads { files: files.into_iter().map(|(id, size)| (id, (size, UploadState::Waiting))).collect() }
    }

    pub fn in_flight(&self) -> usize {
        self.files.values().filter(|(_, state)| *state == UploadState::InFlight).count()
    }

    /// An upload of `id` wants to start: its accepted size, or why not.
    pub fn begin(&mut self, id: &str) -> Result<u64, UploadDenied> {
        let busy = self.in_flight();
        let (size, state) = self.files.get_mut(id).ok_or(UploadDenied::Unknown)?;
        match state {
            UploadState::InFlight => Err(UploadDenied::InFlight),
            UploadState::Completed => Err(UploadDenied::Completed),
            UploadState::Waiting if busy >= MAX_CONCURRENT => Err(UploadDenied::TooMany),
            UploadState::Waiting => {
                *state = UploadState::InFlight;
                Ok(*size)
            }
        }
    }

    /// The upload of `id` ended, saved or not.
    pub fn end(&mut self, id: &str, saved: bool) {
        if let Some((_, state)) = self.files.get_mut(id).filter(|(_, state)| *state == UploadState::InFlight) {
            *state = if saved { UploadState::Completed } else { UploadState::Waiting };
        }
    }
}

/// Counts `bytes` about to be written against the transfer's accepted `total`. `written` is the
/// one counter of the whole transfer, shared by all its files and never reset, so repeated or
/// parallel uploads cannot add up to more than the owner agreed to. False: over the total —
/// nothing more may be written.
pub fn charge(written: &std::sync::atomic::AtomicU64, bytes: u64, total: u64) -> bool {
    use std::sync::atomic::Ordering;
    written.fetch_add(bytes, Ordering::SeqCst).checked_add(bytes).is_some_and(|now| now <= total)
}

/// What to do with an offer that passed the checks.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Verdict {
    /// "Quick receive" is on: take it.
    Accept,
    /// Show it on the screen and wait for the owner.
    Ask,
    /// Nobody can be asked (no shell is connected): turn it down.
    Decline,
}

pub fn verdict(quick: bool, shell_connected: bool) -> Verdict {
    match (quick, shell_connected) {
        (true, _) => Verdict::Accept,
        (false, true) => Verdict::Ask,
        (false, false) => Verdict::Decline,
    }
}

/// Interface languages of the system.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lang {
    Zh,
    Ru,
    En,
}

/// From LC_ALL / LC_MESSAGES / LANG; Chinese when nothing is set.
pub fn lang_from(value: Option<&str>) -> Lang {
    match value.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) if v.starts_with("ru") => Lang::Ru,
        Some(v) if v.starts_with("zh") => Lang::Zh,
        Some(v) if v == "C" || v == "POSIX" => Lang::Zh,
        Some(_) => Lang::En,
        None => Lang::Zh,
    }
}

pub fn lang() -> Lang {
    let value = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()));
    lang_from(value.as_deref())
}

/// The name this laptop shows to other devices.
pub fn alias(lang: Lang) -> &'static str {
    match lang {
        Lang::Zh => "水滴",
        Lang::Ru => "Шабнам",
        Lang::En => "Shabnam",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interfaces() {
        assert!(interface_allowed("wlan0"));
        assert!(interface_allowed("en0"));
        for name in ["usb0", "usb1", "lo", "", "rndis0", "ncm0"] {
            assert!(!interface_allowed(name), "{name}");
        }
    }

    #[test]
    fn networks() {
        let (address, mask) = (Ipv4Addr::new(192, 0, 2, 33), Ipv4Addr::new(255, 255, 255, 0));
        assert!(same_network("192.0.2.163".parse().unwrap(), address, mask));
        assert!(!same_network("198.51.100.5".parse().unwrap(), address, mask));
        assert!(!same_network("10.55.0.5".parse().unwrap(), address, mask));
        assert!(!same_network("fe80::1".parse().unwrap(), address, mask));
        assert!(!same_network("1.2.3.4".parse().unwrap(), address, Ipv4Addr::new(0, 0, 0, 0)));
    }

    #[test]
    fn sizes() {
        let free = 17 << 30;
        assert_eq!(check_size(1, 1000, free), Ok(()));
        assert_eq!(check_size(MAX_FILES + 1, 1000, free), Err(Refusal::TooMany));
        assert_eq!(check_size(1, MAX_TOTAL + 1, 100 << 30), Err(Refusal::TooBig));
        assert_eq!(check_size(1, 5 << 30, 5 << 30), Err(Refusal::NoSpace));
        assert_eq!(check_size(1, u64::MAX, u64::MAX), Err(Refusal::TooBig));
        assert_eq!(check_size(1, 1, RESERVE), Err(Refusal::NoSpace));
    }

    #[test]
    fn sending_has_the_same_limits() {
        assert_eq!(check_send(1, 0), Ok(()));
        assert_eq!(check_send(MAX_FILES, MAX_TOTAL), Ok(()));
        assert_eq!(check_send(MAX_FILES + 1, 1), Err(SubmitRefusal::TooMany));
        assert_eq!(check_send(1, MAX_TOTAL + 1), Err(SubmitRefusal::TooBig));
        assert_eq!(check_send(1, u64::MAX), Err(SubmitRefusal::TooBig));
    }

    #[test]
    fn off_means_nobody_can_send_to_us() {
        // Receiving off: never, whether or not the port is open for a search.
        assert!(!offers_welcome(false, false));
        assert!(!offers_welcome(false, true));
        // Receiving on: yes, except while this device is sending.
        assert!(offers_welcome(true, false));
        assert!(!offers_welcome(true, true));
        assert!(!network_wanted(false, false), "nothing listens when there is nothing to do");
        assert!(network_wanted(false, true) && network_wanted(true, false));
    }

    #[test]
    fn a_file_arrives_once() {
        let mut uploads = Uploads::new([("a".to_string(), 10), ("b".to_string(), 20)]);
        assert_eq!(uploads.begin("nope"), Err(UploadDenied::Unknown));
        assert_eq!(uploads.begin("a"), Ok(10));
        // The same file again while it is arriving.
        assert_eq!(uploads.begin("a"), Err(UploadDenied::InFlight));
        uploads.end("a", true);
        // ... and after it has arrived.
        assert_eq!(uploads.begin("a"), Err(UploadDenied::Completed));
        // A failed upload may be tried again, once at a time.
        assert_eq!(uploads.begin("b"), Ok(20));
        uploads.end("b", false);
        assert_eq!(uploads.begin("b"), Ok(20));
        assert_eq!(uploads.begin("b"), Err(UploadDenied::InFlight));
        // Ending something that is not in flight changes nothing.
        uploads.end("a", false);
        assert_eq!(uploads.begin("a"), Err(UploadDenied::Completed));
    }

    #[test]
    fn only_a_few_at_once() {
        let mut uploads = Uploads::new((0..10).map(|i| (format!("f{i}"), 1)));
        for i in 0..MAX_CONCURRENT {
            assert_eq!(uploads.begin(&format!("f{i}")), Ok(1));
        }
        assert_eq!(uploads.in_flight(), MAX_CONCURRENT);
        assert_eq!(uploads.begin("f9"), Err(UploadDenied::TooMany));
        uploads.end("f0", true);
        assert_eq!(uploads.begin("f9"), Ok(1));
    }

    #[test]
    fn the_whole_transfer_has_one_budget() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let written = AtomicU64::new(0);
        assert!(charge(&written, 60, 100));
        assert!(charge(&written, 40, 100));
        // A retried or repeated file draws on the same budget.
        assert!(!charge(&written, 1, 100));
        assert!(!charge(&written, 0, 100), "once over, it stays over");
        assert!(written.load(Ordering::SeqCst) > 100);
        assert!(!charge(&AtomicU64::new(u64::MAX - 1), 5, u64::MAX));
        assert!(charge(&AtomicU64::new(0), 0, 0));
    }

    #[test]
    fn verdicts() {
        assert_eq!(verdict(false, true), Verdict::Ask);
        assert_eq!(verdict(false, false), Verdict::Decline);
        assert_eq!(verdict(true, false), Verdict::Accept);
        assert_eq!(verdict(true, true), Verdict::Accept);
    }

    #[test]
    fn languages() {
        assert_eq!(lang_from(None), Lang::Zh);
        assert_eq!(lang_from(Some("zh_CN.UTF-8")), Lang::Zh);
        assert_eq!(lang_from(Some("ru_RU.UTF-8")), Lang::Ru);
        assert_eq!(lang_from(Some("en_US.UTF-8")), Lang::En);
        assert_eq!(alias(Lang::Ru), "Шабнам");
    }
}
