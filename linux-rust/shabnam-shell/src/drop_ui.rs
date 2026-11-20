//! 隔空传送 in the shell: the switch in the quick panel, and the notice card above the shelf's
//! right end that asks before a transfer is accepted and tells when files have arrived.
//! The receiver itself is another program (see drop.rs); this file only shows what it reports
//! and sends back the owner's clicks. Its texts are in lang.rs with all the others.

use super::{inside, QuickHit, QuickLayout, Rect, Ui, ACCENT, CHIP, HOVER, ON_ACCENT, PANEL, TEXT, TEXT2, WARN};
use crate::draw::{argb, Canvas};
use crate::drop::ipc::{FromDaemon, Refusal, ToDaemon};
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
}

impl Notice {
    /// How long it stays without a click; None: until something happens.
    fn lifetime(&self) -> Option<Duration> {
        match self {
            Notice::Offer { .. } | Notice::Progress { .. } => None,
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
            FromDaemon::State { alias, address, quick } => {
                (self.on, self.starting, self.alias, self.address, self.quick) = (true, false, alias, address, quick);
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
        }
        true
    }

    /// The receiver is gone: nothing can be answered any more.
    fn went_off(&mut self, failed: bool) {
        (self.on, self.starting, self.quick, self.address) = (false, false, false, None);
        if matches!(self.notice, Some(Notice::Offer { .. } | Notice::Progress { .. })) {
            self.notice = None;
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
    }

    pub fn notice_wanted(&self) -> bool {
        self.drop.notice.is_some()
    }

    /// News from the worker thread.
    pub fn drop_event(&mut self, event: Event) {
        let changed = match event {
            Event::Message(message) => self.drop.apply(message),
            Event::Off { failed } => {
                self.drop.went_off(failed);
                true
            }
        };
        self.dirty_notice |= changed;
        self.dirty_panel |= self.open == Some(super::Panel::Quick);
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
            Some(Notice::Progress { .. }) => (Some(Msg::Cancel), None, false),
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
        assert!(!drop.apply(FromDaemon::State { alias: "水滴".into(), address: Some("192.0.2.33".into()), quick: true }));
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
}
