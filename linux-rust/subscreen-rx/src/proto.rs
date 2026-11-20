//! Wire format of the control channel (video port + 2), see docs/PROTOCOL.md §6.
//!
//! Every message is `type u8 | length u8 | payload[length]`. Nothing here executes anything: a
//! message can only ever become one of the fixed `Command`s below, carrying a clamped integer.
use std::path::Path;
use std::time::{Duration, Instant};

/// Revision 2 added: become second screen, screen off/on, power off/restart, and the status
/// flags "own desktop" and "screen off".
pub const PROTOCOL_VERSION: u8 = 2;
/// A longer payload than this is a protocol violation; the connection is closed.
pub const MAX_PAYLOAD: usize = 32;
pub const VOLUME_STEPS: u8 = 16;

pub const SET_BRIGHTNESS: u8 = 0x01;
pub const SET_VOLUME: u8 = 0x02;
pub const SET_MUTE: u8 = 0x03;
pub const LEAVE: u8 = 0x04;
pub const GET_STATUS: u8 = 0x05;
pub const BECOME_SECOND_SCREEN: u8 = 0x06;
pub const SET_SCREEN: u8 = 0x07;
pub const END_SESSION: u8 = 0x08;
pub const STATUS: u8 = 0x81;

/// Everything the sender can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Panel backlight, 0–100 percent.
    SetBrightness(u8),
    /// Speaker volume, 0–`VOLUME_STEPS`.
    SetVolume(u8),
    SetMute(bool),
    /// Leave second-screen mode and return to the laptop's own desktop. Ignored by the
    /// control-only receiver, which runs while the laptop already is on its own desktop.
    Leave,
    GetStatus,
    /// Leave the laptop's own desktop and become the sender's second screen (what Win+Esc does
    /// on the desktop). Ignored in second-screen mode.
    BecomeSecondScreen,
    /// Panel backlight power: true = on. Brightness is left as it is.
    SetScreen(bool),
    /// Switch the laptop off or restart it.
    EndSession(End),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    PowerOff,
    Restart,
}

impl End {
    /// The word the session script acts on (the same ones the quick panel's buttons leave).
    pub fn word(self) -> &'static str {
        match self { End::PowerOff => "poweroff", End::Restart => "reboot" }
    }
}

/// One message → one command. Unknown types, short payloads and out-of-range switches give None
/// and are skipped; numbers are clamped; payload bytes beyond the ones defined are ignored.
pub fn decode(kind: u8, payload: &[u8]) -> Option<Command> {
    match (kind, payload.first().copied()) {
        (SET_BRIGHTNESS, Some(v)) => Some(Command::SetBrightness(v.min(100))),
        (SET_VOLUME, Some(v)) => Some(Command::SetVolume(v.min(VOLUME_STEPS))),
        (SET_MUTE, Some(0)) => Some(Command::SetMute(false)),
        (SET_MUTE, Some(1)) => Some(Command::SetMute(true)),
        (LEAVE, _) => Some(Command::Leave),
        (GET_STATUS, _) => Some(Command::GetStatus),
        (BECOME_SECOND_SCREEN, _) => Some(Command::BecomeSecondScreen),
        (SET_SCREEN, Some(0)) => Some(Command::SetScreen(false)),
        (SET_SCREEN, Some(1)) => Some(Command::SetScreen(true)),
        (END_SESSION, Some(1)) => Some(Command::EndSession(End::PowerOff)),
        (END_SESSION, Some(2)) => Some(Command::EndSession(End::Restart)),
        _ => None,
    }
}

/// What is left of one batch of commands (everything one read from the socket produced) once
/// only the newest of each kind is kept: at most one brightness, one volume, one mute, one
/// screen switch, one status request and one of each mode change, in that order. A slider dragged on the sender produces a burst
/// of values; only the last one matters, and each may cost a program start on the desktop.
pub fn coalesce(commands: &[Command]) -> Vec<Command> {
    let (mut brightness, mut volume, mut mute, mut screen, mut status) = (None, None, None, None, None);
    let (mut leave, mut second, mut end) = (None, None, None);
    for &c in commands {
        match c {
            Command::SetBrightness(_) => brightness = Some(c),
            Command::SetVolume(_) => volume = Some(c),
            Command::SetMute(_) => mute = Some(c),
            Command::GetStatus => status = Some(c),
            Command::Leave => leave = Some(c),
            Command::SetScreen(_) => screen = Some(c),
            Command::BecomeSecondScreen => second = Some(c),
            Command::EndSession(_) => end = Some(c),
        }
    }
    [brightness, volume, mute, screen, status, leave, second, end].into_iter().flatten().collect()
}

