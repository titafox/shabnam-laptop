//! One send request from the moment files are handed over to the moment they are on their way,
//! kept free of I/O so it can be tested: who may add to it, who may say where it goes, and in
//! which order things may happen.
//!
//! The rule it enforces: the program that hands over the files (the desktop's unprivileged user —
//! the file manager, but possibly the browser) never chooses the target. For a request of that
//! user the target is one of the devices the daemon listed to the shell, chosen there by the
//! owner (`choose`, called only for root's shell connection). Only root's own `shabnam-send --to`
//! names a target itself, and then confirms the device that was found (`confirm`).
//!
//! And the owner's choice is a choice for exactly the files that were shown: every change to
//! the set of files gives the request a new `revision`, the shell names the revision it showed
//! when it reports the choice, and a choice for an older one is not taken. Files join a request
//! only shortly after it appeared (`policy::MERGE_WINDOW`) and never after the owner has
//! started to use the picker (`touch`).

use crate::ipc::{Device, SubmitRefusal};
use crate::policy;
use std::time::Duration;

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
    /// That connection was root's own command, not a program of the desktop user.
    pub root: bool,
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

/// What became of the owner's choice.
#[derive(Clone, PartialEq, Debug)]
pub enum Choice {
    /// Taken: the files go to this device.
    Go(Device),
    /// Made for a set of files that has changed since: the owner is asked again.
    Stale,
    /// Not a choice that can be made (no such device listed, not the time for it).
    No,
}

#[derive(Debug)]
pub struct Request<T> {
    pub id: String,
    pub stage: Stage,
    pub files: Vec<Entry<T>>,
    /// Changes whenever the set of files does.
    pub revision: u32,
    /// The owner has used the picker: the set of files is closed.
    touched: bool,
    /// `--to`: the name or address root asked for, and the connection that may confirm it.
    auto: Option<(String, u64)>,
    /// The devices last listed to the shell: the only ones that can be chosen.
    shown: Vec<Device>,
    pub target: Option<Device>,
}

