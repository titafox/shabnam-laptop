//! 隔空传送 in the shell: the switch in the quick panel, the notice card above the shelf's
//! right end that asks before a transfer is accepted and tells when files have arrived — and,
//! for sending, the picker: a panel in the middle of the screen that lists the devices the
//! daemon found and lets the owner choose where the files go.
//! The daemon itself is another program (see drop.rs); this file only shows what it reports
//! and sends back the owner's clicks. Its texts are in lang.rs with all the others.
//!
//! Sending and trust: the files come from a program of the desktop's unprivileged user (the
//! file manager — or the browser, which must be assumed hostile). That program cannot say where
//! they go. The target is chosen here, by a click in a surface only this (root) shell draws:
//! every device is listed with name, address and certificate fingerprint; a device the owner
//! has sent to before (remembered by fingerprint) is marked and sends at one click, any other
//! asks for a second, explicit confirmation. No key confirms a send.
//!
//! Since the picker appears when such a program wants it to, it must not be possible to time
//! it under a click the owner meant for something else, nor to change what is being sent
//! between the look and the click:
//! - for `GUARD` after the picker appears, and after anything in it changes (the files, the
//!   device list, the view), presses in it are ignored and it is drawn dimmed;
//! - a click counts when it is pressed and released on the same row or button;
//! - a request from a program of the desktop user always goes through the confirmation step,
//!   whether the device is known or not (one click is only for requests root started itself);
//! - every file name is listed (the first few, and how many more), and the choice is sent with
//!   the revision of the file set that was on the screen: the daemon refuses it for another.

use super::{inside, wrap, Act, Key, Panel, QuickHit, QuickLayout, Rect, Ui, ACCENT, CHIP, HOVER, ON_ACCENT, PANEL, TEXT, TEXT2, WARN};
use crate::draw::{argb, Canvas};
use crate::drop::ipc::{short_fingerprint, Device, DeviceKind, FromDaemon, Refusal, SendOutcome, ToDaemon};
use crate::drop::{Event, Request};
use crate::lang::{self, fill, Count, Msg};
use crate::sys;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Where received files are. Fixed here: the receiver's word is never used as a path.
const DOWNLOADS: &str = "/home/shabnam/Downloads";
/// Logical size of the notice card's surface.
pub const NOTICE_SIZE: (u32, u32) = (372, 132);

// ---------- state ----------

/// What the notice card shows.
#[derive(Clone, PartialEq, Debug)]
enum Notice {
    Offer { id: String, from: String, count: u32, bytes: u64, name: String },
    Progress { id: String, from: String, done: u64, total: u64 },
    Done { from: String, saved: u32, failed: u32, name: String },
    Refused { from: String, why: Refusal },
    Text { from: String, text: String },
    StartFailed,
    /// Files are on their way out (`file` 0: waiting for the other side to accept).
    Sending { id: String, to: String, file: u32, count: u32, done: u64, total: u64 },
    SendResult { to: String, outcome: SendOutcome, sent: u32 },
}

