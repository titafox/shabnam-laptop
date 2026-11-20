//! One send request from the moment files are handed over to the moment they are on their way,
//! kept free of I/O so it can be tested: who may add to it, who may say where it goes, and in
//! which order things may happen.
//!
//! The rule it enforces: the program that hands over the files (the desktop's unprivileged user —
//! the file manager, but possibly the browser) never chooses the target. For a request of that
//! user the target is one of the devices the daemon listed to the shell, chosen there by the
//! owner (`choose`, called only for root's shell connection). Only root's own `shabnam-send --to`
//! names a target itself, and then confirms the device that was found (`confirm`).

use crate::ipc::{Device, SubmitRefusal};
use crate::policy;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Stage {
    /// Waiting for a target: the picker is on the screen, or (`--to`) the device is being looked for.
    Picking,
    /// `--to` only: the device was found and reported; waiting for the requester's confirmation.
    Confirming,
    /// Asked the other side; waiting for it to accept.
    Waiting,
    /// The other side wants a PIN; waiting for the owner to type it.
    PinWanted,
    /// Files are going out.
    Uploading,
}

/// One file of a request. `payload` is the open file in the daemon, nothing in the tests.
#[derive(Debug)]
pub struct Entry<T> {
    /// The submit connection it came from.
    pub conn: u64,
    /// Already rewritten into a harmless name.
    pub name: String,
    pub size: u64,
    pub payload: T,
}

/// What a requester's disappearance did to the request.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Left {
    /// Nothing of it was in this request, or it is too late to take anything back.
    Unchanged,
    /// Its files were taken out; others remain.
    Fewer,
    /// Nothing remains (or it was the requester that had to confirm): the request is over.
    Empty,
}

#[derive(Debug)]
pub struct Request<T> {
    pub id: String,
    pub stage: Stage,
    pub files: Vec<Entry<T>>,
    /// `--to`: the name or address root asked for, and the connection that may confirm it.
    auto: Option<(String, u64)>,
    /// The devices last listed to the shell: the only ones that can be chosen.
    shown: Vec<Device>,
    pub target: Option<Device>,
}

impl<T> Request<T> {
    /// A new request. `to` must only be passed on for root's connection (the caller checks that).
    pub fn new(id: String, conn: u64, files: Vec<Entry<T>>, to: Option<String>) -> Request<T> {
        Request { id, stage: Stage::Picking, files, auto: to.map(|to| (to, conn)), shown: Vec::new(), target: None }
    }

    /// True when the target comes from the requester (root's `--to`), not from the picker.
    pub fn is_auto(&self) -> bool {
        self.auto.is_some()
    }