/// Lets an action through at most once per `gap`, so a sender that is stuck repeating a command
/// cannot make the laptop loop through it. What is refused is dropped, not queued.
pub struct RateLimit {
    gap: Duration,
    last: Option<Instant>,
}

impl RateLimit {
    pub const fn new(gap: Duration) -> Self { RateLimit { gap, last: None } }

    pub fn allow(&mut self, now: Instant) -> bool {
        if self.last.is_some_and(|last| now.saturating_duration_since(last) < self.gap) { return false; }
        self.last = Some(now);
        true
    }
}

/// The peer announced a payload longer than `MAX_PAYLOAD`.
#[derive(Debug, PartialEq, Eq)]
pub struct Oversize;

/// Cuts the byte stream into messages. Holds at most one incomplete message (2 + 32 bytes).
#[derive(Default)]
pub struct Parser {
    pending: Vec<u8>,
}

impl Parser {
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Command>, Oversize> {
        let mut out = Vec::new();
        for &b in bytes {
            self.pending.push(b);
            if self.pending.len() < 2 { continue; }
            let len = self.pending[1] as usize;
            if len > MAX_PAYLOAD { return Err(Oversize); }
            if self.pending.len() == 2 + len {
                if let Some(c) = decode(self.pending[0], &self.pending[2..]) { out.push(c); }
                self.pending.clear();
            }
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Power {
    /// Percent, when the machine has a battery that reports one.
    pub battery: Option<u8>,
    pub charging: bool,
    /// Running from the charger (charging or already full).
    pub external: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Status {
    pub brightness: u8,
    pub volume: u8,
    pub muted: bool,
    pub power: Power,
    /// None until the lid sensor has reported once.
    pub lid_closed: Option<bool>,
    /// The laptop is on its own desktop (control-only receiver): there is no second-screen mode
    /// to leave.
    pub own_desktop: bool,
    /// The panel's backlight is switched off (by the lid, or by the sender's request).
    pub screen_off: bool,
    pub version: (u8, u8, u8),
}

impl Status {
    /// The complete STATUS message, header included.
    pub fn encode(&self) -> [u8; 11] {
        let flags = (self.muted as u8)
            | (self.power.battery.is_some() as u8) << 1
            | (self.power.charging as u8) << 2
            | (self.power.external as u8) << 3
            | (self.lid_closed.is_some() as u8) << 4
            | (self.lid_closed.unwrap_or(false) as u8) << 5
            | (self.own_desktop as u8) << 6
            | (self.screen_off as u8) << 7;
        [
            STATUS, 9, PROTOCOL_VERSION,
            self.brightness.min(100), self.volume.min(VOLUME_STEPS), VOLUME_STEPS, flags,
            self.power.battery.unwrap_or(0).min(100),
            self.version.0, self.version.1, self.version.2,
        ]
    }
}

/// Backlight value → percent, rounded (the same arithmetic as the shabnam-brightness script).
pub fn brightness_percent(value: i64, max: i64) -> u8 {
    if max <= 0 { return 0; }
    ((value.clamp(0, max) * 100 + max / 2) / max) as u8
}

/// Percent → backlight value. Never below 4 % of the range: a black panel with no way to see
/// why is worse than a dim one (the same floor as the shabnam-brightness script).
pub fn brightness_value(percent: u8, max: i64) -> i64 {
    if max <= 0 { return 0; }
    let floor = (max * 4 / 100).max(1);
    ((percent.min(100) as i64 * max + 50) / 100).clamp(floor, max)
}

/// Volume step (0–`VOLUME_STEPS`) → percent of the desktop's sound output, rounded.
pub fn volume_percent(step: u8) -> u8 {
    let steps = VOLUME_STEPS as u32;
    ((step.min(VOLUME_STEPS) as u32 * 100 + steps / 2) / steps) as u8
}

/// Percent of the desktop's sound output → the nearest volume step. Above 100 % is the top step.
pub fn volume_step(percent: u32) -> u8 {
    ((percent.min(100) * VOLUME_STEPS as u32 + 50) / 100) as u8
}

/// What `wpctl get-volume` prints — "Volume: 0.45" or "Volume: 0.45 [MUTED]" — as percent and
/// mute. None for anything else (sound is not running).
pub fn parse_sink_volume(text: &str) -> Option<(u32, bool)> {
    let mut words = text.split_whitespace();
    if words.next()? != "Volume:" { return None; }
    let level: f64 = words.next()?.parse().ok()?;
    if !level.is_finite() || level < 0.0 { return None; }
    Some(((level.min(10.0) * 100.0).round() as u32, words.next() == Some("[MUTED]")))
}

/// The lid sensor's line in the kernel's GPIO listing (/sys/kernel/debug/gpio):
/// " gpio-30  (  |hall_mh248  ) in  hi IRQ" — high is open, low is closed, as the shabnam-lid
/// script reads it. None if the listing has no such line.
pub fn parse_lid(gpio_listing: &str) -> Option<bool> {
    let line = gpio_listing.lines().find(|l| l.contains("hall_mh248"))?;
    let after = line.rsplit(')').next()?;
    after.split_whitespace().find_map(|w| match w { "lo" => Some(true), "hi" => Some(false), _ => None })
}

/// Reads battery level and charger state from a directory laid out like /sys/class/power_supply.
pub fn read_power(dir: &Path) -> Power {
    let text = |p: std::path::PathBuf| std::fs::read_to_string(p).map(|s| s.trim().to_owned()).unwrap_or_default();
    let mut power = Power::default();
    let Ok(entries) = std::fs::read_dir(dir) else { return power };
    for e in entries.flatten() {
        let p = e.path();
        if text(p.join("type")) == "Battery" {
            // "Device" marks the battery of something plugged in (a mouse), not our own.
            if text(p.join("scope")) == "Device" { continue; }
            if let Ok(c) = text(p.join("capacity")).parse::<i64>() {
                power.battery = Some(c.clamp(0, 100) as u8);
            }
            match text(p.join("status")).as_str() {
                "Charging" => { power.charging = true; power.external = true; }
                "Full" | "Not charging" => power.external = true,
                _ => {}
            }
        } else if text(p.join("online")) == "1" {
            power.external = true;
        }
    }
    power
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_fixed_commands() {
        assert_eq!(decode(SET_BRIGHTNESS, &[40]), Some(Command::SetBrightness(40)));
        assert_eq!(decode(SET_VOLUME, &[7]), Some(Command::SetVolume(7)));
        assert_eq!(decode(SET_MUTE, &[1]), Some(Command::SetMute(true)));
        assert_eq!(decode(SET_MUTE, &[0]), Some(Command::SetMute(false)));
        assert_eq!(decode(LEAVE, &[]), Some(Command::Leave));
        assert_eq!(decode(GET_STATUS, &[]), Some(Command::GetStatus));
        assert_eq!(decode(BECOME_SECOND_SCREEN, &[]), Some(Command::BecomeSecondScreen));
        assert_eq!(decode(SET_SCREEN, &[0]), Some(Command::SetScreen(false)));
        assert_eq!(decode(SET_SCREEN, &[1]), Some(Command::SetScreen(true)));
        assert_eq!(decode(END_SESSION, &[1]), Some(Command::EndSession(End::PowerOff)));
        assert_eq!(decode(END_SESSION, &[2]), Some(Command::EndSession(End::Restart)));
    }

    /// The bytes the Mac app sends (ControlProtocol.Command.encoded) for the revision-2 commands.
    #[test]
    fn revision_two_commands_from_the_wire() {
        let mut p = Parser::default();
        let got = p.feed(&[0x06, 0, 0x07, 1, 0, 0x07, 1, 1, 0x08, 1, 1, 0x08, 1, 2]);
        assert_eq!(got, Ok(vec![
            Command::BecomeSecondScreen, Command::SetScreen(false), Command::SetScreen(true),
            Command::EndSession(End::PowerOff), Command::EndSession(End::Restart),
        ]));
        assert_eq!(End::PowerOff.word(), "poweroff");
        assert_eq!(End::Restart.word(), "reboot");
    }

    #[test]
    fn power_commands_need_their_exact_value() {
        assert_eq!(decode(END_SESSION, &[]), None); // nothing is switched off by an empty message
        assert_eq!(decode(END_SESSION, &[0]), None);
        for v in 3..=255u8 { assert_eq!(decode(END_SESSION, &[v]), None); }
        assert_eq!(decode(SET_SCREEN, &[]), None);
        assert_eq!(decode(SET_SCREEN, &[2]), None);
    }

    #[test]
    fn rate_limit_drops_repeats() {
        let mut limit = RateLimit::new(Duration::from_secs(3));
        let t0 = Instant::now();
        assert!(limit.allow(t0));
        assert!(!limit.allow(t0));
        assert!(!limit.allow(t0 + Duration::from_millis(2999)));
        assert!(limit.allow(t0 + Duration::from_secs(3)));
        assert!(!limit.allow(t0 + Duration::from_secs(4))); // counted from the last one let through
        assert!(limit.allow(t0 + Duration::from_secs(6)));
        assert!(!limit.allow(t0)); // a clock reading from before: refused, no panic
    }

    #[test]
    fn clamps_numbers() {
        assert_eq!(decode(SET_BRIGHTNESS, &[255]), Some(Command::SetBrightness(100)));
        assert_eq!(decode(SET_BRIGHTNESS, &[101]), Some(Command::SetBrightness(100)));
        assert_eq!(decode(SET_VOLUME, &[17]), Some(Command::SetVolume(16)));
        assert_eq!(decode(SET_VOLUME, &[200]), Some(Command::SetVolume(16)));
    }

    #[test]
    fn ignores_what_it_does_not_understand() {
        assert_eq!(decode(SET_BRIGHTNESS, &[]), None); // value missing
        assert_eq!(decode(SET_VOLUME, &[]), None);
        assert_eq!(decode(SET_MUTE, &[]), None);
        assert_eq!(decode(SET_MUTE, &[2]), None); // neither on nor off
        assert_eq!(decode(0x00, &[1, 2, 3]), None);
        assert_eq!(decode(0x09, &[]), None);
        assert_eq!(decode(STATUS, &[0; 9]), None); // our own message type coming back
        assert_eq!(decode(0xFF, &[0; 32]), None);
    }

    #[test]
    fn extra_payload_bytes_are_ignored() {
        assert_eq!(decode(SET_BRIGHTNESS, &[30, 9, 9]), Some(Command::SetBrightness(30)));
        assert_eq!(decode(LEAVE, &[1, 2]), Some(Command::Leave));
    }

    #[test]
    fn parser_handles_messages_split_across_reads() {
        let mut p = Parser::default();
        assert_eq!(p.feed(&[SET_BRIGHTNESS]), Ok(vec![]));
        assert_eq!(p.feed(&[1]), Ok(vec![]));
        assert_eq!(p.feed(&[55, SET_VOLUME, 1]), Ok(vec![Command::SetBrightness(55)]));
        assert_eq!(p.feed(&[3, GET_STATUS, 0, LEAVE, 0]), Ok(vec![Command::SetVolume(3), Command::GetStatus, Command::Leave]));
    }

    #[test]
    fn parser_skips_unknown_types_and_stays_in_step() {
        let mut p = Parser::default();
        let got = p.feed(&[0x70, 4, 1, 2, 3, 4, SET_MUTE, 1, 1, 0x71, 0, SET_VOLUME, 2, 5, 0]);
        assert_eq!(got, Ok(vec![Command::SetMute(true), Command::SetVolume(5)]));
    }

    #[test]
    fn parser_refuses_oversized_messages() {
        let mut p = Parser::default();
        assert_eq!(p.feed(&[SET_BRIGHTNESS, 32]).map(|v| v.len()), Ok(0)); // the largest allowed
        let mut p = Parser::default();
        assert_eq!(p.feed(&[SET_BRIGHTNESS, 33]), Err(Oversize));
        let mut p = Parser::default();
        assert_eq!(p.feed(&[0xEE, 255, 1, 2, 3]), Err(Oversize));
    }

    #[test]
    fn parser_never_yields_anything_but_valid_commands() {
        // Every possible type with every possible first payload byte.
        for kind in 0..=255u8 {
            for value in 0..=255u8 {
                let mut p = Parser::default();
                for c in p.feed(&[kind, 1, value]).unwrap() {
                    match c {
                        Command::SetBrightness(v) => assert!(v <= 100),
                        Command::SetVolume(v) => assert!(v <= VOLUME_STEPS),
                        Command::SetMute(_) | Command::Leave | Command::GetStatus => {}
                        Command::BecomeSecondScreen | Command::SetScreen(_) => {}
                        Command::EndSession(_) => assert!(kind == END_SESSION && (value == 1 || value == 2)),
                    }
                }
            }
        }
    }

    #[test]
    fn status_layout() {
        let s = Status {
            brightness: 63, volume: 9, muted: true,
            power: Power { battery: Some(82), charging: true, external: true },
            lid_closed: Some(false), own_desktop: false, screen_off: false, version: (0, 2, 0),
        };
        assert_eq!(s.encode(), [STATUS, 9, 2, 63, 9, 16, 0b01_1111, 82, 0, 2, 0]);
        let bare = Status { brightness: 250, volume: 99, ..Default::default() };
        assert_eq!(bare.encode(), [STATUS, 9, 2, 100, 16, 16, 0, 0, 0, 0, 0]);
        let closed = Status { lid_closed: Some(true), ..Default::default() };
        assert_eq!(closed.encode()[6], 0b11_0000);
        let desktop = Status { own_desktop: true, ..Default::default() };
        assert_eq!(desktop.encode()[6], 0b100_0000);
        let dark = Status { screen_off: true, ..Default::default() };
        assert_eq!(dark.encode()[6], 0b1000_0000);
    }

    #[test]
    fn coalesce_keeps_the_newest_of_each_kind() {
        use Command::*;
        assert_eq!(coalesce(&[]), vec![]);
        assert_eq!(coalesce(&[SetVolume(3), SetVolume(4), SetVolume(5)]), vec![SetVolume(5)]);
        assert_eq!(
            coalesce(&[Leave, SetMute(true), SetVolume(3), GetStatus, SetBrightness(10), SetMute(false), SetBrightness(90), GetStatus]),
            vec![SetBrightness(90), SetVolume(3), SetMute(false), GetStatus, Leave]
        );
        assert_eq!(
            coalesce(&[EndSession(End::PowerOff), SetScreen(false), BecomeSecondScreen, SetScreen(true), EndSession(End::Restart), BecomeSecondScreen]),
            vec![SetScreen(true), BecomeSecondScreen, EndSession(End::Restart)]
        );
    }

    #[test]
    fn volume_steps_and_percent_round_trip() {
        assert_eq!(volume_percent(0), 0);
        assert_eq!(volume_percent(8), 50);
        assert_eq!(volume_percent(16), 100);
        assert_eq!(volume_percent(200), 100);
        for step in 0..=VOLUME_STEPS {
            assert_eq!(volume_step(volume_percent(step) as u32), step);
        }
        // The laptop's own keys move in steps of 5 %: every value lands on a step, in order.
        let mut last = 0;
        for percent in (0..=100).step_by(5) {
            let step = volume_step(percent);
            assert!(step >= last && step <= VOLUME_STEPS);
            last = step;
        }
        assert_eq!(volume_step(45), 7);
        assert_eq!(volume_step(100), 16);
        assert_eq!(volume_step(150), 16);
        assert_eq!(volume_step(u32::MAX), 16);
    }

    #[test]
    fn reads_what_wpctl_prints() {
        assert_eq!(parse_sink_volume("Volume: 0.45\n"), Some((45, false)));
        assert_eq!(parse_sink_volume("Volume: 0.45 [MUTED]\n"), Some((45, true)));
        assert_eq!(parse_sink_volume("Volume: 1.00"), Some((100, false)));
        assert_eq!(parse_sink_volume("Volume: 1.50"), Some((150, false)));
        assert_eq!(parse_sink_volume("Volume: 0.00 [MUTED]"), Some((0, true)));
        assert_eq!(parse_sink_volume(""), None);
        assert_eq!(parse_sink_volume("Volume:"), None);
        assert_eq!(parse_sink_volume("Volume: loud"), None);
        assert_eq!(parse_sink_volume("Volume: -1"), None);
        assert_eq!(parse_sink_volume("Volume: NaN"), None);
        assert_eq!(parse_sink_volume("Could not connect to PipeWire"), None);
        assert_eq!(parse_sink_volume("Volume: 1e300").map(|v| v.0), Some(1000));
    }

    #[test]
    fn reads_the_lid_sensor_line() {
        let open = "gpiochip0: GPIOs 0-31, parent: platform/fdd60000.gpio, gpio0:\n gpio-5   (                    |vcc5v0_host         ) out hi\n gpio-30  (                    |hall_mh248          ) in  hi IRQ\n";
        assert_eq!(parse_lid(open), Some(false));
        assert_eq!(parse_lid(&open.replace("in  hi IRQ", "in  lo IRQ")), Some(true));
        assert_eq!(parse_lid("gpio-5 ( |vcc5v0_host ) out hi\n"), None);
        assert_eq!(parse_lid(" gpio-30 ( |hall_mh248 ) in  ?? \n"), None);
        assert_eq!(parse_lid(""), None);
    }

    #[test]
    fn brightness_round_trips() {
        for max in [100, 255, 1023, 4095] {
            for pct in 4..=100u8 {
                assert_eq!(brightness_percent(brightness_value(pct, max), max), pct, "max {max}");
            }
        }
    }

    #[test]
    fn brightness_is_clamped() {
        assert_eq!(brightness_value(0, 255), 10); // the 4 % floor
        assert_eq!(brightness_value(1, 255), 10);
        assert_eq!(brightness_value(100, 255), 255);
        assert_eq!(brightness_value(255, 255), 255);
        assert_eq!(brightness_value(50, 0), 0);
        assert_eq!(brightness_value(0, 10), 1);
        assert_eq!(brightness_percent(-5, 255), 0);
        assert_eq!(brightness_percent(999, 255), 100);
        assert_eq!(brightness_percent(128, 255), 50);
        assert_eq!(brightness_percent(1, 0), 0);
    }

    fn supply(root: &Path, name: &str, files: &[(&str, &str)]) {
        let d = root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        for (f, v) in files { std::fs::write(d.join(f), format!("{v}\n")).unwrap(); }
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("subscreen-rx-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reads_battery_and_charger() {
        let d = scratch("charging");
        supply(&d, "battery", &[("type", "Battery"), ("capacity", "82"), ("status", "Charging")]);
        supply(&d, "usb", &[("type", "USB"), ("online", "1")]);
        assert_eq!(read_power(&d), Power { battery: Some(82), charging: true, external: true });
        std::fs::remove_dir_all(&d).unwrap();

        let d = scratch("discharging");
        supply(&d, "battery", &[("type", "Battery"), ("capacity", "141"), ("status", "Discharging")]);
        supply(&d, "ac", &[("type", "Mains"), ("online", "0")]);
        supply(&d, "mouse", &[("type", "Battery"), ("scope", "Device"), ("capacity", "5"), ("status", "Charging")]);
        assert_eq!(read_power(&d), Power { battery: Some(100), charging: false, external: false });
        std::fs::remove_dir_all(&d).unwrap();

        let d = scratch("full");
        supply(&d, "battery", &[("type", "Battery"), ("capacity", "100"), ("status", "Full")]);
        assert_eq!(read_power(&d), Power { battery: Some(100), charging: false, external: true });
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn missing_or_odd_power_supply_is_not_an_error() {
        assert_eq!(read_power(Path::new("/nonexistent/power_supply")), Power::default());
        let d = scratch("odd");
        supply(&d, "battery", &[("type", "Battery"), ("capacity", "lots"), ("status", "$(reboot)")]);
        assert_eq!(read_power(&d), Power::default());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
