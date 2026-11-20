//! The rules the receiver applies before anything touches the disk, kept free of I/O so they
//! can be tested: who may send, how much, and what happens to an offer.

use crate::ipc::Refusal;
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