impl Notice {
    /// How long it stays without a click; None: until something happens.
    fn lifetime(&self) -> Option<Duration> {
        match self {
            Notice::Offer { .. } | Notice::Progress { .. } | Notice::Sending { .. } => None,
            Notice::SendResult { outcome: SendOutcome::Sent, .. } => Some(Duration::from_secs(15)),
            Notice::SendResult { .. } => Some(Duration::from_secs(30)),
            Notice::Done { .. } => Some(Duration::from_secs(20)),
            Notice::Text { .. } => Some(Duration::from_secs(30)),
            Notice::Refused { .. } | Notice::StartFailed => Some(Duration::from_secs(10)),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum NoticeHit {
    /// The right-hand (main) button, the left-hand one, the cross.
    Main,
    Other,
    Dismiss,
}

// ---------- sending: the picker's state ----------

/// Where devices the owner has sent to are remembered (one certificate fingerprint per line,
/// in the shell's own configuration directory, which only root can write).
const KNOWN_FILE: &str = "drop-known";
const KNOWN_MOST: usize = 64;

fn is_fingerprint(text: &str) -> bool {
    (16..=128).contains(&text.len()) && text.chars().all(|c| c.is_ascii_alphanumeric())
}

/// A device counts as known only by its certificate: one that does not encrypt has nothing
/// that could not be copied, and is asked about every time.
fn is_known(known: &[String], device: &Device) -> bool {
    device.https && known.iter().any(|k| *k == device.fingerprint)
}

/// Notes that the owner sent to `device`; true when the list changed.
fn remember(known: &mut Vec<String>, device: &Device) -> bool {
    if !device.https || !is_fingerprint(&device.fingerprint) || known.first() == Some(&device.fingerprint) {
        return false;
    }
    known.retain(|k| *k != device.fingerprint);
    known.insert(0, device.fingerprint.clone());
    known.truncate(KNOWN_MOST);
    true
}

#[derive(Clone, PartialEq, Debug)]
enum SendView {
    /// The devices to choose from.
    List,
    /// One was clicked that the owner has not sent to before: all of it, and "send?".
    Confirm(Device),
    /// The chosen device wants a PIN.
    Pin { wrong: bool },
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum SendHit {
    Row(usize),
    Refresh,
    Cancel,
    Back,
    Go,
}

/// What the panel should do after something happened.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Wish {
    Open,
    Close,
    Redraw,
}

/// How long the picker ignores presses after it appeared or changed: long enough that a click
/// already on its way when the picker came up (a double click's second half, a click aimed at
/// what was there before) lands on nothing, short enough not to be felt by someone reading it.
const GUARD: Duration = Duration::from_millis(700);
/// File names listed in the picker; of more, the first `NAME_LINES - 1` and a count.
const NAME_LINES: usize = 6;

#[derive(Clone, PartialEq, Debug)]
struct SendUi {
    id: String,
    /// Which set of files this is (it goes back with the choice).
    revision: u32,
    count: u32,
    bytes: u64,
    /// The first names of the files.
    names: Vec<String>,
    /// The request is root's own, not one of a program of the desktop user.
    by_root: bool,
    /// Presses before this moment are ignored.
    quiet_until: Instant,
    /// What the pointer was pressed on (after the quiet time); a release on the same acts.
    pressed: Option<SendHit>,
    /// The daemon has been told that the owner is using the picker.
    touched: bool,
    devices: Vec<Device>,
    searching: bool,
    view: SendView,
    /// The device the owner chose.
    target: Option<Device>,
    /// The owner has decided: closing the picker no longer withdraws the request.
    committed: bool,
    pin: String,
    hover: Option<SendHit>,
    scroll: f32,
    /// Shown by a test command: its buttons only close it.
    demo: bool,
    /// (Test command) show the "no Wi-Fi" text whatever the daemon says.
    no_wifi: bool,
}

impl SendUi {
    fn new(id: String, revision: u32, count: u32, bytes: u64, names: Vec<String>, by_root: bool, now: Instant) -> SendUi {
        // "Searching" until the first list arrives.
        SendUi {
            id,
            revision,
            count,
            bytes,
            names,
            by_root,
            quiet_until: now + GUARD,
            pressed: None,
            touched: false,
            devices: Vec::new(),
            searching: true,
            view: SendView::List,
            target: None,
            committed: false,
            pin: String::new(),
            hover: None,
            scroll: 0.0,
            demo: false,
            no_wifi: false,
        }
    }

    /// Something the owner would have to read again changed: no press counts for a moment.
    fn hush(&mut self, now: Instant) {
        (self.quiet_until, self.pressed, self.hover) = (now + GUARD, None, None);
    }

    fn quiet(&self, now: Instant) -> bool {
        now < self.quiet_until
    }

    /// The pointer went down on `hit` (None: on nothing). During the quiet time it is not noted.
    fn press(&mut self, hit: Option<SendHit>, now: Instant) {
        self.pressed = if self.quiet(now) { None } else { hit };
    }

    /// The pointer came up on `hit`: a click, if that is what it went down on.
    fn release(&mut self, hit: Option<SendHit>, now: Instant) -> Option<SendHit> {
        let pressed = self.pressed.take()?;
        (Some(pressed) == hit && !self.quiet(now)).then_some(pressed)
    }

    /// The lines of the file list: the names, or the first of them and how many more.
    fn name_lines(&self) -> (usize, u32) {
        let count = self.count as usize;
        if count <= NAME_LINES.min(self.names.len()) {
            (count, 0)
        } else {
            let shown = self.names.len().min(NAME_LINES - 1);
            (shown, (count - shown) as u32)
        }
    }

    /// The buttons at the bottom: the left one (if any) and the right one.
    fn buttons(&self) -> (Option<(SendHit, Msg)>, (SendHit, Msg)) {
        match &self.view {
            SendView::List if self.searching => (None, (SendHit::Cancel, Msg::Cancel)),
            SendView::List => (Some((SendHit::Refresh, Msg::SendRefresh)), (SendHit::Cancel, Msg::Cancel)),
            SendView::Confirm(_) => (Some((SendHit::Back, Msg::SendBack)), (SendHit::Go, Msg::SendGo)),
            SendView::Pin { .. } => (Some((SendHit::Cancel, Msg::Cancel)), (SendHit::Go, Msg::SendGo)),
        }
    }
}

#[derive(Default)]
pub(super) struct DropUi {
    requests: Option<mpsc::Sender<Request>>,
    /// The receiver is running and connected.
    on: bool,
    /// Asked to start, no word from it yet.
    starting: bool,
    alias: String,
    address: Option<String>,
    quick: bool,
    notice: Option<Notice>,
    until: Option<Instant>,
    hover: Option<NoticeHit>,
    /// Who the running transfer is from (the progress messages do not repeat it).
    sender: String,
    /// Shown by a test command: its buttons only close it.
    demo: bool,
    /// The send request that is being decided or is on its way (the picker shows it).
    send: Option<SendUi>,
    /// Fingerprints of devices the owner has sent to, newest first.
    known: Vec<String>,
    /// Set by `apply`: what the picker panel should do now.
    wish: Option<Wish>,
    /// When the daemon was last started because somebody wanted to send.
    last_wake: Option<Instant>,
}

/// The switch's logic, apart from drawing: what a click on the tile asks for.
fn tile_request(on: bool, starting: bool) -> Option<Request> {
    match (on, starting) {
        (_, true) => None,
        (true, false) => Some(Request::Stop),
        (false, false) => Some(Request::Start),
    }
}

impl DropUi {
    fn send(&self, request: Request) {
        if let Some(requests) = &self.requests {
            let _ = requests.send(request);
        }
    }

    fn show(&mut self, notice: Notice) {
        self.until = notice.lifetime().map(|d| Instant::now() + d);
        self.notice = Some(notice);
        self.hover = None;
        self.demo = false;
    }

    /// Applies a report from the receiver; true when the notice changed.
    fn apply(&mut self, message: FromDaemon) -> bool {
        match message {
            FromDaemon::State { alias, address, quick, receiving } => {
                // A daemon that only runs to send does not make the switch show "on".
                (self.on, self.starting, self.alias, self.address, self.quick) = (receiving, false, alias, address, quick);
                if !receiving && matches!(self.notice, Some(Notice::Offer { .. } | Notice::Progress { .. })) {
                    self.notice = None;
                    return true;
                }
                self.wish = self.send.is_some().then_some(Wish::Redraw);
                return false;
            }
            FromDaemon::Offer { id, from, count, bytes, name } => {
                self.sender = from.clone();
                self.show(Notice::Offer { id, from, count, bytes, name });
            }
            FromDaemon::OfferGone { id } => {
                if !matches!(&self.notice, Some(Notice::Offer { id: shown, .. }) if *shown == id) {
                    return false;
                }
                self.notice = None;
            }
            FromDaemon::Progress { id, done, total } => {
                // Not over an offer that is still waiting for its answer.
                if matches!(&self.notice, Some(Notice::Offer { id: shown, .. }) if *shown != id) {
                    return false;
                }
                self.notice = Some(Notice::Progress { id, from: self.sender.clone(), done: done.min(total), total });
                self.until = None;
            }
            FromDaemon::Done { from, saved, failed, name, .. } => {
                self.sender.clear();
                self.show(Notice::Done { from, saved, failed, name });
            }
            FromDaemon::Refused { from, why } => self.show(Notice::Refused { from, why }),
            FromDaemon::Text { from, text } => self.show(Notice::Text { from, text }),
            FromDaemon::Devices(_) => return false,
            FromDaemon::SendRequest { id, revision, count, bytes, names, by_root } => {
                let now = Instant::now();
                match self.send.as_mut().filter(|s| s.id == id && !s.demo) {
                    // The files of the picker that is up changed, or the daemon did not take a
                    // choice because it was made for other files: the owner starts over, with
                    // what is there now, and nothing chosen.
                    Some(send) => {
                        let again = send.revision != revision || send.committed;
                        (send.revision, send.count, send.bytes, send.names, send.by_root) = (revision, count, bytes, names, by_root);
                        if again && !matches!(send.view, SendView::Pin { .. }) {
                            (send.view, send.target, send.committed) = (SendView::List, None, false);
                            send.hush(now);
                            self.wish = Some(Wish::Open);
                        } else {
                            self.wish = Some(Wish::Redraw);
                        }
                    }
                    None => {
                        self.send = Some(SendUi::new(id, revision, count, bytes, names, by_root, now));
                        self.wish = Some(Wish::Open);
                    }
                }
                return false;
            }
            FromDaemon::SendDevices { id, searching, list } => {
                if let Some(send) = self.send.as_mut().filter(|s| s.id == id && !s.demo) {
                    // A device that is no longer offered cannot be confirmed.
                    if matches!(&send.view, SendView::Confirm(device) if !list.contains(device)) {
                        send.view = SendView::List;
                    }
                    // Rows may have moved under the pointer.
                    if send.devices != list {
                        send.hush(Instant::now());
                    }
                    (send.devices, send.searching, send.hover) = (list, searching, None);
                    self.wish = Some(Wish::Redraw);
                }
                return false;
            }
            FromDaemon::SendPinWanted { id, wrong } => {
                let send = match self.send.take().filter(|s| s.id == id && !s.demo) {
                    Some(send) => send,
                    None => SendUi::new(id, 0, 0, 0, Vec::new(), false, Instant::now()),
                };
                let mut send = SendUi { view: SendView::Pin { wrong }, committed: false, pin: String::new(), ..send };
                send.hush(Instant::now());
                self.send = Some(send);
                self.wish = Some(Wish::Open);
                // The "waiting" card goes while the PIN is asked for.
                if matches!(self.notice, Some(Notice::Sending { .. })) {
                    self.notice = None;
                    return true;
                }
                return false;
            }
            FromDaemon::SendProgress { id, to, file, count, done, total } => {
                // Not over an offer that is still waiting for its answer.
                if matches!(self.notice, Some(Notice::Offer { .. })) {
                    return false;
                }
                self.notice = Some(Notice::Sending { id, to, file, count, done: done.min(total), total });
                (self.until, self.demo) = (None, false);
            }
            FromDaemon::SendDone { id, to, outcome, sent } => {
                if let Some(send) = self.send.take_if(|s| s.id == id && !s.demo) {
                    // Sent, so the owner chose this device and it took the files: remember it.
                    if let (SendOutcome::Sent, Some(device)) = (outcome, &send.target) {
                        // (Unit tests must not write the machine's real list.)
                        if remember(&mut self.known, device) && !cfg!(test) {
                            sys::write_list(KNOWN_FILE, &self.known);
                        }
                    }
                    self.wish = Some(Wish::Close);
                }
                let was_showing = matches!(&self.notice, Some(Notice::Sending { id: shown, .. }) if *shown == id);
                if outcome == SendOutcome::Cancelled {
                    // The owner's own doing (or the requester's): nothing to announce.
                    if was_showing {
                        self.notice = None;
                    }
                    return was_showing;
                }
                self.show(Notice::SendResult { to, outcome, sent });
            }
        }
        true
    }

    /// What a click in the picker does: the message for the daemon, and whether the picker closes.
    /// This is the only place a target is ever named — from the list the daemon sent, by a click.
    fn send_click(&mut self, hit: SendHit) -> (Option<ToDaemon>, bool) {
        let known = &self.known;
        let Some(send) = self.send.as_mut() else { return (None, true) };
        if send.demo {
            return (None, true);
        }
        let to = |send: &mut SendUi, device: Device| {
            let message = ToDaemon::SendTo { id: send.id.clone(), revision: send.revision, fingerprint: device.fingerprint.clone(), host: device.host.clone(), port: device.port };
            (send.target, send.committed) = (Some(device), true);
            (Some(message), true)
        };
        match (send.view.clone(), hit) {
            (SendView::List, SendHit::Row(row)) => match send.devices.get(row).cloned() {
                // One click only for a request root started itself, to a device sent to before.
                // What a program of the desktop user asked for is always confirmed.
                Some(device) if send.by_root && is_known(known, &device) => to(send, device),
                Some(device) => {
                    send.view = SendView::Confirm(device);
                    send.hush(Instant::now());
                    (None, false)
                }
                None => (None, false),
            },
            (SendView::List, SendHit::Refresh) if !send.searching => {
                send.searching = true;
                (Some(ToDaemon::SendRefresh { id: send.id.clone() }), false)
            }
            (SendView::Confirm(device), SendHit::Go) => to(send, device),
            (SendView::Confirm(_), SendHit::Back) => {
                send.view = SendView::List;
                send.hush(Instant::now());
                (None, false)
            }
            (SendView::Pin { .. }, SendHit::Go) if !send.pin.is_empty() => {
                send.committed = true;
                (Some(ToDaemon::SendPin { id: send.id.clone(), pin: std::mem::take(&mut send.pin) }), true)
            }
            // Closing without having decided withdraws the request (see `send_closed`).
            (_, SendHit::Cancel) => (None, true),
            _ => (None, false),
        }
    }

    /// The first time the owner does something in the picker, the daemon is told: from then on
    /// it adds nothing to the request.
    fn touch(&mut self) -> Option<ToDaemon> {
        let send = self.send.as_mut().filter(|s| !s.demo && !s.touched)?;
        send.touched = true;
        Some(ToDaemon::SendTouch { id: send.id.clone() })
    }

    /// The picker went away (cancel, Escape, a click beside it, another panel): unless the owner
    /// had decided, the request is withdrawn.
    fn send_closed(&mut self) -> Option<ToDaemon> {
        let send = self.send.as_ref()?;
        if send.committed && !send.demo {
            return None;
        }
        let send = self.send.take()?;
        (!send.demo).then_some(ToDaemon::SendCancel { id: send.id })
    }

    /// The receiver is gone: nothing can be answered any more.
    fn went_off(&mut self, failed: bool) {
        (self.on, self.starting, self.quick, self.address) = (false, false, false, None);
        if matches!(self.notice, Some(Notice::Offer { .. } | Notice::Progress { .. } | Notice::Sending { .. })) {
            self.notice = None;
        }
        // Nothing can be sent through a daemon that is gone.
        if self.send.take_if(|s| !s.demo).is_some() {
            self.wish = Some(Wish::Close);
        }
        if failed {
            self.show(Notice::StartFailed);
        }
    }

    /// What a click on the notice does: the message for the receiver, whether to open the
    /// downloads folder, and whether the notice goes away.
    fn click(&self, hit: NoticeHit) -> (Option<ToDaemon>, bool, bool) {
        if self.demo {
            return (None, false, true);
        }
        match (&self.notice, hit) {
            (Some(Notice::Offer { id, .. }), NoticeHit::Main) => (Some(ToDaemon::Answer { id: id.clone(), accept: true }), false, true),
            (Some(Notice::Offer { id, .. }), _) => (Some(ToDaemon::Answer { id: id.clone(), accept: false }), false, true),
            (Some(Notice::Progress { id, .. }), NoticeHit::Main) => (Some(ToDaemon::Cancel { id: id.clone() }), false, false),
            (Some(Notice::Progress { .. }), _) => (None, false, false),
            (Some(Notice::Sending { id, .. }), NoticeHit::Main) => (Some(ToDaemon::SendCancel { id: id.clone() }), false, false),
            (Some(Notice::Sending { .. }), _) => (None, false, false),
            (Some(Notice::Done { saved, .. }), NoticeHit::Main) if *saved > 0 => (None, true, true),
            _ => (None, false, true),
        }
    }
}

// ---------- the shell's side ----------

fn buttons() -> (Rect, Rect, Rect) {
    let (w, h) = (NOTICE_SIZE.0 as f32, NOTICE_SIZE.1 as f32);
    // Main (right), other (left of it), the cross in the top right corner.
    ((w - 14.0 - 132.0, h - 14.0 - 32.0, 132.0, 32.0), (w - 14.0 - 132.0 - 8.0 - 96.0, h - 14.0 - 32.0, 96.0, 32.0), (w - 14.0 - 26.0, 10.0, 26.0, 26.0))
}

impl Ui {
    /// Gives the feature its line to the worker thread and asks whether a receiver is already running.
    pub fn set_drop(&mut self, requests: mpsc::Sender<Request>) {
        let _ = requests.send(Request::Probe);
        self.drop.requests = Some(requests);
        self.drop.known = sys::read_list(KNOWN_FILE).unwrap_or_default().into_iter().filter(|k| is_fingerprint(k)).take(KNOWN_MOST).collect();
    }

    /// Somebody connected to the wake socket: the daemon is wanted for sending.
    pub fn drop_wake(&mut self) {
        // Anybody of the desktop user can knock; however often, the daemon is started at most
        // once in a while.
        let now = Instant::now();
        if self.drop.last_wake.is_some_and(|last| now.duration_since(last) < Duration::from_secs(3)) {
            return;
        }
        self.drop.last_wake = Some(now);
        self.drop.send(Request::Wake);
    }

    pub fn notice_wanted(&self) -> bool {
        self.drop.notice.is_some()
    }

    /// News from the worker thread. The answer says what main.rs has to do about the picker.
    pub fn drop_event(&mut self, event: Event) -> Act {
        let changed = match event {
            Event::Message(message) => self.drop.apply(message),
            Event::Off { failed } => {
                self.drop.went_off(failed);
                true
            }
        };
        self.dirty_notice |= changed;
        self.dirty_panel |= self.open == Some(Panel::Quick);
        match (self.drop.wish.take(), self.open == Some(Panel::Send)) {
            (Some(Wish::Open), false) => Act::Open(Panel::Send),
            (Some(Wish::Close), true) => Act::Close,
            (Some(Wish::Open | Wish::Redraw), true) => {
                self.dirty_panel = true;
                Act::None
            }
            _ => Act::None,
        }
    }

    /// Once a second: a notice that has been up long enough goes away.
    pub fn drop_tick(&mut self) {
        if self.drop.until.is_some_and(|until| Instant::now() >= until) {
            self.drop.notice = None;
            self.drop.until = None;
        }
    }

    /// Switches receiving on or off (the tile, or `shabnam-shell drop on|off`). The choice is
    /// noted in ~/.config/shabnam/drop for a session script that wants to restore it.
    pub fn drop_set(&mut self, on: bool) {
        if on == (self.drop.on || self.drop.starting) {
            return;
        }
        if let Some(request) = tile_request(self.drop.on, self.drop.starting) {
            self.drop.starting = matches!(request, Request::Start);
            self.drop.send(request);
            sys::write_list("drop", &[if on { "on" } else { "off" }.to_string()]);
        }
        self.dirty_panel = true;
    }

    pub(super) fn drop_tile_click(&mut self) {
        self.drop_set(!(self.drop.on || self.drop.starting));
    }

    pub(super) fn drop_quick_click(&mut self) {
        if self.drop.on {
            self.drop.send(Request::Send(ToDaemon::Quick(!self.drop.quick)));
        }
    }

    /// The row under Wi-Fi / second screen: the switch, and next to it — while receiving is on —
    /// the "quick receive" switch.
    pub(super) fn draw_drop_row(&mut self, c: &mut Canvas, layout: &QuickLayout) {
        let lit = self.drop.on || self.drop.starting;
        let sub = if self.drop.starting {
            self.t(Msg::DropStarting).to_string()
        } else if !self.drop.on {
            self.t(Msg::DropOff).to_string()
        } else if self.drop.address.is_none() {
            self.t(Msg::DropNoWifi).to_string()
        } else {
            fill(self.t(Msg::DropOn), &[&self.drop.alias])
        };
        let (fill_color, ink) = if lit { (ACCENT, ON_ACCENT) } else { (CHIP, TEXT) };
        let hover = self.quick_hover == Some(QuickHit::Drop);
        let title = self.t(Msg::DropTitle);
        self.pill(c, layout.drop, title, &sub, fill_color, ink, if lit { ON_ACCENT } else { TEXT2 }, hover && !lit);
        if hover && lit {
            let (x, y, w, h) = layout.drop;
            c.rrect(x, y, w, h, 24.0, argb(0x33FFFFFF));
        }
        if let Some(rect) = layout.drop_quick {
            let on = self.drop.quick;
            let (title, sub) = (self.t(Msg::DropQuickTitle), self.t(if on { Msg::DropQuickOn } else { Msg::DropQuickOff }));
            let hover = self.quick_hover == Some(QuickHit::DropQuick);
            // Orange while on: this is the state in which nobody is asked.
            if on {
                self.pill(c, rect, title, sub, WARN, ON_ACCENT, ON_ACCENT, false);
            } else {
                self.pill(c, rect, title, sub, CHIP, TEXT, TEXT2, hover);
            }
        }
    }

    /// Whether the "quick receive" switch is shown (only while receiving is on).
    pub(super) fn drop_quick_shown(&self) -> bool {
        self.drop.on
    }

    fn notice_buttons(&self) -> (Option<Msg>, Option<Msg>, bool) {
        // (main button, other button, cross)
        match &self.drop.notice {
            Some(Notice::Offer { .. }) => (Some(Msg::DropAccept), Some(Msg::DropDecline), false),
            Some(Notice::Progress { .. } | Notice::Sending { .. }) => (Some(Msg::Cancel), None, false),
            Some(Notice::Done { saved, .. }) if *saved > 0 => (Some(Msg::DropShowInFolder), None, true),
            Some(_) => (Some(Msg::DropClose), None, true),
            None => (None, None, false),
        }
    }

    pub(super) fn notice_hit(&self, x: f32, y: f32) -> Option<NoticeHit> {
        let (main, other, cross) = self.notice_buttons();
        let (main_rect, other_rect, cross_rect) = buttons();
        if main.is_some() && inside(main_rect, x, y) {
            Some(NoticeHit::Main)
        } else if other.is_some() && inside(other_rect, x, y) {
            Some(NoticeHit::Other)
        } else if cross && inside(cross_rect, x, y) {
            Some(NoticeHit::Dismiss)
        } else {
            None
        }
    }

    pub(super) fn notice_move(&mut self, x: f32, y: f32) {
        let hit = self.notice_hit(x, y);
        if hit != self.drop.hover {
            self.drop.hover = hit;
            self.dirty_notice = true;
        }
    }

    pub(super) fn notice_leave(&mut self) {
        if self.drop.hover.take().is_some() {
            self.dirty_notice = true;
        }
    }

    pub(super) fn notice_press(&mut self, x: f32, y: f32) {
        let Some(hit) = self.notice_hit(x, y) else { return };
        let (message, open_folder, close) = self.drop.click(hit);
        if let Some(message) = message {
            self.drop.send(Request::Send(message));
        }
        if open_folder {
            if let Err(e) = sys::spawn(&["shabnam-files", DOWNLOADS]) {
                eprintln!("shabnam-shell: cannot open {DOWNLOADS}: {e}");
            }
        }
        if close {
            self.drop.notice = None;
            self.drop.until = None;
        }
        self.dirty_notice = true;
    }

    pub fn draw_notice(&mut self, c: &mut Canvas) {
        self.dirty_notice = false;
        c.clear();
        let Some(notice) = self.drop.notice.clone() else { return };
        let (w, h) = (NOTICE_SIZE.0 as f32, NOTICE_SIZE.1 as f32);
        c.rrect(0.0, 0.0, w, h, 19.0, argb(0x30FFFFFF));
        c.rrect(1.0, 1.0, w - 2.0, h - 2.0, 18.0, PANEL);

        // Heading: two arrows passing each other in a teal disc, and the feature's name.
        c.circle(27.0, 24.0, 10.0, ACCENT);
        c.shape(19.0, 16.0, 16.0, 16.0, ON_ACCENT, |gx, gy| {
            let arrow = |cx: f32, up: bool| {
                let t = if up { gy - 3.0 } else { 13.0 - gy };
                let shaft = (gx - cx).abs() <= 0.8 && (0.5..=10.0).contains(&t);
                let head = (0.0..=3.6).contains(&t) && (gx - cx).abs() <= t * 0.85 + 0.2;
                shaft || head
            };
            arrow(5.6, true) || arrow(10.4, false)
        });
        let title = self.t(Msg::DropTitle);
        c.text(&mut self.fonts, title, 44.0, 24.0, 12.0, TEXT2);
        let lang = lang::current();

        let (line, sub, bar) = match &notice {
            Notice::Offer { from, count, bytes, name, .. } => {
                let line = Count::DropOffer.text(lang, *count as u64, &[from]);
                let first = if *count == 1 { name.clone() } else { fill(self.t(Msg::DropAndMore), &[name]) };
                let sub = format!("{first} · {}", lang::human_size(lang, *bytes));
                (line, sub, None)
            }
            Notice::Progress { from, done, total, .. } => {
                let part = if *total == 0 { 1.0 } else { *done as f32 / *total as f32 };
                let line = if from.is_empty() { self.t(Msg::DropReceivingPlain).to_string() } else { fill(self.t(Msg::DropReceiving), &[from]) };
                (line, format!("{} / {} · {}%", lang::human_size(lang, *done), lang::human_size(lang, *total), (part * 100.0) as u32), Some(part))
            }
            Notice::Done { from, saved, failed, name } => {
                let line = match saved {
                    0 => self.t(Msg::DropNothingSaved).to_string(),
                    n => Count::DropReceived.text(lang, *n as u64, &[]),
                };
                let mut sub = if *saved == 0 { fill(self.t(Msg::DropFrom), &[from]) } else if *saved == 1 { name.clone() } else { fill(self.t(Msg::DropAndMore), &[name]) };
                if *failed > 0 && *saved > 0 {
                    sub = format!("{sub} · {}", Count::DropFailed.text(lang, *failed as u64, &[]));
                }
                (line, sub, None)
            }
            Notice::Refused { from, why } => {
                let why = match why {
                    Refusal::TooBig => Msg::DropWhyTooBig,
                    Refusal::NoSpace => Msg::DropWhyNoSpace,
                    Refusal::TooMany => Msg::DropWhyTooMany,
                };
                (fill(self.t(Msg::DropRefused), &[from]), self.t(why).to_string(), None)
            }
            Notice::Text { from, text } => (fill(self.t(Msg::DropMessageFrom), &[from]), text.clone(), None),
            Notice::StartFailed => (self.t(Msg::DropStartFailed).to_string(), self.t(Msg::DropStartFailedWhy).to_string(), None),
            Notice::Sending { to, file: 0, .. } => (fill(self.t(Msg::SendWaiting), &[to]), self.t(Msg::SendWaitingSub).to_string(), None),
            Notice::Sending { to, file, count, done, total, .. } => {
                let part = if *total == 0 { 1.0 } else { *done as f32 / *total as f32 };
                let sizes = format!("{} / {} · {}%", lang::human_size(lang, *done), lang::human_size(lang, *total), (part * 100.0) as u32);
                let sub = if *count > 1 { format!("{} · {sizes}", Count::SendFileOf.text(lang, *file as u64, &[&count.to_string()])) } else { sizes };
                (fill(self.t(Msg::SendSending), &[to]), sub, Some(part))
            }
            Notice::SendResult { to, outcome, sent } => {
                let partly = |why: Msg| match sent {
                    0 => self.t(why).to_string(),
                    // (One line only: how far it got says more than why it stopped.)
                    n => Count::SendSentPart.text(lang, *n as u64, &[]),
                };
                match outcome {
                    SendOutcome::Sent => (Count::SendSent.text(lang, *sent as u64, &[]), fill(self.t(Msg::SendSentTo), &[to]), None),
                    SendOutcome::Declined => (fill(self.t(Msg::SendDeclined), &[to]), self.t(Msg::SendNothingSent).to_string(), None),
                    SendOutcome::Busy => (fill(self.t(Msg::SendBusy), &[to]), self.t(Msg::SendNothingSent).to_string(), None),
                    SendOutcome::PinNeeded => (fill(self.t(Msg::SendFailed), &[to]), partly(Msg::SendWhyPin), None),
                    SendOutcome::Unreachable | SendOutcome::NoSuchDevice => (fill(self.t(Msg::SendFailed), &[to]), partly(Msg::SendWhyUnreachable), None),
                    SendOutcome::FileChanged => (fill(self.t(Msg::SendFailed), &[to]), partly(Msg::SendWhyFile), None),
                    SendOutcome::NoAnswer => (fill(self.t(Msg::SendFailed), &[to]), partly(Msg::SendWhyNoAnswer), None),
                    SendOutcome::Failed | SendOutcome::Cancelled => (fill(self.t(Msg::SendFailed), &[to]), partly(Msg::SendWhyInterrupted), None),
                }
            }
        };
        let line = self.fonts.fit(&line, 14.0, w - 36.0);
        c.text(&mut self.fonts, &line, 18.0, 52.0, 14.0, TEXT);
        let (main, other, cross) = self.notice_buttons();
        let (main_rect, other_rect, cross_rect) = buttons();
        let sub = self.fonts.fit(&sub, 12.0, w - 36.0);
        c.text(&mut self.fonts, &sub, 18.0, 73.0, 12.0, TEXT2);
        if let Some(part) = bar {
            let (x, y, length) = (18.0, h - 14.0 - 18.0, main_rect.0 - 18.0 - 16.0);
            c.rrect(x, y, length, 5.0, 2.5, CHIP);
            if part > 0.0 {
                c.rrect(x, y, (length * part.clamp(0.0, 1.0)).max(5.0), 5.0, 2.5, ACCENT);
            }
        }
        let hover = self.drop.hover;
        if cross {
            let (x, y, s, _) = cross_rect;
            if hover == Some(NoticeHit::Dismiss) {
                c.circle(x + s / 2.0, y + s / 2.0, s / 2.0, HOVER);
            }
            c.shape(x + 7.0, y + 7.0, 12.0, 12.0, TEXT2, |gx, gy| ((gx - gy).abs() <= 0.85 || (gx + gy - 12.0).abs() <= 0.85) && (1.0..=11.0).contains(&gx));
        }
        if let Some(label) = other {
            let (x, y, bw, bh) = other_rect;
            c.rrect(x, y, bw, bh, 16.0, CHIP);
            if hover == Some(NoticeHit::Other) {
                c.rrect(x, y, bw, bh, 16.0, HOVER);
            }
            let label = self.t(label);
            self.label_centered(c, label, x + bw / 2.0, y + bh / 2.0, 13.0, bw - 12.0, TEXT);
        }
        if let Some(label) = main {
            let (x, y, bw, bh) = main_rect;
            // Only "accept" and "show" are invitations; "cancel" and "OK" are quiet.
            let strong = matches!(label, Msg::DropAccept | Msg::DropShowInFolder);
            c.rrect(x, y, bw, bh, 16.0, if strong { ACCENT } else { CHIP });
            if hover == Some(NoticeHit::Main) {
                c.rrect(x, y, bw, bh, 16.0, if strong { argb(0x33FFFFFF) } else { HOVER });
            }
            let label = self.t(label);
            self.label_centered(c, label, x + bw / 2.0, y + bh / 2.0, 13.0, bw - 12.0, if strong { ON_ACCENT } else { TEXT });
        }
    }

    /// Test command: shows a notice with made-up content. Its buttons only close it.
    pub fn test_drop(&mut self, which: &str) {
        let notice = match which {
            "offer" => Notice::Offer { id: "demo".into(), from: "小米 14".into(), count: 3, bytes: 12_400_000, name: "IMG_20261120_0731.jpg".into() },
            "progress" => Notice::Progress { id: "demo".into(), from: "小米 14".into(), done: 5_600_000, total: 12_400_000 },
            "done" => Notice::Done { from: "小米 14".into(), saved: 3, failed: 0, name: "IMG_20261120_0731.jpg".into() },
            "text" => Notice::Text { from: "小米 14".into(), text: "https://example.com/一个链接".into() },
            "refused" => Notice::Refused { from: "小米 14".into(), why: Refusal::NoSpace },
            _ => return,
        };
        self.drop.show(notice);
        self.drop.demo = true;
        self.drop.until = Some(Instant::now() + Duration::from_secs(8));
        self.dirty_notice = true;
    }
}

// ---------- sending: the picker panel ----------

const SEND_W: f32 = 468.0;
/// The heading without the list of file names, and one line of that list.
const SEND_HEAD: f32 = 92.0;
const NAME_LINE: f32 = 18.0;
const SEND_FOOT: f32 = 62.0;
const SEND_ROW: f32 = 58.0;
/// Rows shown without scrolling.
const SEND_ROWS: usize = 4;

struct SendLayout {
    rect: Rect,
    /// Between the heading and the buttons.
    body: Rect,
    /// The part of the body that holds device rows.
    rows: Rect,
    left: Rect,
    right: Rect,
    field: Rect,
}

/// The two arrows passing each other in a teal disc: the feature's mark.
fn badge(c: &mut Canvas, cx: f32, cy: f32) {
    c.circle(cx, cy, 10.0, ACCENT);
    c.shape(cx - 8.0, cy - 8.0, 16.0, 16.0, ON_ACCENT, |gx, gy| {
        let arrow = |ax: f32, up: bool| {
            let t = if up { gy - 3.0 } else { 13.0 - gy };
            let shaft = (gx - ax).abs() <= 0.8 && (0.5..=10.0).contains(&t);
            let head = (0.0..=3.6).contains(&t) && (gx - ax).abs() <= t * 0.85 + 0.2;
            shaft || head
        };
        arrow(5.6, true) || arrow(10.4, false)
    });
}

/// A device's picture in a 20 x 20 box: a phone, a screen on a foot, or a plain ring.
fn kind_shape(kind: DeviceKind, gx: f32, gy: f32) -> bool {
    let frame = |x0: f32, y0: f32, x1: f32, y1: f32, t: f32| {
        gx >= x0 && gx <= x1 && gy >= y0 && gy <= y1 && !(gx >= x0 + t && gx <= x1 - t && gy >= y0 + t && gy <= y1 - t)
    };
    match kind {
        DeviceKind::Mobile => frame(5.5, 2.0, 14.5, 18.0, 1.5) || ((gx - 10.0).abs() <= 1.5 && (15.2..=16.2).contains(&gy)),
        DeviceKind::Desktop => frame(2.0, 3.5, 18.0, 13.5, 1.5) || ((gx - 10.0).abs() <= 1.0 && (13.5..=16.0).contains(&gy)) || ((gx - 10.0).abs() <= 4.0 && (16.0..=17.4).contains(&gy)),
        DeviceKind::Web | DeviceKind::Other => {
            let d = ((gx - 10.0).powi(2) + (gy - 10.0).powi(2)).sqrt();
            (6.3..=7.8).contains(&d) || d <= 2.0
        }
    }
}

fn kind_name(kind: DeviceKind) -> Msg {
    match kind {
        DeviceKind::Mobile => Msg::KindPhone,
        DeviceKind::Desktop => Msg::KindComputer,
        DeviceKind::Web => Msg::KindBrowser,
        DeviceKind::Other => Msg::KindDevice,
    }
}

impl Ui {
    fn send_layout(&self) -> Option<SendLayout> {
        let send = self.drop.send.as_ref()?;
        let (pw, ph) = self.panel_size;
        let w = SEND_W.min(pw - 24.0);
        let shown = send.devices.len().min(SEND_ROWS) as f32 * SEND_ROW;
        let body_h = match &send.view {
            SendView::List if send.devices.is_empty() => 104.0,
            SendView::List => shown + 30.0,
            SendView::Confirm(_) => 128.0,
            SendView::Pin { .. } => 84.0,
        };
        let (names, more) = send.name_lines();
        let head = SEND_HEAD + (names + usize::from(more > 0)) as f32 * NAME_LINE + if send.count > 0 { 6.0 } else { 0.0 };
        let h = head + body_h + SEND_FOOT;
        let (x, y) = (((pw - w) / 2.0).round(), ((ph - h) * 0.42).max(12.0).round());
        let right = (x + w - 18.0 - 124.0, y + h - 18.0 - 34.0, 124.0, 34.0);
        Some(SendLayout {
            rect: (x, y, w, h),
            body: (x + 18.0, y + head, w - 36.0, body_h),
            rows: (x + 8.0, y + head, w - 16.0, shown),
            left: (right.0 - 10.0 - 136.0, right.1, 136.0, 34.0),
            right,
            field: (x + 18.0, y + head + 2.0, w - 36.0, 42.0),
        })
    }

    fn send_hit(&self, x: f32, y: f32) -> Option<SendHit> {
        let (layout, send) = (self.send_layout()?, self.drop.send.as_ref()?);
        let (left, right) = send.buttons();
        if inside(layout.right, x, y) {
            Some(right.0)
        } else if let Some((hit, _)) = left.filter(|_| inside(layout.left, x, y)) {
            Some(hit)
        } else if send.view == SendView::List && inside(layout.rows, x, y) {
            let row = ((y - layout.rows.1 + send.scroll) / SEND_ROW) as usize;
            (row < send.devices.len()).then_some(SendHit::Row(row))
        } else {
            None
        }
    }

    /// Whether the point is on the picker's card (a press anywhere else closes it).
    pub(super) fn send_inside(&self, x: f32, y: f32) -> bool {
        self.send_layout().is_some_and(|layout| inside(layout.rect, x, y))
    }

    pub(super) fn send_move(&mut self, x: f32, y: f32) {
        let hit = self.send_hit(x, y);
        if let Some(send) = self.drop.send.as_mut().filter(|s| s.hover != hit) {
            send.hover = hit;
            self.dirty_panel = true;
        }
    }

    pub(super) fn send_leave(&mut self) -> bool {
        self.drop.send.as_mut().is_some_and(|s| s.hover.take().is_some())
    }

    pub(super) fn send_scroll(&mut self, steps: f32) {
        let Some(send) = self.drop.send.as_mut() else { return };
        let most = (send.devices.len().saturating_sub(SEND_ROWS)) as f32 * SEND_ROW;
        let scroll = (send.scroll + steps * SEND_ROW).clamp(0.0, most);
        if scroll != send.scroll {
            (send.scroll, send.hover) = (scroll, None);
            self.dirty_panel = true;
        }
    }

    /// One of the picker's buttons or rows was clicked (or a test command says so).
    fn send_do(&mut self, hit: SendHit) -> Act {
        if let Some(message) = self.drop.touch() {
            self.drop.send(Request::Send(message));
        }
        let (message, close) = self.drop.send_click(hit);
        if let Some(message) = message {
            self.drop.send(Request::Send(message));
        }
        self.dirty_panel = true;
        if close {
            Act::Close
        } else {
            Act::None
        }
    }

    /// The pointer went down in the picker. Nothing happens yet: see `send_release`.
    pub(super) fn send_press(&mut self, x: f32, y: f32) {
        let hit = self.send_hit(x, y);
        if let Some(send) = self.drop.send.as_mut() {
            send.press(hit, Instant::now());
        }
    }

    /// The pointer came up: a click, if it went down on the same row or button, and not in the
    /// moment after the picker appeared or changed.
    pub(super) fn send_release(&mut self, x: f32, y: f32) -> Act {
        let hit = self.send_hit(x, y);
        match self.drop.send.as_mut().and_then(|send| send.release(hit, Instant::now())) {
            Some(hit) => self.send_do(hit),
            None => Act::None,
        }
    }

    /// While the picker ignores presses: how long that still lasts (main.rs redraws it then).
    pub fn send_quiet_left(&self) -> Option<Duration> {
        let send = self.drop.send.as_ref().filter(|_| self.open == Some(Panel::Send))?;
        send.quiet_until.checked_duration_since(Instant::now()).filter(|left| !left.is_zero())
    }

    /// The keyboard in the picker: Escape closes it; a PIN can be typed and entered. No key
    /// chooses a device or confirms one — that takes a click on what is shown.
    pub(super) fn send_key(&mut self, key: Key) -> Act {
        let Some(send) = self.drop.send.as_mut() else { return Act::Close };
        match (&send.view, key) {
            (_, Key::Escape) => return Act::Close,
            (SendView::Pin { .. }, Key::Enter) => return self.send_do(SendHit::Go),
            (SendView::Pin { .. }, Key::Backspace) => {
                send.pin.pop();
            }
            (SendView::Pin { .. }, Key::ClearLine) => send.pin.clear(),
            (SendView::Pin { .. }, Key::Text(text)) => {
                if send.pin.chars().count() + text.chars().count() <= 32 {
                    send.pin.push_str(&text);
                }
            }
            _ => return Act::None,
        }
        self.dirty_panel = true;
        Act::None
    }

    /// The picker's panel was closed, by whatever means.
    pub(super) fn send_panel_closed(&mut self) {
        if let Some(message) = self.drop.send_closed() {
            self.drop.send(Request::Send(message));
        }
    }

    /// `live` false: drawn as it is while presses are ignored — dimmed, no highlight.
    fn send_button(&mut self, c: &mut Canvas, rect: Rect, label: Msg, strong: bool, hover: bool, live: bool) {
        let (x, y, w, h) = rect;
        let (strong, hover) = (strong && live, hover && live);
        c.rrect(x, y, w, h, 17.0, if strong { ACCENT } else { CHIP });
        if hover {
            c.rrect(x, y, w, h, 17.0, if strong { argb(0x33FFFFFF) } else { HOVER });
        }
        let label = self.t(label);
        self.label_centered(c, label, x + w / 2.0, y + h / 2.0, 13.0, w - 14.0, if strong { ON_ACCENT } else if live { TEXT } else { TEXT2 });
    }

    pub(super) fn draw_send(&mut self, c: &mut Canvas) {
        let (Some(layout), Some(send)) = (self.send_layout(), self.drop.send.clone()) else { return };
        let lang = lang::current();
        let (pw, ph) = self.panel_size;
        // The rest of the screen steps back: this is a question that wants an answer.
        c.rrect(0.0, 0.0, pw, ph, 0.0, argb(0x73000000));
        let (x, y, w, h) = layout.rect;
        c.rrect(x - 1.0, y - 1.0, w + 2.0, h + 2.0, 23.0, argb(0x30FFFFFF));
        c.rrect(x, y, w, h, 22.0, argb(0xFF202124));

        badge(c, x + 28.0, y + 26.0);
        let title = self.t(Msg::DropTitle);
        c.text(&mut self.fonts, title, x + 46.0, y + 26.0, 12.0, TEXT2);
        let alias = match (&send.view, &send.target) {
            (SendView::Confirm(device), _) => device.alias.clone(),
            (_, Some(device)) => device.alias.clone(),
            _ => "…".to_string(),
        };
        // While presses are ignored the picker says so by being dimmed.
        let live = !send.quiet(Instant::now());
        let files = Count::SendFiles.text(lang, send.count as u64, &[]);
        let size = lang::human_size(lang, send.bytes);
        let (line, sub) = match &send.view {
            SendView::List => (files, size),
            SendView::Confirm(_) => (fill(self.t(Msg::SendToTitle), &[&alias]), format!("{files} · {size}")),
            SendView::Pin { .. } if send.count == 0 => (fill(self.t(Msg::SendPinTitle), &[&alias]), String::new()),
            SendView::Pin { .. } => (fill(self.t(Msg::SendPinTitle), &[&alias]), format!("{files} · {size}")),
        };
        let line = self.fonts.fit(&line, 16.0, w - 36.0);
        c.text(&mut self.fonts, &line, x + 18.0, y + 54.0, 16.0, TEXT);
        let sub = self.fonts.fit(&sub, 12.0, w - 36.0);
        c.text(&mut self.fonts, &sub, x + 18.0, y + 76.0, 12.0, TEXT2);
        // Every file that would go, by name: the first ones, and how many more.
        let (names, more) = send.name_lines();
        for (i, name) in send.names.iter().take(names).enumerate() {
            let name = self.fonts.fit(name, 12.5, w - 36.0 - 14.0);
            let ny = y + SEND_HEAD + 7.0 + i as f32 * NAME_LINE;
            c.circle(x + 22.0, ny, 1.6, TEXT2);
            c.text(&mut self.fonts, &name, x + 32.0, ny, 12.5, TEXT);
        }
        if more > 0 {
            let rest = Count::SendMore.text(lang, more as u64, &[]);
            c.text(&mut self.fonts, &rest, x + 32.0, y + SEND_HEAD + 7.0 + names as f32 * NAME_LINE, 12.5, TEXT2);
        }

        let body = layout.body;
        match &send.view {
            SendView::List if send.devices.is_empty() => {
                let no_wifi = send.no_wifi || (!send.demo && !send.searching && self.drop.address.is_none());
                let text = self.t(if send.searching { Msg::SendSearching } else if no_wifi { Msg::SendNoWifi } else { Msg::SendEmpty });
                let lines = wrap(&mut self.fonts, text, 13.0, body.2 - 24.0, 3);
                let top = body.1 + (body.3 - lines.len() as f32 * 20.0) / 2.0 + 10.0;
                for (i, text) in lines.iter().enumerate() {
                    c.text_centered(&mut self.fonts, text, x + w / 2.0, top + i as f32 * 20.0, 13.0, body.2, if send.searching { TEXT2 } else { TEXT });
                }
            }
            SendView::List => {
                let rows = layout.rows;
                c.set_clip(Some(rows));
                for (i, device) in send.devices.iter().enumerate() {
                    let ry = rows.1 + i as f32 * SEND_ROW - send.scroll;
                    if ry + SEND_ROW <= rows.1 || ry >= rows.1 + rows.3 {
                        continue;
                    }
                    if live && send.hover == Some(SendHit::Row(i)) {
                        c.rrect(rows.0, ry + 2.0, rows.2, SEND_ROW - 4.0, 14.0, HOVER);
                    }
                    c.circle(rows.0 + 30.0, ry + SEND_ROW / 2.0, 18.0, CHIP);
                    let kind = device.kind;
                    c.shape(rows.0 + 20.0, ry + SEND_ROW / 2.0 - 10.0, 20.0, 20.0, TEXT, |gx, gy| kind_shape(kind, gx, gy));
                    let known = is_known(&self.drop.known, device);
                    let mut room = rows.2 - 60.0 - 14.0;
                    if known {
                        let label = self.t(Msg::SendKnown);
                        let lw = self.fonts.width(label, 11.0) + 16.0;
                        let (lx, ly) = (rows.0 + rows.2 - 12.0 - lw, ry + 11.0);
                        c.rrect(lx, ly, lw, 20.0, 10.0, HOVER);
                        c.text(&mut self.fonts, label, lx + 8.0, ly + 10.0, 11.0, ACCENT);
                        room -= lw + 8.0;
                    }
                    let name = self.fonts.fit(&device.alias, 14.0, room);
                    c.text(&mut self.fonts, &name, rows.0 + 60.0, ry + 20.0, 14.0, if live { TEXT } else { TEXT2 });
                    // Everything that identifies it, before anything is sent: kind, address, certificate.
                    let place = format!("{} · {}", self.t(kind_name(device.kind)), device.host);
                    let detail = if device.https { format!("{place} · {}", short_fingerprint(&device.fingerprint)) } else { format!("{place} · {}", self.t(Msg::SendPlain)) };
                    let detail = self.fonts.fit(&detail, 11.5, rows.2 - 60.0 - 14.0);
                    c.text(&mut self.fonts, &detail, rows.0 + 60.0, ry + 40.0, 11.5, if device.https { TEXT2 } else { WARN });
                }
                c.set_clip(None);
                let more = send.devices.len() > SEND_ROWS;
                let status = self.t(if send.searching { Msg::SendSearching } else { Msg::SendChoose });
                let status = if more { format!("{status} · {}", send.devices.len()) } else { status.to_string() };
                c.text(&mut self.fonts, &status, x + 18.0, rows.1 + rows.3 + 16.0, 12.0, TEXT2);
            }
            SendView::Confirm(device) => {
                let place = format!("{} · {}:{}", self.t(kind_name(device.kind)), device.host, device.port);
                c.text(&mut self.fonts, &place, body.0, body.1 + 12.0, 13.5, TEXT);
                if device.https {
                    let proof = fill(self.t(Msg::SendFingerprint), &[&short_fingerprint(&device.fingerprint)]);
                    c.text(&mut self.fonts, &proof, body.0, body.1 + 36.0, 13.5, TEXT);
                } else {
                    let proof = self.t(Msg::SendUnencrypted);
                    c.text(&mut self.fonts, proof, body.0, body.1 + 36.0, 13.5, WARN);
                }
                let note = self.t(if is_known(&self.drop.known, device) { Msg::SendKnownNote } else { Msg::SendFirstTime });
                for (i, text) in wrap(&mut self.fonts, note, 12.0, body.2, 3).iter().enumerate() {
                    c.text(&mut self.fonts, text, body.0, body.1 + 66.0 + i as f32 * 18.0, 12.0, TEXT2);
                }
            }
            SendView::Pin { wrong } => {
                let (fx, fy, fw, fh) = layout.field;
                c.rrect(fx, fy, fw, fh, 20.0, CHIP);
                let mid = fy + fh / 2.0;
                let caret = if send.pin.is_empty() {
                    let hint = self.t(Msg::SendPinField);
                    c.text(&mut self.fonts, hint, fx + 21.0, mid, 14.0, TEXT2);
                    fx + 16.0
                } else {
                    let shown = self.fonts.fit(&send.pin, 15.0, fw - 36.0);
                    fx + 16.0 + c.text(&mut self.fonts, &shown, fx + 16.0, mid, 15.0, TEXT) + 1.0
                };
                c.rrect(caret, mid - 9.0, 1.4, 18.0, 0.0, ACCENT);
                let (hint, ink) = if *wrong { (self.t(Msg::SendPinWrong), WARN) } else { (self.t(Msg::SendPinHint), TEXT2) };
                let hint = self.fonts.fit(hint, 11.5, fw - 8.0);
                c.text(&mut self.fonts, &hint, fx + 4.0, fy + fh + 18.0, 11.5, ink);
            }
        }

        let (left, right) = send.buttons();
        if let Some((hit, label)) = left {
            self.send_button(c, layout.left, label, false, send.hover == Some(hit), live);
        }
        self.send_button(c, layout.right, right.1, right.0 == SendHit::Go, send.hover == Some(right.0), live);
    }

    /// Test commands (`shabnam-shell test-send …`), for looking at and driving the picker without
    /// mouse or keyboard.
    ///   demo list|searching|empty|nowifi|confirm|plain|pin|waiting|progress|sent|declined|failed
    ///        made-up content; the buttons only close it
    ///   row N | send | back | refresh | cancel | type TEXT | stop
    ///        what a click on the N-th device, on 发送, 返回, 重新查找, 取消 would do in the picker
    ///        that is really up; typing a PIN; the cancel button of the "sending" card
    pub fn test_send(&mut self, command: &str) -> Act {
        let (verb, rest) = command.split_once(' ').map_or((command, ""), |(verb, rest)| (verb, rest.trim()));
        let live = self.open == Some(Panel::Send) && self.drop.send.as_ref().is_some_and(|s| !s.demo);
        match verb {
            "demo" => self.test_send_demo(rest),
            "row" if live => rest.parse::<usize>().ok().filter(|n| *n >= 1).map_or(Act::None, |n| self.send_do(SendHit::Row(n - 1))),
            "send" if live => self.send_do(SendHit::Go),
            "back" if live => self.send_do(SendHit::Back),
            "refresh" if live => self.send_do(SendHit::Refresh),
            "cancel" if live => Act::Close,
            "type" if live => {
                self.send_key(Key::ClearLine);
                self.send_key(Key::Text(rest.chars().filter(|c| !c.is_control()).take(32).collect()))
            }
            "stop" => {
                if let (Some(ToDaemon::SendCancel { id }), ..) = self.drop.click(NoticeHit::Main) {
                    self.drop.send(Request::Send(ToDaemon::SendCancel { id }));
                }
                Act::None
            }
            _ => Act::None,
        }
    }

    fn test_send_demo(&mut self, which: &str) -> Act {
        let device = |alias: &str, host: &str, fingerprint: &str, kind: DeviceKind, https: bool| Device { alias: alias.into(), fingerprint: fingerprint.into(), host: host.into(), port: 53317, https, kind };
        let phone = device("小米 14", "192.0.2.52", "7C1A09E4B2D35F6680A1C3E5", DeviceKind::Mobile, true);
        let list = vec![
            phone.clone(),
            device("MacBook Air", "192.0.2.163", "0F3B77D2A9C4E1586B20D4F1", DeviceKind::Desktop, true),
            device("Old tablet", "192.0.2.77", "plain", DeviceKind::Other, false),
        ];
        let notice = match which {
            "waiting" => Some(Notice::Sending { id: "demo".into(), to: phone.alias.clone(), file: 0, count: 3, done: 0, total: 12_400_000 }),
            "progress" => Some(Notice::Sending { id: "demo".into(), to: phone.alias.clone(), file: 2, count: 3, done: 5_600_000, total: 12_400_000 }),
            "sent" => Some(Notice::SendResult { to: phone.alias.clone(), outcome: SendOutcome::Sent, sent: 3 }),
            "declined" => Some(Notice::SendResult { to: phone.alias.clone(), outcome: SendOutcome::Declined, sent: 0 }),
            "failed" => Some(Notice::SendResult { to: phone.alias.clone(), outcome: SendOutcome::Unreachable, sent: 1 }),
            _ => None,
        };
        if let Some(notice) = notice {
            self.drop.show(notice);
            self.drop.demo = true;
            self.drop.until = Some(Instant::now() + Duration::from_secs(8));
            self.dirty_notice = true;
            return Act::None;
        }
        // Never over a picker that is really asking.
        if self.drop.send.as_ref().is_some_and(|s| !s.demo) {
            return Act::None;
        }
        let names = ["IMG_20261120_0731.jpg", "IMG_20261120_0732.jpg", "合同 2026-11.pdf", "报价单.xlsx", "notes.txt", "视频 01.mp4", "视频 02.mp4", "视频 03.mp4"];
        // `demo files N` shows the list with N files (the names beyond the eighth are not sent to the shell).
        let (which, count) = match which.split_once(' ') {
            Some(("files", n)) => ("list", n.parse::<u32>().unwrap_or(3).clamp(1, 500)),
            _ => (which, 3),
        };
        let shown = names.iter().take(count as usize).map(|n| n.to_string()).collect();
        // Already past the quiet time: a demo is there to be looked at.
        let mut send = SendUi::new("demo".into(), 1, count, 4_133_000 * count as u64, shown, false, Instant::now() - GUARD);
        (send.demo, send.searching) = (true, false);
        match which {
            "list" => send.devices = list,
            "searching" => send.searching = true,
            "empty" => {}
            "nowifi" => send.no_wifi = true,
            "confirm" => (send.devices, send.view) = (list, SendView::Confirm(phone)),
            "plain" => (send.view, send.devices) = (SendView::Confirm(list[2].clone()), list),
            "pin" => (send.target, send.view, send.pin) = (Some(phone), SendView::Pin { wrong: false }, "4821".into()),
            _ => return Act::None,
        }
        // In the demo one device is marked as known, to show the mark.
        self.drop.send = Some(send);
        self.dirty_panel = true;
        if self.open == Some(Panel::Send) {
            Act::None
        } else {
            Act::Open(Panel::Send)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(id: &str) -> FromDaemon {
        FromDaemon::Offer { id: id.into(), from: "Phone".into(), count: 2, bytes: 10, name: "a.jpg".into() }
    }

    #[test]
    fn the_switch() {
        assert!(matches!(tile_request(false, false), Some(Request::Start)));
        assert!(matches!(tile_request(true, false), Some(Request::Stop)));
        assert!(tile_request(false, true).is_none());
        let mut drop = DropUi::default();
        assert!(!drop.on);
        assert!(!drop.apply(FromDaemon::State { alias: "水滴".into(), address: Some("192.0.2.33".into()), quick: true, receiving: true }));
        assert!(drop.on && drop.quick);
        // The receiver went away: off, and quick receive does not survive it.
        drop.went_off(false);
        assert!(!drop.on && !drop.quick && drop.notice.is_none());
        drop.went_off(true);
        assert_eq!(drop.notice, Some(Notice::StartFailed));
    }

    #[test]
    fn an_offer_is_answered_only_by_a_click() {
        let mut drop = DropUi::default();
        assert!(drop.apply(offer("s1")));
        assert!(drop.until.is_none(), "an offer does not time out on the shell's side");
        assert_eq!(drop.click(NoticeHit::Main), (Some(ToDaemon::Answer { id: "s1".into(), accept: true }), false, true));
        assert_eq!(drop.click(NoticeHit::Other), (Some(ToDaemon::Answer { id: "s1".into(), accept: false }), false, true));
        // Somebody else's "gone" leaves it; its own removes it.
        assert!(!drop.apply(FromDaemon::OfferGone { id: "other".into() }));
        assert!(drop.notice.is_some());
        assert!(drop.apply(FromDaemon::OfferGone { id: "s1".into() }));
        assert!(drop.notice.is_none());
        // The receiver dying takes a waiting offer with it.
        drop.apply(offer("s2"));
        drop.went_off(false);
        assert!(drop.notice.is_none());
    }

    #[test]
    fn progress_and_arrival() {
        let mut drop = DropUi::default();
        drop.apply(offer("s1"));
        drop.apply(FromDaemon::Progress { id: "s1".into(), done: 5, total: 10 });
        assert_eq!(drop.notice, Some(Notice::Progress { id: "s1".into(), from: "Phone".into(), done: 5, total: 10 }));
        assert_eq!(drop.click(NoticeHit::Main), (Some(ToDaemon::Cancel { id: "s1".into() }), false, false));
        drop.apply(FromDaemon::Done { id: "s1".into(), from: "Phone".into(), saved: 2, failed: 0, name: "a.jpg".into() });
        assert!(drop.until.is_some());
        // "Show in folder" sends nothing to the receiver; it opens the fixed folder.
        assert_eq!(drop.click(NoticeHit::Main), (None, true, true));
        assert_eq!(drop.click(NoticeHit::Dismiss), (None, false, true));
        drop.apply(FromDaemon::Done { id: "s1".into(), from: "Phone".into(), saved: 0, failed: 2, name: String::new() });
        assert_eq!(drop.click(NoticeHit::Main), (None, false, true));
        // A demo notice never sends anything.
        drop.apply(offer("s3"));
        drop.demo = true;
        assert_eq!(drop.click(NoticeHit::Main), (None, false, true));
    }

    fn device(alias: &str, https: bool) -> Device {
        Device { alias: alias.into(), fingerprint: format!("{:A<24}", alias.to_uppercase()), host: "192.0.2.5".into(), port: 53317, https, kind: DeviceKind::Mobile }
    }

    fn picker(devices: Vec<Device>) -> DropUi {
        let mut drop = DropUi::default();
        assert!(!drop.apply(request(1, 2, false)));
        assert_eq!(drop.wish.take(), Some(Wish::Open));
        assert!(drop.send.as_ref().unwrap().searching, "searching until the first list arrives");
        drop.apply(FromDaemon::SendDevices { id: "o-1".into(), searching: false, list: devices });
        assert_eq!(drop.wish.take(), Some(Wish::Redraw));
        drop
    }

    /// A send request as the daemon announces it: `count` files, from root's command or not.
    fn request(revision: u32, count: u32, by_root: bool) -> FromDaemon {
        let names = (0..count.min(8)).map(|i| format!("f{i}.jpg")).collect();
        FromDaemon::SendRequest { id: "o-1".into(), revision, count, bytes: 15 * count as u64, names, by_root }
    }

    fn send_to(device: &Device) -> ToDaemon {
        ToDaemon::SendTo { id: "o-1".into(), revision: 1, fingerprint: device.fingerprint.clone(), host: device.host.clone(), port: device.port }
    }

    #[test]
    fn an_unknown_device_takes_two_clicks() {
        let phone = device("phone", true);
        let mut drop = picker(vec![phone.clone()]);
        // The first click only shows it in full; nothing is sent to the daemon.
        assert_eq!(drop.send_click(SendHit::Row(0)), (None, false));
        assert_eq!(drop.send.as_ref().unwrap().view, SendView::Confirm(phone.clone()));
        // Back, and nothing has happened.
        assert_eq!(drop.send_click(SendHit::Back), (None, false));
        assert_eq!(drop.send.as_ref().unwrap().view, SendView::List);
        assert_eq!(drop.send_click(SendHit::Go), (None, false), "there is no send button in the list");
        drop.send_click(SendHit::Row(0));
        assert_eq!(drop.send_click(SendHit::Go), (Some(send_to(&phone)), true));
        // The picker closes; the request is not withdrawn by that.
        assert_eq!(drop.send_closed(), None);
        assert!(drop.send.is_some());
        // A row that is not there does nothing.
        let mut drop = picker(vec![phone]);
        assert_eq!(drop.send_click(SendHit::Row(5)), (None, false));
    }

    #[test]
    fn closing_the_picker_undecided_withdraws_the_request() {
        let mut drop = picker(vec![device("phone", true)]);
        assert_eq!(drop.send_click(SendHit::Cancel), (None, true));
        assert_eq!(drop.send_closed(), Some(ToDaemon::SendCancel { id: "o-1".into() }));
        assert!(drop.send.is_none());
        assert_eq!(drop.send_closed(), None);
        // The same from the confirmation step (Escape, a click beside the card).
        let mut drop = picker(vec![device("phone", true)]);
        drop.send_click(SendHit::Row(0));
        assert_eq!(drop.send_closed(), Some(ToDaemon::SendCancel { id: "o-1".into() }));
    }

    #[test]
    fn devices_sent_to_are_remembered_by_certificate() {
        let (phone, plain) = (device("phone", true), device("plain", false));
        let mut known = Vec::new();
        assert!(!is_known(&known, &phone));
        assert!(remember(&mut known, &phone));
        assert!(!remember(&mut known, &phone), "already first: nothing to write");
        assert!(is_known(&known, &phone));
        // Same name, another certificate: a stranger.
        assert!(!is_known(&known, &Device { fingerprint: "B".repeat(24), ..phone.clone() }));
        // A device that does not encrypt is never remembered and never known, whatever it claims.
        assert!(!remember(&mut known, &plain));
        assert!(!is_known(&[plain.fingerprint.clone()], &plain));
        assert!(!remember(&mut known, &Device { fingerprint: "short".into(), ..phone.clone() }));
        for i in 0..100 {
            remember(&mut known, &Device { fingerprint: format!("{i:024}"), ..phone.clone() });
        }
        assert_eq!(known.len(), KNOWN_MOST);
        assert!(!is_known(&known, &phone), "the oldest fall out");
        assert!(is_fingerprint(&"A".repeat(64)) && !is_fingerprint("../../etc/passwd") && !is_fingerprint(""));
    }

    #[test]
    fn only_roots_own_request_sends_to_a_known_device_at_one_click() {
        let (phone, plain) = (device("phone", true), device("plain", false));
        let mut drop = picker(vec![phone.clone(), plain.clone()]);
        drop.send_click(SendHit::Row(0));
        assert_eq!(drop.send_click(SendHit::Go), (Some(send_to(&phone)), true));
        // Declined: not remembered.
        assert!(drop.apply(FromDaemon::SendDone { id: "o-1".into(), to: "phone".into(), outcome: SendOutcome::Declined, sent: 0 }));
        assert!(drop.known.is_empty() && drop.send.is_none());
        assert_eq!(drop.notice, Some(Notice::SendResult { to: "phone".into(), outcome: SendOutcome::Declined, sent: 0 }));
        // (Marking a device known writes the list; here it is put there directly.)
        drop.known = vec![phone.fingerprint.clone(), plain.fingerprint.clone()];
        // A program of the desktop user asks: known or not, the device is confirmed first.
        drop.apply(request(1, 1, false));
        drop.apply(FromDaemon::SendDevices { id: "o-1".into(), searching: false, list: vec![phone.clone(), plain.clone()] });
        assert_eq!(drop.send_click(SendHit::Row(0)), (None, false));
        assert_eq!(drop.send.as_ref().unwrap().view, SendView::Confirm(phone.clone()));
        assert_eq!(drop.send_click(SendHit::Go), (Some(send_to(&phone)), true));
        drop.apply(FromDaemon::SendDone { id: "o-1".into(), to: "phone".into(), outcome: SendOutcome::Cancelled, sent: 0 });
        // Root's own request: one click for the known device, still two for one that does not encrypt.
        drop.apply(request(1, 1, true));
        drop.apply(FromDaemon::SendDevices { id: "o-1".into(), searching: false, list: vec![phone.clone(), plain.clone()] });
        assert_eq!(drop.send_click(SendHit::Row(1)), (None, false), "unencrypted: asked every time");
        assert_eq!(drop.send.as_ref().unwrap().view, SendView::Confirm(plain));
        drop.send_click(SendHit::Back);
        assert_eq!(drop.send_click(SendHit::Row(0)), (Some(send_to(&phone)), true));
    }

    #[test]
    fn presses_are_ignored_right_after_the_picker_appears_or_changes() {
        let start = Instant::now();
        let mut send = SendUi::new("o-1".into(), 1, 1, 1, vec!["a".into()], false, start);
        let (row, go) = (Some(SendHit::Row(0)), Some(SendHit::Go));
        // A click that was already on its way when the picker came up: nothing.
        send.press(row, start + Duration::from_millis(100));
        assert_eq!(send.release(row, start + Duration::from_millis(180)), None);
        // Pressed during the quiet time, held, released after it: still nothing.
        send.press(row, start + GUARD - Duration::from_millis(1));
        assert_eq!(send.release(row, start + GUARD + Duration::from_secs(2)), None);
        // After it, a press and release on the same row is a click.
        let later = start + GUARD;
        assert!(send.quiet(later - Duration::from_millis(1)) && !send.quiet(later));
        send.press(row, later);
        assert_eq!(send.release(row, later + Duration::from_millis(90)), row);
        assert_eq!(send.release(row, later + Duration::from_millis(95)), None, "one press, one click");
        // Pressed on one thing, released on another, or on nothing: no click.
        send.press(row, later);
        assert_eq!(send.release(go, later), None);
        send.press(row, later);
        assert_eq!(send.release(None, later), None);
        send.press(None, later);
        assert_eq!(send.release(row, later), None);
        send.press(row, later);
        assert_eq!(send.release(Some(SendHit::Row(1)), later), None);
        // The picker changes between press and release (a list arrives, files change): the
        // press is forgotten, and the quiet time starts again.
        send.press(row, later);
        send.hush(later + Duration::from_millis(50));
        assert_eq!(send.release(row, later + Duration::from_millis(60)), None);
        send.press(go, later + Duration::from_millis(400));
        assert_eq!(send.release(go, later + Duration::from_millis(450)), None);
        send.press(go, later + Duration::from_millis(50) + GUARD);
        assert_eq!(send.release(go, later + Duration::from_millis(60) + GUARD), go);
    }

    #[test]
    fn what_changes_the_picker_starts_the_quiet_time_again() {
        let phone = device("phone", true);
        let mut drop = picker(vec![phone.clone()]);
        let rest = |drop: &mut DropUi| {
            let send = drop.send.as_mut().unwrap();
            assert!(send.quiet(Instant::now()), "quiet again");
            send.quiet_until = Instant::now() - Duration::from_millis(1);
        };
        rest(&mut drop);
        // The same list again changes nothing; another one does.
        drop.apply(FromDaemon::SendDevices { id: "o-1".into(), searching: true, list: vec![phone.clone()] });
        assert!(!drop.send.as_ref().unwrap().quiet(Instant::now()));
        drop.apply(FromDaemon::SendDevices { id: "o-1".into(), searching: false, list: vec![device("mac", true), phone.clone()] });
        rest(&mut drop);
        // The owner's own step to the confirmation (the send button appears), and back.
        drop.send_click(SendHit::Row(1));
        rest(&mut drop);
        drop.send_click(SendHit::Back);
        rest(&mut drop);
        // Other files.
        drop.apply(request(2, 3, false));
        rest(&mut drop);
        // The device asks for a PIN.
        drop.send_click(SendHit::Row(1));
        drop.send.as_mut().unwrap().revision = 2;
        drop.send_click(SendHit::Go);
        drop.apply(FromDaemon::SendPinWanted { id: "o-1".into(), wrong: false });
        rest(&mut drop);
    }

    #[test]
    fn the_choice_is_for_the_files_that_were_shown() {
        let phone = device("phone", true);
        let mut drop = picker(vec![phone.clone()]);
        assert_eq!(drop.touch(), Some(ToDaemon::SendTouch { id: "o-1".into() }), "the first click tells the daemon to close the set");
        assert_eq!(drop.touch(), None);
        drop.send_click(SendHit::Row(0));
        // The files change while the owner is at the confirmation: back to the list, nothing chosen.
        drop.wish = None;
        drop.apply(request(2, 3, false));
        let send = drop.send.as_ref().unwrap();
        assert_eq!((&send.view, send.revision, send.count, send.names.len()), (&SendView::List, 2, 3, 3));
        assert_eq!(drop.send_click(SendHit::Go), (None, false));
        // The choice names the revision that is on the screen.
        drop.send_click(SendHit::Row(0));
        let sent = drop.send_click(SendHit::Go);
        assert_eq!(sent, (Some(ToDaemon::SendTo { id: "o-1".into(), revision: 2, fingerprint: phone.fingerprint.clone(), host: phone.host.clone(), port: phone.port }), true));
        assert!(drop.send.as_ref().unwrap().committed);
        // The daemon did not take it (the set had changed once more) and announces the request
        // again: the picker comes back, undecided, and closing it withdraws the request.
        drop.wish = None;
        drop.apply(request(3, 4, false));
        let send = drop.send.as_ref().unwrap();
        assert_eq!((drop.wish, &send.view, send.committed, send.target.clone(), send.revision), (Some(Wish::Open), &SendView::List, false, None, 3));
        assert_eq!(drop.send_closed(), Some(ToDaemon::SendCancel { id: "o-1".into() }));
        // The same when it comes back with the number the shell already had.
        let mut drop = picker(vec![phone.clone()]);
        drop.send_click(SendHit::Row(0));
        drop.send_click(SendHit::Go);
        drop.wish = None;
        drop.apply(request(1, 2, false));
        assert_eq!((drop.wish, drop.send.as_ref().unwrap().committed), (Some(Wish::Open), false));
    }

    #[test]
    fn every_file_is_named_or_counted() {
        let lines = |count: u32| {
            let names = (0..count.min(8)).map(|i| i.to_string()).collect();
            SendUi::new("o-1".into(), 1, count, 1, names, false, Instant::now()).name_lines()
        };
        assert_eq!(lines(1), (1, 0));
        assert_eq!(lines(NAME_LINES as u32), (NAME_LINES, 0));
        assert_eq!(lines(NAME_LINES as u32 + 1), (NAME_LINES - 1, 2));
        assert_eq!(lines(500), (NAME_LINES - 1, 500 - (NAME_LINES as u32 - 1)));
        // Fewer names than the count says (a daemon that sends too few): the rest is counted.
        assert_eq!(SendUi::new("o-1".into(), 1, 4, 1, vec!["a".into()], false, Instant::now()).name_lines(), (1, 3));
    }

    #[test]
    fn the_list_follows_the_daemon() {
        let phone = device("phone", true);
        let mut drop = picker(vec![phone.clone()]);
        // Lists for another request are not this picker's.
        drop.apply(FromDaemon::SendDevices { id: "o-9".into(), searching: true, list: vec![] });
        assert_eq!(drop.send.as_ref().unwrap().devices.len(), 1);
        // The device being confirmed disappears: back to the list, it cannot be confirmed.
        drop.send_click(SendHit::Row(0));
        drop.apply(FromDaemon::SendDevices { id: "o-1".into(), searching: false, list: vec![] });
        assert_eq!(drop.send.as_ref().unwrap().view, SendView::List);
        assert_eq!(drop.send_click(SendHit::Go), (None, false));
        // The same request announced again, unchanged: nothing starts over.
        drop.wish = None;
        drop.apply(request(1, 2, false));
        assert_eq!((drop.wish.take(), drop.send.as_ref().unwrap().count), (Some(Wish::Redraw), 2));
        // Searching again is asked for once.
        assert_eq!(drop.send_click(SendHit::Refresh), (Some(ToDaemon::SendRefresh { id: "o-1".into() }), false));
        assert_eq!(drop.send_click(SendHit::Refresh), (None, false));
        // The requester went away (the daemon says "cancelled"): the picker closes without a notice.
        assert!(!drop.apply(FromDaemon::SendDone { id: "o-1".into(), to: String::new(), outcome: SendOutcome::Cancelled, sent: 0 }));
        assert_eq!((drop.wish.take(), drop.send.is_none(), drop.notice.is_none()), (Some(Wish::Close), true, true));
    }

    #[test]
    fn waiting_progress_and_result() {
        let phone = device("phone", true);
        let mut drop = picker(vec![phone.clone()]);
        drop.send_click(SendHit::Row(0));
        drop.send_click(SendHit::Go);
        assert!(drop.apply(FromDaemon::SendProgress { id: "o-1".into(), to: "phone".into(), file: 0, count: 2, done: 0, total: 30 }));
        assert!(drop.until.is_none());
        assert!(drop.apply(FromDaemon::SendProgress { id: "o-1".into(), to: "phone".into(), file: 1, count: 2, done: 50, total: 30 }));
        assert_eq!(drop.notice, Some(Notice::Sending { id: "o-1".into(), to: "phone".into(), file: 1, count: 2, done: 30, total: 30 }));
        // Its button cancels the send; the cross does nothing while it runs.
        assert_eq!(drop.click(NoticeHit::Main), (Some(ToDaemon::SendCancel { id: "o-1".into() }), false, false));
        assert_eq!(drop.click(NoticeHit::Dismiss), (None, false, false));
        // An offer that waits for its answer is not covered by progress.
        drop.apply(offer("s1"));
        assert!(!drop.apply(FromDaemon::SendProgress { id: "o-1".into(), to: "phone".into(), file: 2, count: 2, done: 20, total: 30 }));
        assert!(matches!(drop.notice, Some(Notice::Offer { .. })));
        drop.apply(FromDaemon::OfferGone { id: "s1".into() });
        // Sent: the result is shown and the device is known from now on.
        assert!(drop.apply(FromDaemon::SendDone { id: "o-1".into(), to: "phone".into(), outcome: SendOutcome::Sent, sent: 2 }));
        assert_eq!(drop.notice, Some(Notice::SendResult { to: "phone".into(), outcome: SendOutcome::Sent, sent: 2 }));
        assert!(drop.until.is_some() && is_known(&drop.known, &phone));
        assert_eq!(drop.click(NoticeHit::Main), (None, false, true));
        // Cancelled while it ran: the card goes, nothing is announced, nothing is remembered.
        let tablet = device("tablet", true);
        let mut drop = picker(vec![tablet.clone()]);
        drop.send_click(SendHit::Row(0));
        drop.send_click(SendHit::Go);
        drop.apply(FromDaemon::SendProgress { id: "o-1".into(), to: "tablet".into(), file: 1, count: 2, done: 5, total: 30 });
        assert!(drop.apply(FromDaemon::SendDone { id: "o-1".into(), to: "tablet".into(), outcome: SendOutcome::Cancelled, sent: 0 }));
        assert!(drop.notice.is_none() && drop.known.is_empty());
        // The daemon dying takes picker and card with it.
        let mut drop = picker(vec![tablet]);
        drop.apply(FromDaemon::SendProgress { id: "o-1".into(), to: "tablet".into(), file: 1, count: 2, done: 5, total: 30 });
        drop.went_off(false);
        assert_eq!((drop.wish.take(), drop.send.is_none(), drop.notice.is_none()), (Some(Wish::Close), true, true));
    }

    #[test]
    fn a_pin_is_typed_in_the_picker() {
        let phone = device("phone", true);
        let mut drop = picker(vec![phone.clone()]);
        drop.send_click(SendHit::Row(0));
        drop.send_click(SendHit::Go);
        drop.apply(FromDaemon::SendProgress { id: "o-1".into(), to: "phone".into(), file: 0, count: 2, done: 0, total: 30 });
        drop.wish = None;
        // The device wants a PIN: the picker comes back, and closing it now withdraws the request.
        assert!(drop.apply(FromDaemon::SendPinWanted { id: "o-1".into(), wrong: false }));
        assert_eq!(drop.wish.take(), Some(Wish::Open));
        assert!(drop.notice.is_none());
        let send = drop.send.as_ref().unwrap();
        assert_eq!((&send.view, send.committed, send.target.as_ref()), (&SendView::Pin { wrong: false }, false, Some(&phone)));
        assert_eq!(drop.send_click(SendHit::Go), (None, false), "an empty PIN is not sent");
        drop.send.as_mut().unwrap().pin = "4821".into();
        assert_eq!(drop.send_click(SendHit::Go), (Some(ToDaemon::SendPin { id: "o-1".into(), pin: "4821".into() }), true));
        assert_eq!(drop.send_closed(), None);
        drop.apply(FromDaemon::SendPinWanted { id: "o-1".into(), wrong: true });
        assert_eq!(drop.send.as_ref().unwrap().view, SendView::Pin { wrong: true });
        assert!(drop.send.as_ref().unwrap().pin.is_empty());
        assert_eq!(drop.send_closed(), Some(ToDaemon::SendCancel { id: "o-1".into() }));
    }

    #[test]
    fn a_demo_picker_sends_nothing() {
        let mut drop = DropUi::default();
        let mut send = SendUi::new("demo".into(), 1, 1, 1, vec!["x".into()], false, Instant::now());
        (send.demo, send.devices) = (true, vec![device("phone", true)]);
        drop.send = Some(send);
        assert_eq!(drop.send_click(SendHit::Row(0)), (None, true));
        assert_eq!(drop.send_click(SendHit::Go), (None, true));
        assert_eq!(drop.send_closed(), None);
        assert!(drop.send.is_none());
    }

    #[test]
    fn the_switch_shows_receiving_not_running() {
        let mut drop = DropUi::default();
        // A daemon that only runs to send: the switch stays off.
        drop.apply(FromDaemon::State { alias: "水滴".into(), address: Some("192.0.2.33".into()), quick: false, receiving: false });
        assert!(!drop.on);
        assert!(matches!(tile_request(drop.on, drop.starting), Some(Request::Start)));
        drop.apply(FromDaemon::State { alias: "水滴".into(), address: Some("192.0.2.33".into()), quick: false, receiving: true });
        assert!(drop.on);
        // Switched off while an offer waits: the offer goes.
        drop.apply(offer("s1"));
        assert!(drop.apply(FromDaemon::State { alias: "水滴".into(), address: None, quick: false, receiving: false }));
        assert!(!drop.on && drop.notice.is_none());
    }
}