    pub fn count(&self) -> usize {
        self.files.len()
    }

    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).fold(0, u64::saturating_add)
    }

    pub fn first_name(&self) -> &str {
        self.files.first().map(|f| f.name.as_str()).unwrap_or("")
    }

    /// More files for the same picker (the file manager starts one sender per selected file).
    /// Only while the picker is still up, never for a `--to` request, and within the limits.
    pub fn add(&mut self, files: Vec<Entry<T>>) -> Result<(), SubmitRefusal> {
        if self.stage != Stage::Picking || self.is_auto() {
            return Err(SubmitRefusal::Busy);
        }
        let bytes = files.iter().map(|f| f.size).fold(self.bytes(), u64::saturating_add);
        policy::check_send(self.files.len() + files.len(), bytes)?;
        self.files.extend(files);
        Ok(())
    }

    /// The requester on `conn` went away. Before the owner has decided, its files go with it;
    /// after that a picker request runs on (the owner cancels on the screen), while a `--to`
    /// request ends with its terminal.
    pub fn left(&mut self, conn: u64) -> Left {
        if !self.files.iter().any(|f| f.conn == conn) {
            return Left::Unchanged;
        }
        if self.auto.as_ref().is_some_and(|(_, owner)| *owner == conn) {
            return Left::Empty;
        }
        if self.stage != Stage::Picking {
            return Left::Unchanged;
        }
        self.files.retain(|f| f.conn != conn);
        if self.files.is_empty() {
            Left::Empty
        } else {
            Left::Fewer
        }
    }

    /// Notes the list that is being shown to the owner. Returns false when it is the same as before.
    pub fn show(&mut self, list: Vec<Device>) -> bool {
        if self.shown == list {
            return false;
        }
        self.shown = list;
        true
    }

    /// The owner's choice, as the shell reports it: accepted only for a picker request that is
    /// still waiting for one, and only if exactly this device — fingerprint, address and port —
    /// is among those that were listed.
    pub fn choose(&mut self, fingerprint: &str, host: &str, port: u16) -> Option<Device> {
        if self.stage != Stage::Picking || self.is_auto() {
            return None;
        }
        let device = self.shown.iter().find(|d| d.fingerprint == fingerprint && d.host == host && d.port == port)?.clone();
        self.target = Some(device.clone());
        self.stage = Stage::Waiting;
        Some(device)
    }

    /// `--to`: the device with the wanted name or address among `list`, if it is there now.
    /// It becomes the target and waits for the requester's confirmation.
    pub fn find_wanted(&mut self, list: &[Device]) -> Option<Device> {
        let (wanted, _) = self.auto.as_ref().filter(|_| self.stage == Stage::Picking)?;
        let device = list.iter().find(|d| d.alias == *wanted || d.alias.eq_ignore_ascii_case(wanted) || d.host == *wanted)?.clone();
        self.target = Some(device.clone());
        self.stage = Stage::Confirming;
        Some(device)
    }

    /// `--to`: the requester's yes to the reported device — only from the connection that asked.
    pub fn confirm(&mut self, conn: u64) -> Option<Device> {
        if self.stage != Stage::Confirming || !self.auto.as_ref().is_some_and(|(_, owner)| *owner == conn) {
            return None;
        }
        self.stage = Stage::Waiting;
        self.target.clone()
    }

    /// The other side asked for a PIN. False (for a `--to` request, which has nobody to ask): give up.
    pub fn pin_wanted(&mut self) -> bool {
        if self.stage != Stage::Waiting || self.is_auto() {
            return false;
        }
        self.stage = Stage::PinWanted;
        true
    }

    /// The owner typed a PIN. False when none is being waited for.
    pub fn pin_given(&mut self) -> bool {
        if self.stage != Stage::PinWanted {
            return false;
        }
        self.stage = Stage::Waiting;
        true
    }

    pub fn accepted(&mut self) {
        if self.stage == Stage::Waiting {
            self.stage = Stage::Uploading;
        }
    }
}

