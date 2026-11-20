//! Content meter (phase 1: measurement only — nothing acts on it yet).
//!
//! What is on the screen shows in two numbers that cost nothing to obtain: when each decoded
//! frame arrives and how many encoded bytes came in since the one before. The sender only sends
//! frames when the picture changes (plus a few tiny repeats of a still picture), so:
//!
//!   idle        hardly any frames at all;
//!   text-like   short bursts (typing, scrolling, the pointer) separated by pauses, each burst
//!               ending in tiny refinement frames;
//!   video-like  an unbroken run of frames at a steady rate, most of them of real size.
//!
//! No pixel is looked at. The byte count per frame is approximate: network reads do not line up
//! with frames, so a frame can be credited with part of its neighbour's bytes; sums are exact.
use std::fmt;

/// Frames closer together than this belong to the same run of activity.
const RUN_GAP_US: u64 = 50_000;
const TINY: u64 = 1_000;
const SMALL: u64 = 16_000;
const MEDIUM: u64 = 128_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind { Idle, TextLike, VideoLike }

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub kind: Kind,
    pub seconds: f64,
    pub frames: usize,
    /// Share of the window spent inside runs of frames, in percent.
    pub active_percent: u32,
    pub bursts: u32,
    pub longest_run_ms: u64,
    pub bytes: u64,
    pub p50: u64,
    pub p95: u64,
    pub max: u64,
    /// Frames under 1 kB, under 16 kB, under 128 kB, and larger.
    pub sizes: [u32; 4],
}

impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let kind = match self.kind { Kind::Idle => "idle", Kind::TextLike => "text-like", Kind::VideoLike => "video-like" };
        write!(f, "content: {kind} | {} frames in {:.1} s, {}% active, {} burst(s), longest run {:.1} s | frame bytes p50 {} p95 {} max {} | <1k {}, <16k {}, <128k {}, larger {} | {:.2} Mbit/s",
            self.frames, self.seconds, self.active_percent, self.bursts, self.longest_run_ms as f64 / 1000.0,
            self.p50, self.p95, self.max, self.sizes[0], self.sizes[1], self.sizes[2], self.sizes[3],
            if self.seconds > 0.0 { self.bytes as f64 * 8.0 / 1e6 / self.seconds } else { 0.0 })
    }
}

/// Collects one window's worth of frames. At 60 frames a second and a 10 s window that is 600
/// numbers; the cap only matters if nobody asks for the summary.
#[derive(Default)]
pub struct Meter {
    sizes: Vec<u64>,
    active_us: u64,
    run_us: u64,
    longest_run_us: u64,
    bursts: u32,
}

impl Meter {
    const CAP: usize = 4096;

    /// One decoded frame: the time since the previous one and the bytes received in between.
    pub fn frame(&mut self, gap_us: u64, bytes: u64) {
        if self.sizes.len() < Self::CAP { self.sizes.push(bytes); }
        if gap_us <= RUN_GAP_US && self.bursts > 0 {
            self.active_us += gap_us;
            self.run_us += gap_us;
        } else {
            self.bursts += 1;
            self.run_us = 0;
        }
        self.longest_run_us = self.longest_run_us.max(self.run_us);
    }

    /// Sums the window up and starts the next one.
    pub fn take(&mut self, seconds: f64) -> Summary {
        let mut sizes = std::mem::take(&mut self.sizes);
        sizes.sort_unstable();
        let frames = sizes.len();
        let at = |q: usize| if frames == 0 { 0 } else { sizes[(frames - 1) * q / 100] };
        let mut buckets = [0u32; 4];
        for &s in &sizes {
            buckets[if s < TINY { 0 } else if s < SMALL { 1 } else if s < MEDIUM { 2 } else { 3 }] += 1;
        }
        let window_us = (seconds * 1e6).max(1.0);
        let active_percent = ((self.active_us as f64 * 100.0 / window_us).round() as u32).min(100);
        let kind = if frames == 0 || active_percent < 3 {
            Kind::Idle
        } else if active_percent >= 60 && self.longest_run_us >= 3_000_000 && (buckets[0] as usize) * 2 < frames {
            Kind::VideoLike
        } else {
            Kind::TextLike
        };
        let summary = Summary {
            kind, seconds, frames, active_percent, bursts: self.bursts, longest_run_ms: self.longest_run_us / 1000,
            bytes: sizes.iter().sum(), p50: at(50), p95: at(95), max: sizes.last().copied().unwrap_or(0), sizes: buckets,
        };
        *self = Meter::default();
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_arriving_is_idle() {
        let s = Meter::default().take(10.0);
        assert_eq!(s.kind, Kind::Idle);
        assert_eq!((s.frames, s.bursts, s.bytes, s.p50, s.max), (0, 0, 0, 0, 0));
    }

    #[test]
    fn a_blinking_cursor_is_idle() {
        let mut m = Meter::default();
        for _ in 0..20 { m.frame(500_000, 300); } // two tiny frames a second
        let s = m.take(10.0);
        assert_eq!(s.kind, Kind::Idle);
        assert_eq!(s.bursts, 20);
        assert_eq!(s.sizes, [20, 0, 0, 0]);
    }

    #[test]
    fn steady_full_frames_are_video_like() {
        let mut m = Meter::default();
        for _ in 0..600 { m.frame(16_667, 40_000); }
        let s = m.take(10.0);
        assert_eq!(s.kind, Kind::VideoLike);
        assert_eq!(s.frames, 600);
        assert_eq!(s.bursts, 1);
        assert!(s.active_percent >= 99 && s.longest_run_ms > 9_900);
        assert_eq!(s.bytes, 24_000_000);
        assert_eq!((s.p50, s.p95, s.max), (40_000, 40_000, 40_000));
        assert_eq!(s.sizes, [0, 0, 600, 0]);
    }

    #[test]
    fn typing_is_text_like() {
        let mut m = Meter::default();
        // Eight keystrokes: one changed frame, then ten small refinement repeats at 30 a second.
        for _ in 0..8 {
            m.frame(900_000, 6_000);
            for _ in 0..10 { m.frame(33_000, 400); }
        }
        let s = m.take(10.0);
        assert_eq!(s.kind, Kind::TextLike);
        assert_eq!(s.bursts, 8);
        assert_eq!(s.active_percent, 26);
        assert_eq!(s.longest_run_ms, 330);
    }

    #[test]
    fn a_still_picture_being_refined_is_not_video() {
        let mut m = Meter::default();
        for _ in 0..300 { m.frame(33_000, 200); } // long run, but of empty frames
        assert_eq!(m.take(10.0).kind, Kind::TextLike);
    }

    #[test]
    fn taking_the_summary_starts_a_fresh_window() {
        let mut m = Meter::default();
        for _ in 0..100 { m.frame(16_000, 50_000); }
        assert_eq!(m.take(10.0).frames, 100);
        assert_eq!(m.take(10.0).frames, 0);
    }

    #[test]
    fn the_line_is_one_line() {
        let mut m = Meter::default();
        for _ in 0..600 { m.frame(16_667, 200_000); }
        let line = m.take(10.0).to_string();
        assert!(line.starts_with("content: video-like | 600 frames in 10.0 s"), "{line}");
        assert!(line.ends_with("96.00 Mbit/s"), "{line}");
        assert!(!line.contains('\n'));
    }
}