impl<T> Request<T> {
    /// A new request. `to` must only be passed on for root's connection (the caller checks that).
    pub fn new(id: String, conn: u64, files: Vec<Entry<T>>, to: Option<String>) -> Request<T> {
        Request { id, stage: Stage::Picking, files, revision: 1, touched: false, auto: to.map(|to| (to, conn)), shown: Vec::new(), target: None }
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

    /// True when every file came from root's own command.
    pub fn by_root(&self) -> bool {
        self.files.iter().all(|f| f.root)
    }

    /// The owner has started to use the picker.
    pub fn touch(&mut self) {
        self.touched = true;
    }

    /// More files for the same picker (the file manager starts one sender per selected file).
    /// Only shortly after the request appeared (`age` is how long ago that was), only before the
    /// owner has used the picker, never for a `--to` request, and within the limits. What the
    /// owner is asked about afterwards is a new revision.
    pub fn add(&mut self, files: Vec<Entry<T>>, age: Duration) -> Result<(), SubmitRefusal> {
        if self.stage != Stage::Picking || self.is_auto() || self.touched || age > policy::MERGE_WINDOW {
            return Err(SubmitRefusal::Busy);
        }
        let bytes = files.iter().map(|f| f.size).fold(self.bytes(), u64::saturating_add);
        policy::check_send(self.files.len() + files.len(), bytes)?;
        self.files.extend(files);
        self.revision += 1;
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
        self.revision += 1;
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
    /// still waiting for one, only if it was made looking at the files as they are now
    /// (`revision`), and only if exactly this device — fingerprint, address and port — is among
    /// those that were listed.
    pub fn choose(&mut self, revision: u32, fingerprint: &str, host: &str, port: u16) -> Choice {
        if self.stage != Stage::Picking || self.is_auto() {
            return Choice::No;
        }
        if revision != self.revision {
            return Choice::Stale;
        }
        let Some(device) = self.shown.iter().find(|d| d.fingerprint == fingerprint && d.host == host && d.port == port).cloned() else { return Choice::No };
        self.target = Some(device.clone());
        self.stage = Stage::Waiting;
        Choice::Go(device)
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
        sizes.iter().enumerate().map(|(i, size)| Entry { conn, root: conn >= 100, name: format!("f{conn}-{i}"), size: *size, payload: () }).collect()
    }

    fn picker() -> Request<()> {
        Request::new("o-1".into(), 1, files(1, &[10, 20]), None)
    }

    #[test]
    fn only_a_listed_device_can_be_chosen() {
        let mut request = picker();
        let phone = device("Phone", "192.0.2.5");
        // Nothing was listed yet: nothing can be chosen.
        assert_eq!(request.choose(1, &phone.fingerprint, &phone.host, phone.port), Choice::No);
        assert!(request.show(vec![phone.clone(), device("Mac", "192.0.2.6")]));
        assert!(!request.show(vec![phone.clone(), device("Mac", "192.0.2.6")]), "the same list again is no news");
        // Right fingerprint at another address, another port, an unknown fingerprint: no.
        assert_eq!(request.choose(1, &phone.fingerprint, "192.0.2.99", phone.port), Choice::No);
        assert_eq!(request.choose(1, &phone.fingerprint, &phone.host, 1), Choice::No);
        assert_eq!(request.choose(1, "FPOTHER", &phone.host, phone.port), Choice::No);
        assert_eq!(request.stage, Stage::Picking);
        assert_eq!(request.choose(1, &phone.fingerprint, &phone.host, phone.port), Choice::Go(phone.clone()));
        assert_eq!((request.stage, request.target.clone()), (Stage::Waiting, Some(phone.clone())));
        // Once chosen it cannot be chosen again or elsewhere.
        assert_eq!(request.choose(1, "FPMAC", "192.0.2.6", 53317), Choice::No);
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
        let soon = Duration::from_millis(300);
        let mut request = picker();
        assert_eq!(request.add(files(2, &[5]), soon), Ok(()));
        assert_eq!((request.count(), request.bytes(), request.first_name()), (3, 35, "f1-0"));
        assert_eq!(request.add(files(3, &[policy::MAX_TOTAL]), soon), Err(SubmitRefusal::TooBig));
        assert_eq!(request.add(files(3, &vec![1; policy::MAX_FILES]), soon), Err(SubmitRefusal::TooMany));
        assert_eq!(request.count(), 3, "a refused addition adds nothing");
        request.show(vec![device("Phone", "192.0.2.5")]);
        assert!(matches!(request.choose(request.revision, "FPPHONE", "192.0.2.5", 53317), Choice::Go(_)));
        assert_eq!(request.add(files(4, &[1]), soon), Err(SubmitRefusal::Busy));
        let mut to = Request::new("o-2".into(), 1, files(1, &[1]), Some("Mac".into()));
        assert_eq!(to.add(files(2, &[1]), soon), Err(SubmitRefusal::Busy));
    }

    #[test]
    fn files_join_only_at_the_start_and_never_once_the_owner_is_at_it() {
        let mut request = picker();
        assert_eq!(request.add(files(2, &[5]), policy::MERGE_WINDOW), Ok(()));
        // The burst is over: what comes now is somebody else's business.
        assert_eq!(request.add(files(3, &[5]), policy::MERGE_WINDOW + Duration::from_millis(1)), Err(SubmitRefusal::Busy));
        assert_eq!(request.add(files(3, &[5]), Duration::from_secs(60)), Err(SubmitRefusal::Busy));
        assert_eq!((request.count(), request.revision), (3, 2));
        // The owner clicked in the picker right away: closed, even inside the window.
        let mut request = picker();
        request.touch();
        assert_eq!(request.add(files(2, &[5]), Duration::ZERO), Err(SubmitRefusal::Busy));
        assert_eq!((request.count(), request.revision), (2, 1));
    }

    #[test]
    fn a_choice_counts_only_for_the_files_that_were_shown() {
        let phone = device("Phone", "192.0.2.5");
        let mut request = picker();
        request.show(vec![phone.clone()]);
        assert_eq!(request.revision, 1);
        // The owner is looking at two files; a third is slipped in.
        assert_eq!(request.add(files(2, &[5]), Duration::from_millis(100)), Ok(()));
        assert_eq!(request.revision, 2);
        assert_eq!(request.choose(1, &phone.fingerprint, &phone.host, phone.port), Choice::Stale);
        assert_eq!((request.stage, request.target.clone(), request.count()), (Stage::Picking, None, 3), "nothing was sent anywhere");
        // A requester taking its files back changes the set as well.
        assert_eq!(request.left(2), Left::Fewer);
        assert_eq!(request.revision, 3);
        assert_eq!(request.choose(2, &phone.fingerprint, &phone.host, phone.port), Choice::Stale);
        assert_eq!(request.choose(u32::MAX, &phone.fingerprint, &phone.host, phone.port), Choice::Stale);
        // Asked again, with what is there now.
        assert_eq!(request.choose(3, &phone.fingerprint, &phone.host, phone.port), Choice::Go(phone));
        // A stale number for a device that is not listed is still just "stale" — nothing is learnt from it.
        let mut request = picker();
        assert_eq!(request.choose(7, "FPNONE", "192.0.2.9", 1), Choice::Stale);
    }

    #[test]
    fn who_asked_is_remembered() {
        assert!(!picker().by_root());
        let mut request = Request::new("o-5".into(), 100, files(100, &[1]), None);
        assert!(request.by_root());
        // One file from a program of the desktop user, and it is no longer root's request.
        request.add(files(2, &[1]), Duration::ZERO).unwrap();
        assert!(!request.by_root());
    }

    #[test]
    fn a_requester_that_leaves_takes_its_files_along() {
        let mut request = picker();
        request.add(files(2, &[5]), Duration::ZERO).unwrap();
        assert_eq!(request.left(9), Left::Unchanged);
        assert_eq!(request.left(1), Left::Fewer);
        assert_eq!((request.count(), request.bytes()), (1, 5));
        assert_eq!(request.left(2), Left::Empty);
        // After the owner has chosen, the request no longer depends on the requester.
        let mut request = picker();
        request.show(vec![device("Phone", "192.0.2.5")]);
        assert!(matches!(request.choose(request.revision, "FPPHONE", "192.0.2.5", 53317), Choice::Go(_)));
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
        assert_eq!(request.choose(1, "FPPHONE", "192.0.2.5", 53317), Choice::No);
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
        assert!(matches!(request.choose(request.revision, "FPPHONE", "192.0.2.5", 53317), Choice::Go(_)));
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