/// The devices to offer: each once (by fingerprint and address), by name.
pub fn sorted(mut list: Vec<Device>) -> Vec<Device> {
    list.sort_by(|a, b| (a.alias.to_lowercase(), &a.host, a.port).cmp(&(b.alias.to_lowercase(), &b.host, b.port)));
    list.dedup_by(|a, b| a.fingerprint == b.fingerprint && a.host == b.host && a.port == b.port);
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::DeviceKind;

    fn device(alias: &str, host: &str) -> Device {
        Device { alias: alias.into(), fingerprint: format!("FP{}", alias.to_uppercase()), host: host.into(), port: 53317, https: true, kind: DeviceKind::Mobile }
    }

    fn files(conn: u64, sizes: &[u64]) -> Vec<Entry<()>> {
        sizes.iter().enumerate().map(|(i, size)| Entry { conn, name: format!("f{conn}-{i}"), size: *size, payload: () }).collect()
    }

    fn picker() -> Request<()> {
        Request::new("o-1".into(), 1, files(1, &[10, 20]), None)
    }

    #[test]
    fn only_a_listed_device_can_be_chosen() {
        let mut request = picker();
        let phone = device("Phone", "192.0.2.5");
        // Nothing was listed yet: nothing can be chosen.
        assert_eq!(request.choose(&phone.fingerprint, &phone.host, phone.port), None);
        assert!(request.show(vec![phone.clone(), device("Mac", "192.0.2.6")]));
        assert!(!request.show(vec![phone.clone(), device("Mac", "192.0.2.6")]), "the same list again is no news");
        // Right fingerprint at another address, another port, an unknown fingerprint: no.
        assert_eq!(request.choose(&phone.fingerprint, "192.0.2.99", phone.port), None);
        assert_eq!(request.choose(&phone.fingerprint, &phone.host, 1), None);
        assert_eq!(request.choose("FPOTHER", &phone.host, phone.port), None);
        assert_eq!(request.stage, Stage::Picking);
        assert_eq!(request.choose(&phone.fingerprint, &phone.host, phone.port), Some(phone.clone()));
        assert_eq!((request.stage, request.target.clone()), (Stage::Waiting, Some(phone.clone())));
        // Once chosen it cannot be chosen again or elsewhere.
        assert_eq!(request.choose("FPMAC", "192.0.2.6", 53317), None);
        assert_eq!(request.target, Some(phone));
    }

    #[test]
    fn a_requester_never_confirms_a_picker_request() {
        let mut request = picker();
        request.show(vec![device("Phone", "192.0.2.5")]);
        // `find_wanted` and `confirm` belong to root's --to; for a picker request they do nothing.
        assert_eq!(request.find_wanted(&[device("Phone", "192.0.2.5")]), None);
        assert_eq!(request.confirm(1), None);
        assert_eq!((request.stage, request.target.clone()), (Stage::Picking, None));
    }

    #[test]
    fn files_are_added_only_while_the_picker_is_up() {
        let mut request = picker();
        assert_eq!(request.add(files(2, &[5])), Ok(()));
        assert_eq!((request.count(), request.bytes(), request.first_name()), (3, 35, "f1-0"));
        assert_eq!(request.add(files(3, &[policy::MAX_TOTAL])), Err(SubmitRefusal::TooBig));
        assert_eq!(request.add(files(3, &vec![1; policy::MAX_FILES])), Err(SubmitRefusal::TooMany));
        assert_eq!(request.count(), 3, "a refused addition adds nothing");
        request.show(vec![device("Phone", "192.0.2.5")]);
        request.choose("FPPHONE", "192.0.2.5", 53317).unwrap();
        assert_eq!(request.add(files(4, &[1])), Err(SubmitRefusal::Busy));
        let mut to = Request::new("o-2".into(), 1, files(1, &[1]), Some("Mac".into()));
        assert_eq!(to.add(files(2, &[1])), Err(SubmitRefusal::Busy));
    }

    #[test]
    fn a_requester_that_leaves_takes_its_files_along() {
        let mut request = picker();
        request.add(files(2, &[5])).unwrap();
        assert_eq!(request.left(9), Left::Unchanged);
        assert_eq!(request.left(1), Left::Fewer);
        assert_eq!((request.count(), request.bytes()), (1, 5));
        assert_eq!(request.left(2), Left::Empty);
        // After the owner has chosen, the request no longer depends on the requester.
        let mut request = picker();
        request.show(vec![device("Phone", "192.0.2.5")]);
        request.choose("FPPHONE", "192.0.2.5", 53317).unwrap();
        assert_eq!(request.left(1), Left::Unchanged);
        assert_eq!(request.count(), 2);
    }

    #[test]
    fn to_finds_confirms_and_ends_with_its_terminal() {
        let mut request = Request::new("o-3".into(), 7, files(7, &[1]), Some("testmac".into()));
        assert!(request.is_auto());
        assert_eq!(request.find_wanted(&[device("Phone", "192.0.2.5")]), None);
        assert_eq!(request.confirm(7), None, "nothing to confirm yet");
        // The shell cannot choose for it.
        request.show(vec![device("Phone", "192.0.2.5")]);
        assert_eq!(request.choose("FPPHONE", "192.0.2.5", 53317), None);
        let mac = device("TestMac", "192.0.2.6");
        assert_eq!(request.find_wanted(&[device("Phone", "192.0.2.5"), mac.clone()]), Some(mac.clone()));
        assert_eq!(request.stage, Stage::Confirming);
        assert_eq!(request.confirm(8), None, "another connection cannot confirm");
        assert_eq!(request.confirm(7), Some(mac));
        assert_eq!(request.stage, Stage::Waiting);
        assert!(!request.pin_wanted(), "nobody to ask for a PIN");
        assert_eq!(request.left(7), Left::Empty, "its terminal closing ends it at any stage");
        // By address too.
        let mut request = Request::new("o-4".into(), 7, files(7, &[1]), Some("192.0.2.5".into()));
        assert!(request.find_wanted(&[device("Phone", "192.0.2.5")]).is_some());
    }

    #[test]
    fn the_pin_round() {
        let mut request = picker();
        assert!(!request.pin_wanted(), "not before a target was chosen");
        request.show(vec![device("Phone", "192.0.2.5")]);
        request.choose("FPPHONE", "192.0.2.5", 53317).unwrap();
        assert!(!request.pin_given());
        assert!(request.pin_wanted());
        assert_eq!(request.stage, Stage::PinWanted);
        assert!(request.pin_given());
        assert!(!request.pin_given());
        request.accepted();
        assert_eq!(request.stage, Stage::Uploading);
        assert!(!request.pin_wanted());
    }

    #[test]
    fn the_list_is_sorted_and_without_repeats() {
        let list = sorted(vec![device("mac", "192.0.2.6"), device("Phone", "192.0.2.5"), device("mac", "192.0.2.6"), device("Alpha", "192.0.2.9")]);
        assert_eq!(list.iter().map(|d| d.alias.as_str()).collect::<Vec<_>>(), ["Alpha", "mac", "Phone"]);
    }
}
