//! What the shell shows and how it reacts: the shelf, the launcher panel and the quick panel.
//! Nothing here knows about Wayland; main.rs hands in canvases and input and carries out `Act`.
//! All coordinates are logical units (the design is laid out for a 1280-wide screen).

use crate::apps::{self, App};
use crate::draw::{argb, Canvas, Color, Fonts, Image};
use crate::icons::Icons;
use crate::lang::{self, fill, Count, Lang, Msg};
use crate::sys::{self, Battery, NetDetail, Volume, Window};
use crate::wifi::{self, Security};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

#[path = "drop_ui.rs"]
mod drop_ui;
pub use drop_ui::NOTICE_SIZE;

// Theme (the dark theme of the Android original, with Shabnam teal as the accent).
const PANEL: Color = argb(0xF2202124);
const TEXT: Color = argb(0xFFE8EAED);
const TEXT2: Color = argb(0xFF9AA0A6);
const ACCENT: Color = argb(0xFF12C8C4);
const ON_ACCENT: Color = argb(0xFF202124);
const SHELF: Color = argb(0x99121419);
const CHIP: Color = argb(0x24FFFFFF);
const HOVER: Color = argb(0x2E12C8C4);
const WARN: Color = argb(0xFFF0A020);
const MENU: Color = argb(0xFA2D2F33);

pub const SHELF_H: f32 = 52.0;
const MARK: &str = "/usr/share/shabnam/mark.png";

/// What the session-ending buttons run: the word tells the session script what to do once the
/// compositor has quit ("mac": hand the panel to the Mac; "poweroff"; "reboot"; "desktop": start
/// the desktop again, which is how a new system language reaches every program).
fn end_session_command(mode: &str) -> String {
    format!("echo {mode} > /run/mode; niri msg action quit --skip-confirmation")
}

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;

type Rect = (f32, f32, f32, f32);

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 && y >= r.1 && x < r.0 + r.2 && y < r.1 + r.3
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Panel {
    Launcher,
    Quick,
    /// Nothing but a context menu (the one of a shelf icon).
    Menu,
}

/// What main.rs has to do after an input event.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Act {
    None,
    Toggle(Panel),
    /// Close whatever is open, then show this panel.
    Open(Panel),
    Close,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Where {
    Shelf,
    Panel,
    /// The volume / brightness indicator.
    Osd,
    /// The 隔空传送 card (incoming transfer, arrival).
    Notice,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum OsdKind {
    Volume,
    Brightness,
}

/// Logical size of the indicator's surface.
pub const OSD_SIZE: (u32, u32) = (236, 46);

/// A loudspeaker in an 18 x 18 box: body, cone, then a sound wave or a stroke through it.
fn speaker_shape(gx: f32, gy: f32, muted: bool) -> bool {
    let dy = (gy - 9.0).abs();
    let speaker = (gx >= 2.5 && gx <= 6.0 && dy <= 2.6) || (gx > 6.0 && gx <= 10.0 && dy <= 2.6 + (gx - 6.0) * 0.95);
    let extra = if muted {
        gx >= 3.0 && gx <= 15.5 && (gy - (gx + 0.2)).abs() <= 0.9
    } else {
        let d = ((gx - 9.5).powi(2) + (gy - 9.0).powi(2)).sqrt();
        gx > 11.5 && (4.3..=5.7).contains(&d) && dy <= (gx - 9.5) * 1.1
    };
    speaker || extra
}

/// A sun in an 18 x 18 box: a disc with eight rays.
fn sun_shape(gx: f32, gy: f32) -> bool {
    let (dx, dy) = (gx - 9.0, gy - 9.0);
    let d = (dx * dx + dy * dy).sqrt();
    if d <= 3.7 {
        return true;
    }
    let step = std::f32::consts::FRAC_PI_4;
    let angle = dy.atan2(dx);
    let off = (angle - (angle / step).round() * step).abs();
    (5.6..=8.4).contains(&d) && off * d <= 0.85
}

pub enum Key {
    Escape,
    Enter,
    Up,
    Down,
    Left,
    Right,
    Backspace,
    ClearLine,
    Text(String),
}

/// One icon on the shelf: a pinned app, a running one, or both.
struct ShelfItem {
    /// Index into `apps`; None for a window no desktop file claims.
    app: Option<usize>,
    key: String,
    label: String,
    icon: String,
    windows: Vec<u64>,
    focused: bool,
    pinned: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum ShelfHit {
    Launcher,
    Item(usize),
    Status,
}

struct ShelfLayout {
    /// x of each item's 44-wide cell.
    cells: Vec<f32>,
    separator: Option<f32>,
    status: Rect,
}

#[derive(Clone, PartialEq)]
enum Target {
    App(usize),
    Url(String),
}

/// Something that can be chosen in the launcher; the rectangle is relative to the scrolled content.
#[derive(Clone)]
struct Entry {
    rect: Rect,
    target: Target,
    /// Grid cell (icon above name) or list row (icon left of name).
    cell: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Slider {
    Brightness,
    Volume,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum QuickHit {
    Wifi,
    Mac,
    Mute,
    Poweroff,
    Reboot,
    /// The 隔空传送 switch and its "quick receive" switch.
    Drop,
    DropQuick,
    /// The language row, one of its three choices, and the "restart the desktop" button of the
    /// note that appears while other programs still run in the previous language.
    Lang,
    LangPick(Lang),
    RestartDesktop,
}

impl QuickHit {
    /// The word written to /run/mode by the buttons that end the session.
    fn mode(self) -> Option<&'static str> {
        match self {
            QuickHit::Mac => Some("mac"),
            QuickHit::Poweroff => Some("poweroff"),
            QuickHit::Reboot => Some("reboot"),
            QuickHit::RestartDesktop => Some("desktop"),
            _ => None,
        }
    }
}

/// What the quick panel shows: its controls, the network list, or the password field.
#[derive(Clone, Copy, PartialEq, Debug)]
enum QuickView {
    Main,
    Wifi,
    Password,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum WifiHit {
    Back,
    Rescan,
    Row(usize),
    Reveal,
    Cancel,
    Connect,
}

struct WifiRow {
    ssid: Vec<u8>,
    name: String,
    signal: i32,
    security: Security,
    /// wpa_supplicant's id when it already has a configuration for this network.
    known: Option<u32>,
    current: bool,
}

struct WifiLayout {
    rect: Rect,
    back: Rect,
    rescan: Rect,
    /// The list, or the password form.
    body: Rect,
    field: Rect,
    reveal: Rect,
    cancel: Rect,
    connect: Rect,
    footer_y: f32,
}

const WIFI_ROW: f32 = 48.0;

#[derive(Default)]
struct WifiUi {
    snapshot: wifi::Snapshot,
    rows: Vec<WifiRow>,
    scanning: bool,
    /// A connection attempt is under way.
    busy: bool,
    /// Text for the bottom line and whether it reports a problem.
    message: Option<(String, bool)>,
    scroll: f32,
    hover: Option<WifiHit>,
    /// The network the password form is for.
    target: Option<(Vec<u8>, Security)>,
    password: String,
    reveal: bool,
    bad_password: bool,
    /// The form was opened by a test command: its connect button does nothing.
    demo: bool,
}

impl WifiUi {
    /// Forgets the typed password, overwriting it first.
    fn wipe_password(&mut self) {
        let mut bytes = std::mem::take(&mut self.password).into_bytes();
        bytes.fill(0);
        self.bad_password = false;
        self.reveal = false;
    }
}

#[derive(Clone, PartialEq)]
enum MenuAct {
    /// Start the app with this id (another window, or the first).
    Start(String),
    TogglePin(String),
    ToggleHidden(String),
    CloseWindows(Vec<u64>),
}

/// A context menu, drawn on the panel surface.
struct Menu {
    title: String,
    items: Vec<(String, MenuAct)>,
    /// Shelf menu: x of the icon's middle. Launcher menu: where the pointer was.
    anchor: (f32, f32),
    from_shelf: bool,
    width: f32,
    hover: Option<usize>,
}

const MENU_ITEM: f32 = 34.0;
const MENU_HEAD: f32 = 30.0;

struct QuickLayout {
    rect: Rect,
    wifi: Rect,
    mac: Rect,
    drop: Rect,
    drop_quick: Option<Rect>,
    poweroff: Rect,
    reboot: Rect,
    brightness: Option<Rect>,
    volume: Option<Rect>,
    mute: Option<Rect>,
    /// The language row; under it, while it is open, the three choices; then the note (its box
    /// and, when a desktop restart would help, its button).
    lang: Rect,
    lang_choices: Option<[Rect; 3]>,
    lang_note: Option<(Rect, Option<Rect>)>,
    footer_y: f32,
}

/// Height of one line of the language note, and of its button.
const NOTE_LINE: f32 = 16.0;
const NOTE_BUTTON: f32 = 30.0;

/// Breaks `text` into at most `max_lines` lines no wider than `max_w`: at spaces where it has
/// them (Russian, English), between characters where it has none (Chinese). What does not fit
/// on the last line ends in an ellipsis.
fn wrap(fonts: &mut Fonts, text: &str, size: f32, max_w: f32, max_lines: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        if lines.len() + 1 >= max_lines || fonts.width(rest, size) <= max_w {
            lines.push(fonts.fit(rest, size, max_w));
            break;
        }
        let (mut end, mut space) = (0, None);
        for (i, ch) in rest.char_indices() {
            if fonts.width(&rest[..i + ch.len_utf8()], size) > max_w {
                break;
            }
            if ch == ' ' {
                space = Some(i);
            }
            end = i + ch.len_utf8();
        }
        if end == 0 {
            end = rest.chars().next().map_or(rest.len(), char::len_utf8);
        }
        // Not through the middle of a word when there is a space to break at.
        let cut = match space {
            Some(space) if space > 0 && end < rest.len() && !rest[end..].starts_with(' ') => space,
            _ => end,
        };
        lines.push(rest[..cut].trim_end().to_string());
        rest = rest[cut..].trim_start();
    }
    lines
}

pub struct Ui {
    pub fonts: Fonts,
    icons: Icons,
    /// The language the session was started in (its LANG): what programs that are already
    /// running, and those the compositor starts, still speak after the language was switched.
    session_lang: Lang,
    /// The three choices under the language row are showing.
    lang_open: bool,
    /// The last language choice could not be written to the locale file.
    lang_error: bool,
    /// The note under the language row, broken into lines; empty when there is nothing to say.
    lang_note: Vec<String>,
    /// The armed button was armed by a test command: the confirming click only disarms it.
    armed_demo: bool,
    apps: Vec<App>,
    apps_stamp: Vec<Option<SystemTime>>,
    pins: Vec<String>,
    recent: Vec<String>,
    windows: Vec<Window>,

    time: String,
    date: String,
    battery: Option<Battery>,
    online: bool,
    mark: Option<Image>,
    mark_scaled: Option<Image>,

    shelf_w: f32,
    shelf_items: Vec<ShelfItem>,
    shelf_hover: Option<ShelfHit>,

    pub open: Option<Panel>,
    panel_size: (f32, f32),

    query: String,
    entries: Vec<Entry>,
    /// Entry indices row by row, for the arrow keys.
    rows: Vec<Vec<usize>>,
    titles: Vec<(f32, String)>,
    content_h: f32,
    selected: Option<usize>,
    scroll: f32,

    brightness: Option<u32>,
    volume: Option<Volume>,
    net: NetDetail,
    drag: Option<Slider>,
    volume_sent: Instant,
    quick_hover: Option<QuickHit>,
    /// A session-ending button (second screen, power off, restart) was clicked once and waits
    /// for the confirming click.
    armed: Option<QuickHit>,
    quick_view: QuickView,
    wifi: WifiUi,
    wifi_requests: Option<mpsc::Sender<wifi::Request>>,

    /// Ids of apps the owner took out of the launcher's grid (search still finds them).
    hidden: Vec<String>,
    menu: Option<Menu>,
    /// The menu to show as soon as the menu panel is open.
    menu_request: Option<Menu>,

    /// What the indicator shows: which value and its percentage.
    osd: Option<(OsdKind, u32)>,

    /// 隔空传送: the receiver's state and the notice card (drop_ui.rs).
    drop: drop_ui::DropUi,
    pub dirty_notice: bool,

    pub dirty_shelf: bool,
    pub dirty_panel: bool,
    pub dirty_osd: bool,
}

fn app_dirs() -> Vec<String> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var("HOME").ok().filter(|h| !h.is_empty()) {
        dirs.push(format!("{home}/.local/share/applications"));
    }
    dirs.push("/usr/local/share/applications".into());
    dirs.push("/usr/share/applications".into());
    dirs
}

fn dirs_stamp() -> Vec<Option<SystemTime>> {
    app_dirs().iter().map(|d| std::fs::metadata(d).and_then(|m| m.modified()).ok()).collect()
}

/// The pins used until the owner changes them: browser, terminal, file manager and text editor, whichever exist.
fn default_pins(apps: &[App]) -> Vec<String> {
    const GROUPS: [&[&str]; 4] = [
        &["shabnam-browser", "chromium", "chromium-browser", "firefox"],
        &["foot", "shabnam-terminal", "rio", "org.wezfurlong.wezterm", "Alacritty"],
        &["yazi", "shabnam-files", "thunar", "pcmanfm", "org.gnome.Nautilus"],
        &["Helix", "helix"],
    ];
    GROUPS.iter().filter_map(|group| group.iter().find(|id| apps.iter().any(|a| a.id == **id))).map(|id| id.to_string()).collect()
}

fn looks_like_url(query: &str) -> bool {
    let q = query.trim();
    !q.is_empty() && !q.contains(char::is_whitespace) && (q.starts_with("http") || (q.contains('.') && !q.starts_with('.') && !q.ends_with('.')))
}

impl Ui {
    pub fn new() -> Ui {
        let mut fonts = Fonts::load();
        // The language: the word the language switch left in the locale file, else the session's LANG.
        let file = std::fs::read_to_string(lang::LOCALE_FILE).ok();
        let var = std::env::var("LANG").ok();
        let mut language = lang::startup(file.as_deref(), var.as_deref());
        // A language no installed font has the letters for cannot be shown: English then.
        let sample = match language {
            Lang::Zh => "搜索应用网址最近使用全部亮度音量电量副屏语言",
            Lang::Ru => "Поиск приложений Яркость Громкость Язык Выключить",
            Lang::En => "",
        };
        if !fonts.can_show(sample) {
            language = Lang::En;
        }
        lang::set_current(language);
        let mut ui = Ui {
            fonts,
            icons: Icons::new(),
            session_lang: var.as_deref().and_then(Lang::from_locale).unwrap_or(Lang::Zh),
            lang_open: false,
            lang_error: false,
            lang_note: Vec::new(),
            armed_demo: false,
            apps: Vec::new(),
            apps_stamp: Vec::new(),
            pins: Vec::new(),
            recent: sys::read_list("recent").unwrap_or_default(),
            windows: sys::windows().unwrap_or_default(),
            time: String::new(),
            date: String::new(),
            battery: None,
            online: false,
            mark: Image::load_png(MARK),
            mark_scaled: None,
            shelf_w: 1280.0,
            shelf_items: Vec::new(),
            shelf_hover: None,
            open: None,
            panel_size: (1280.0, 668.0),
            query: String::new(),
            entries: Vec::new(),
            rows: Vec::new(),
            titles: Vec::new(),
            content_h: 0.0,
            selected: None,
            scroll: 0.0,
            brightness: None,
            volume: None,
            net: NetDetail::default(),
            drag: None,
            volume_sent: Instant::now(),
            quick_hover: None,
            armed: None,
            quick_view: QuickView::Main,
            wifi: WifiUi::default(),
            wifi_requests: None,
            hidden: sys::read_list("hidden").unwrap_or_default(),
            menu: None,
            menu_request: None,
            osd: None,
            drop: drop_ui::DropUi::default(),
            dirty_notice: false,
            dirty_shelf: true,
            dirty_panel: false,
            dirty_osd: false,
        };
        ui.reload_apps();
        ui.tick();
        ui.refresh_lang_note();
        ui
    }

    /// A text of the shell in the language on screen (all of them are in lang.rs).
    fn t(&self, msg: Msg) -> &'static str {
        msg.text(lang::current())
    }

    /// An application's name in the language on screen: Name[ru] in Russian, Name[zh_CN] in
    /// Chinese …, the plain Name where the desktop file has no translation.
    fn name_of(&self, app: &App) -> String {
        app.name_in(lang::current().desktop_keys()).to_string()
    }

    /// The largest text size, from `size` down to 2.5 below it, at which `text` is no wider
    /// than `max_w` — for labels that are longer in one language than in another.
    fn size_to_fit(&mut self, text: &str, size: f32, max_w: f32) -> f32 {
        let mut fitted = size;
        while fitted > size - 2.5 && self.fonts.width(text, fitted) > max_w {
            fitted -= 0.5;
        }
        fitted
    }

    /// Like `Canvas::text_centered`, but a label too long for `max_w` is first drawn smaller
    /// and only then shortened.
    fn label_centered(&mut self, c: &mut Canvas, text: &str, cx: f32, mid_y: f32, size: f32, max_w: f32, ink: Color) {
        let size = self.size_to_fit(text, size, max_w);
        c.text_centered(&mut self.fonts, text, cx, mid_y, size, max_w, ink);
    }

    // ---------- language ----------

    /// Switches the system language: the language row's choices and `shabnam-shell lang WORD`
    /// both end here. The word goes to the locale file (the session script exports it as LANG
    /// the next time the desktop starts), the shell's own texts change at once, and programs
    /// started from the shell from now on get the new LANG.
    pub fn set_language(&mut self, language: Lang) {
        self.lang_error = match lang::save(language) {
            Ok(()) => false,
            Err(e) => {
                eprintln!("shabnam-shell: cannot write {}: {e}", lang::LOCALE_FILE);
                true
            }
        };
        self.lang_open = false;
        lang::set_current(language);
        self.relabel();
    }

    /// Everything that holds text made in the previous language is made again.
    fn relabel(&mut self) {
        self.icons.forget();
        self.rebuild_shelf();
        self.date.clear();
        self.tick();
        if self.open == Some(Panel::Launcher) {
            self.layout_launcher();
        }
        self.menu = None;
        self.armed = None;
        if !self.wifi.busy {
            self.wifi.message = None;
        }
        self.refresh_lang_note();
        (self.dirty_shelf, self.dirty_panel, self.dirty_osd, self.dirty_notice) = (true, self.open.is_some(), true, true);
    }

    /// The note under the language row: while the language on screen is not the one the session
    /// was started in, other programs have not followed yet.
    fn refresh_lang_note(&mut self) {
        let msg = if self.lang_error {
            Some(Msg::LanguageNotSaved)
        } else if lang::current() != self.session_lang {
            Some(Msg::LanguageNotice)
        } else {
            None
        };
        let room = 384.0f32.min(self.panel_size.0 - 20.0) - 28.0 - 24.0;
        self.lang_note = match msg {
            Some(msg) => wrap(&mut self.fonts, msg.text(lang::current()), 12.0, room, 2),
            None => Vec::new(),
        };
    }

    /// Test command: with the quick panel open, shows the three language choices.
    pub fn test_lang_menu(&mut self) {
        if self.open == Some(Panel::Quick) {
            self.quick_view = QuickView::Main;
            self.lang_open = true;
            self.dirty_panel = true;
        }
    }

    /// Test command: with the quick panel open, shows a session-ending button as it looks after
    /// its first click ("poweroff", "reboot", "mac", "desktop"). Clicking it then only disarms it.
    pub fn test_arm(&mut self, word: &str) {
        let hit = [QuickHit::Poweroff, QuickHit::Reboot, QuickHit::Mac, QuickHit::RestartDesktop].into_iter().find(|h| h.mode() == Some(word));
        if let (Some(Panel::Quick), Some(hit)) = (self.open, hit) {
            self.quick_view = QuickView::Main;
            (self.armed, self.armed_demo) = (Some(hit), true);
            self.dirty_panel = true;
        }
    }

    fn reload_apps(&mut self) {
        self.apps_stamp = dirs_stamp();
        self.apps = apps::load(&app_dirs(), &std::env::var("PATH").unwrap_or_default());
        self.pins = sys::read_list("pins").unwrap_or_else(|| default_pins(&self.apps));
        self.rebuild_shelf();
    }

    /// Re-reads the application list when a .desktop file was added or removed.
    fn refresh_apps(&mut self) {
        if dirs_stamp() != self.apps_stamp {
            self.reload_apps();
        }
    }

    // ---------- periodic ----------

    /// Refreshes clock, battery and network; returns how long until the next call is due.
    pub fn tick(&mut self) -> Duration {
        let now = sys::now();
        let date = lang::date(lang::current(), now.weekday, now.month, now.day);
        let battery = sys::battery();
        let online = sys::network_interface().is_some();
        if now.time != self.time || battery != self.battery || online != self.online {
            self.dirty_shelf = true;
        }
        if date != self.date || battery != self.battery || online != self.online {
            self.dirty_panel |= self.open == Some(Panel::Quick);
        }
        (self.time, self.date, self.battery, self.online) = (now.time, date, battery, online);
        // Land just after the minute changes, and look at the battery at least every 30 s.
        Duration::from_secs(now.to_next_minute.clamp(1, 30)) + Duration::from_millis(50)
    }

    pub fn set_windows(&mut self, windows: Vec<Window>) {
        if windows != self.windows {
            self.windows = windows;
            self.rebuild_shelf();
        }
    }

    // ---------- shelf ----------

    fn rebuild_shelf(&mut self) {
        let mut items: Vec<ShelfItem> = Vec::new();
        for id in &self.pins {
            let Some(index) = self.apps.iter().position(|a| &a.id == id) else { continue };
            if items.iter().any(|i| i.app == Some(index)) {
                continue;
            }
            let app = &self.apps[index];
            items.push(ShelfItem {
                app: Some(index),
                key: app.id.clone(),
                label: self.name_of(app),
                icon: app.icon.clone(),
                windows: Vec::new(),
                focused: false,
                pinned: true,
            });
        }
        for window in &self.windows {
            let app = apps::match_window(&self.apps, &window.app_id);
            let key = match app {
                Some(index) => self.apps[index].id.clone(),
                None => format!("window:{}", window.app_id),
            };
            let position = match items.iter().position(|i| i.key == key) {
                Some(position) => position,
                None => {
                    let (label, icon) = match app {
                        Some(index) => (self.name_of(&self.apps[index]), self.apps[index].icon.clone()),
                        // No desktop file: a name made from its app id (else its title), and an
                        // icon if one is installed under the app id or its last part.
                        None => {
                            let mut label = apps::name_from_app_id(&window.app_id);
                            if label.is_empty() {
                                label = window.title.trim().to_string();
                            }
                            if label.is_empty() {
                                label = self.t(Msg::Window).to_string();
                            }
                            let lower = window.app_id.to_lowercase();
                            let last = lower.rsplit('.').next().unwrap_or("").to_string();
                            let icon = [window.app_id.clone(), lower.clone(), last]
                                .into_iter()
                                .find(|name| self.icons.has(name))
                                .unwrap_or_else(|| window.app_id.clone());
                            (label, icon)
                        }
                    };
                    items.push(ShelfItem { app, key, label, icon, windows: Vec::new(), focused: false, pinned: false });
                    items.len() - 1
                }
            };
            items[position].windows.push(window.id);
            items[position].focused |= window.focused;
        }
        self.shelf_items = items;
        self.shelf_hover = None;
        self.dirty_shelf = true;
    }

    pub fn set_shelf_width(&mut self, width: f32) {
        self.shelf_w = width;
        self.dirty_shelf = true;
    }

    fn battery_text(&self) -> Option<String> {
        self.battery.as_ref().map(|b| format!("{}%", b.percent))
    }

    fn shelf_layout(&mut self) -> ShelfLayout {
        let pinned = self.shelf_items.iter().filter(|i| i.pinned).count();
        let others = self.shelf_items.len() - pinned;
        let gap = if pinned > 0 && others > 0 { 17.0 } else { 0.0 };
        let total = self.shelf_items.len() as f32 * 50.0 + gap;
        let mut x = ((self.shelf_w - total) / 2.0).round();
        let mut cells = Vec::new();
        let mut separator = None;
        for (i, _) in self.shelf_items.iter().enumerate() {
            if i == pinned && gap > 0.0 {
                separator = Some(x + 8.0);
                x += gap;
            }
            cells.push(x + 3.0);
            x += 50.0;
        }
        // Status pill: network, battery, clock.
        let mut width = 12.0 + 15.0 + 9.0 + self.fonts.width(&self.time, 13.0) + 14.0;
        if let Some(text) = self.battery_text() {
            width += 20.0 + 4.0 + self.fonts.width(&text, 12.0) + 9.0;
        }
        let width = width.ceil();
        ShelfLayout { cells, separator, status: (self.shelf_w - 10.0 - width, 9.0, width, 34.0) }
    }

    fn shelf_hit(&mut self, x: f32, y: f32) -> Option<ShelfHit> {
        let layout = self.shelf_layout();
        if inside((6.0, 4.0, 44.0, 44.0), x, y) {
            return Some(ShelfHit::Launcher);
        }
        if inside(layout.status, x, y) {
            return Some(ShelfHit::Status);
        }
        layout.cells.iter().position(|cx| inside((*cx - 3.0, 0.0, 50.0, SHELF_H), x, y)).map(ShelfHit::Item)
    }

    pub fn draw_shelf(&mut self, c: &mut Canvas) {
        self.dirty_shelf = false;
        let layout = self.shelf_layout();
        c.clear();
        c.rrect(0.0, 0.0, self.shelf_w, SHELF_H, 0.0, SHELF);

        // Launcher button: the droplet in a circle.
        c.circle(28.0, 26.0, 18.0, CHIP);
        if self.shelf_hover == Some(ShelfHit::Launcher) || self.open == Some(Panel::Launcher) {
            c.circle(28.0, 26.0, 18.0, HOVER);
        }
        let mark_px = (22.0 * c.s).round() as u32;
        if self.mark_scaled.as_ref().map(|m| m.h) != Some(mark_px) {
            self.mark_scaled = self.mark.as_ref().map(|m| m.fitted(mark_px, mark_px));
        }
        match &self.mark_scaled {
            Some(mark) => c.image(28.0 - mark.w as f32 / c.s / 2.0, 26.0 - mark.h as f32 / c.s / 2.0, mark),
            None => c.shape(28.0 - 8.0, 26.0 - 8.0, 16.0, 16.0, TEXT, |x, y| {
                let d = ((x - 8.0).powi(2) + (y - 8.0).powi(2)).sqrt();
                (5.0..=7.5).contains(&d)
            }),
        }

        // Apps.
        let icon_px = (34.0 * c.s).round() as u32;
        for (i, x) in layout.cells.iter().enumerate() {
            let item = &self.shelf_items[i];
            if item.focused {
                c.rrect(*x, 4.0, 44.0, 44.0, 12.0, CHIP);
            }
            if self.shelf_hover == Some(ShelfHit::Item(i)) {
                c.rrect(*x, 4.0, 44.0, 44.0, 12.0, HOVER);
            }
            let tile = self.icons.tile(&item.key, &item.icon, item.app.is_some(), &item.label, icon_px, &mut self.fonts);
            c.image(x + 5.0, 9.0, &tile);
            if !item.windows.is_empty() {
                let (width, color) = if item.focused { (14.0, ACCENT) } else { (5.0, TEXT2) };
                c.rrect(x + 22.0 - width / 2.0, 44.0, width, 3.0, 1.5, color);
            }
        }
        if let Some(x) = layout.separator {
            c.rrect(x, 14.0, 1.0, 24.0, 0.0, CHIP);
        }

        // Status pill.
        let (px, py, pw, ph) = layout.status;
        c.rrect(px, py, pw, ph, 17.0, CHIP);
        if self.shelf_hover == Some(ShelfHit::Status) || self.open == Some(Panel::Quick) {
            c.rrect(px, py, pw, ph, 17.0, HOVER);
        }
        let mid = py + ph / 2.0;
        let mut x = px + 12.0;
        let mut net_color = TEXT;
        if !self.online {
            net_color[3] = 90;
        }
        c.shape(x, mid - 7.5, 15.0, 15.0, net_color, |gx, gy| {
            // A quarter disc standing on its point: the usual signal fan.
            let (dx, dy) = (gx - 7.5, gy - 13.2);
            dy < 0.0 && dx.abs() <= -dy && (dx * dx + dy * dy).sqrt() <= 10.8
        });
        x += 15.0 + 9.0;
        if let (Some(battery), Some(text)) = (self.battery.clone(), self.battery_text()) {
            let level = (battery.percent as f32 / 100.0).max(0.05);
            let color = if battery.charging { ACCENT } else { TEXT };
            c.shape(x, mid - 7.5, 20.0, 15.0, color, |gx, gy| {
                let stroke = 1.35;
                let (l, t, r, b) = (stroke, 3.6, 17.6, 11.4);
                let within = |grow: f32| gx >= l - grow && gx <= r + grow && gy >= t - grow && gy <= b + grow;
                let body = within(stroke / 2.0) && !within(-stroke / 2.0);
                let nub = gx >= 18.0 && (6.0..=9.0).contains(&gy);
                let inset = stroke * 1.6;
                let charge = gx >= l + inset && gx <= l + inset + (r - l - 2.0 * inset) * level && gy >= t + inset && gy <= b - inset;
                body || nub || charge
            });
            x += 20.0 + 4.0;
            x += c.text(&mut self.fonts, &text, x, mid, 12.0, TEXT) + 9.0;
        }
        c.text(&mut self.fonts, &self.time.clone(), x, mid, 13.0, TEXT);
    }

    fn shelf_click(&mut self, x: f32, y: f32, button: u32) -> Act {
        match (self.shelf_hit(x, y), button) {
            (Some(ShelfHit::Launcher), BTN_LEFT) => Act::Toggle(Panel::Launcher),
            (Some(ShelfHit::Launcher), BTN_RIGHT) | (Some(ShelfHit::Status), BTN_LEFT) => Act::Toggle(Panel::Quick),
            (Some(ShelfHit::Item(i)), BTN_LEFT) => {
                let item = &self.shelf_items[i];
                if item.windows.is_empty() {
                    if let Some(app) = item.app {
                        self.start(Target::App(app));
                    }
                } else {
                    // Bring its window forward; with several, each click moves on to the next one.
                    let mine: Vec<&Window> = self.windows.iter().filter(|w| item.windows.contains(&w.id)).collect();
                    let target = match mine.iter().position(|w| w.focused) {
                        Some(current) => mine[(current + 1) % mine.len()],
                        None => mine.iter().copied().max_by_key(|w| w.focus_time).unwrap_or(mine[0]),
                    };
                    if !target.focused {
                        sys::focus_window(target.id);
                    }
                }
                if self.open.is_some() {
                    Act::Close
                } else {
                    Act::None
                }
            }
            // Middle click: another window of the same app.
            (Some(ShelfHit::Item(i)), BTN_MIDDLE) => {
                if let Some(app) = self.shelf_items[i].app {
                    self.start(Target::App(app));
                }
                Act::None
            }
            // Right click: the icon's menu (new window, pin, close).
            (Some(ShelfHit::Item(i)), BTN_RIGHT) => {
                self.menu_request = Some(self.shelf_menu(i));
                Act::Open(Panel::Menu)
            }
            _ => Act::None,
        }
    }

    fn toggle_pin(&mut self, app: usize) {
        let id = self.apps[app].id.clone();
        match self.pins.iter().position(|p| *p == id) {
            Some(position) => {
                self.pins.remove(position);
            }
            None => self.pins.push(id),
        }
        sys::write_list("pins", &self.pins);
        self.rebuild_shelf();
    }

    // ---------- starting things ----------

    fn start(&mut self, target: Target) {
        match target {
            Target::App(index) => {
                let app = self.apps[index].clone();
                if let Err(e) = sys::launch(&app, &[]) {
                    eprintln!("shabnam-shell: cannot start {}: {e}", app.command);
                    return;
                }
                self.recent.retain(|id| *id != app.id);
                self.recent.insert(0, app.id);
                self.recent.truncate(20);
                sys::write_list("recent", &self.recent);
            }
            Target::Url(url) => {
                let url = if url.contains("://") { url } else { format!("https://{url}") };
                let browser = ["shabnam-browser", "chromium", "chromium-browser", "firefox"]
                    .iter()
                    .find_map(|id| self.apps.iter().find(|a| a.id == *id));
                let result = match browser {
                    Some(app) => sys::launch(app, &[url.clone()]),
                    None => sys::spawn(&["xdg-open", &url]),
                };
                if let Err(e) = result {
                    eprintln!("shabnam-shell: cannot open {url}: {e}");
                }
            }
        }
    }

    // ---------- panels in general ----------

    pub fn set_panel_size(&mut self, width: f32, height: f32) {
        self.panel_size = (width, height);
        if self.open == Some(Panel::Launcher) {
            self.layout_launcher();
        }
        self.dirty_panel = true;
    }

    /// Prepares the state of a panel that is about to be shown.
    pub fn opened(&mut self, panel: Panel) {
        self.open = Some(panel);
        self.drag = None;
        self.armed = None;
        self.quick_hover = None;
        self.quick_view = QuickView::Main;
        self.lang_open = false;
        self.menu = None;
        match panel {
            Panel::Menu => self.menu = self.menu_request.take(),
            Panel::Launcher => {
                self.refresh_apps();
                self.query.clear();
                self.scroll = 0.0;
                self.layout_launcher();
            }
            Panel::Quick => {
                self.brightness = sys::brightness();
                self.volume = sys::volume();
                self.net = sys::network_detail();
                self.refresh_lang_note();
            }
        }
        self.dirty_panel = true;
        self.dirty_shelf = true;
    }

    /// Puts text into the launcher's search field (the launcher must be open).
    pub fn set_query(&mut self, query: &str) {
        if self.open == Some(Panel::Launcher) {
            self.query = query.chars().filter(|c| !c.is_control()).take(200).collect();
            self.layout_launcher();
        }
    }

    pub fn closed(&mut self) {
        self.open = None;
        self.drag = None;
        self.armed = None;
        self.menu = None;
        self.lang_open = false;
        self.quick_view = QuickView::Main;
        self.wifi.wipe_password();
        self.wifi.target = None;
        self.wifi.hover = None;
        self.dirty_shelf = true;
    }

    pub fn draw_panel(&mut self, c: &mut Canvas) {
        self.dirty_panel = false;
        c.clear();
        match self.open {
            Some(Panel::Launcher) => self.draw_launcher(c),
            Some(Panel::Quick) if self.quick_view == QuickView::Main => self.draw_quick(c),
            Some(Panel::Quick) => self.draw_wifi(c),
            Some(Panel::Menu) | None => {}
        }
        self.draw_menu(c);
    }

    // ---------- launcher ----------

    fn launcher_rect(&self) -> Rect {
        let (pw, ph) = self.panel_size;
        let width = (pw * 0.36).round().clamp(320.0, (pw - 20.0).max(320.0));
        let height = ((ph + SHELF_H) * 0.74).round().min(ph - 16.0).max(200.0);
        (10.0, ph - 8.0 - height, width, height)
    }

    /// The scrolling area under the search field.
    fn launcher_view(&self) -> Rect {
        let (x, y, w, h) = self.launcher_rect();
        (x + 14.0, y + 14.0 + 40.0 + 8.0, w - 28.0, h - 14.0 - 40.0 - 8.0 - 8.0)
    }

    fn layout_launcher(&mut self) {
        let width = self.launcher_view().2;
        let (mut entries, mut rows, mut titles) = (Vec::new(), Vec::new(), Vec::new());
        let mut y = 0.0;
        if self.query.trim().is_empty() {
            let cell_w = width / 5.0;
            let mut grid = |title: &str, apps: &[usize], y: &mut f32| {
                titles.push((*y, title.to_string()));
                *y += 30.0;
                for chunk in apps.chunks(5) {
                    let mut row = Vec::new();
                    for (column, app) in chunk.iter().enumerate() {
                        row.push(entries.len());
                        entries.push(Entry { rect: (column as f32 * cell_w, *y, cell_w, 82.0), target: Target::App(*app), cell: true });
                    }
                    rows.push(row);
                    *y += 82.0;
                }
            };
            let recent: Vec<usize> = self
                .recent
                .iter()
                .filter(|id| !self.hidden.contains(id))
                .filter_map(|id| self.apps.iter().position(|a| a.id == *id))
                .take(5)
                .collect();
            if !recent.is_empty() {
                grid(self.t(Msg::Recent), &recent, &mut y);
            }
            let mut all: Vec<usize> = (0..self.apps.len()).filter(|i| !self.hidden.contains(&self.apps[*i].id)).collect();
            all.sort_by_key(|i| self.name_of(&self.apps[*i]).to_lowercase());
            grid(self.t(Msg::AllApps), &all, &mut y);
            self.selected = None;
        } else {
            let mut targets = Vec::new();
            if looks_like_url(&self.query) {
                targets.push(Target::Url(self.query.trim().to_string()));
            }
            targets.extend(apps::filter(&self.apps, &self.query, lang::current().desktop_keys()).into_iter().map(Target::App));
            for target in targets {
                rows.push(vec![entries.len()]);
                entries.push(Entry { rect: (0.0, y, width, 48.0), target, cell: false });
                y += 48.0;
            }
            self.selected = if entries.is_empty() { None } else { Some(0) };
        }
        (self.entries, self.rows, self.titles, self.content_h) = (entries, rows, titles, y);
        self.scroll = 0.0;
        self.dirty_panel = true;
    }

    fn max_scroll(&self) -> f32 {
        (self.content_h - self.launcher_view().3).max(0.0)
    }

    fn select(&mut self, entry: usize) {
        self.selected = Some(entry);
        let (_, top, _, height) = self.entries[entry].rect;
        let view_h = self.launcher_view().3;
        if top < self.scroll {
            // Going back to the first row also brings its heading back into view.
            self.scroll = if top <= 30.0 { 0.0 } else { top };
        } else if top + height > self.scroll + view_h {
            self.scroll = top + height - view_h;
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        self.dirty_panel = true;
    }

    fn move_selection(&mut self, dx: isize, dy: isize) {
        if self.entries.is_empty() {
            return;
        }
        let Some(current) = self.selected else {
            self.select(0);
            return;
        };
        let Some(row) = self.rows.iter().position(|r| r.contains(&current)) else { return };
        let column = self.rows[row].iter().position(|e| *e == current).unwrap_or(0);
        let entry = if dy != 0 {
            let target = (row as isize + dy).clamp(0, self.rows.len() as isize - 1) as usize;
            self.rows[target][column.min(self.rows[target].len() - 1)]
        } else {
            (current as isize + dx).clamp(0, self.entries.len() as isize - 1) as usize
        };
        self.select(entry);
    }

    fn launcher_entry_at(&self, x: f32, y: f32) -> Option<usize> {
        let view = self.launcher_view();
        if !inside(view, x, y) {
            return None;
        }
        let (cx, cy) = (x - view.0, y - view.1 + self.scroll);
        self.entries.iter().position(|e| inside(e.rect, cx, cy))
    }

    fn launcher_key(&mut self, key: Key) -> Act {
        match key {
            Key::Escape => return Act::Close,
            Key::Enter => {
                if let Some(entry) = self.selected.and_then(|i| self.entries.get(i)) {
                    self.start(entry.target.clone());
                    return Act::Close;
                }
            }
            Key::Up => self.move_selection(0, -1),
            Key::Down => self.move_selection(0, 1),
            Key::Left => self.move_selection(-1, 0),
            Key::Right => self.move_selection(1, 0),
            Key::Backspace => {
                if self.query.pop().is_some() {
                    self.layout_launcher();
                }
            }
            Key::ClearLine => {
                if !self.query.is_empty() {
                    self.query.clear();
                    self.layout_launcher();
                }
            }
            Key::Text(text) => {
                if self.query.chars().count() < 200 {
                    self.query.push_str(&text);
                    self.layout_launcher();
                }
            }
        }
        Act::None
    }

    fn draw_launcher(&mut self, c: &mut Canvas) {
        let (x, y, w, h) = self.launcher_rect();
        c.rrect(x, y, w, h, 20.0, PANEL);

        // Search field.
        let (fx, fy, fw) = (x + 14.0, y + 14.0, w - 28.0);
        c.rrect(fx, fy, fw, 40.0, 20.0, CHIP);
        let mid = fy + 20.0;
        let caret = if self.query.is_empty() {
            let hint = self.t(Msg::SearchHint);
            c.text(&mut self.fonts, hint, fx + 21.0, mid, 14.0, TEXT2);
            fx + 16.0
        } else {
            // A query too long for the field shows its end, where the typing happens.
            let mut shown = self.query.as_str();
            while self.fonts.width(shown, 14.0) > fw - 34.0 {
                let mut cut = shown.char_indices();
                cut.next();
                shown = cut.next().map_or("", |(i, _)| &shown[i..]);
            }
            let shown = shown.to_string();
            fx + 16.0 + c.text(&mut self.fonts, &shown, fx + 16.0, mid, 14.0, TEXT) + 1.0
        };
        c.rrect(caret, mid - 9.0, 1.4, 18.0, 0.0, ACCENT);

        // Scrolling content.
        let view = self.launcher_view();
        c.set_clip(Some(view));
        let top = view.1 - self.scroll;
        for (ty, title) in self.titles.clone() {
            c.text(&mut self.fonts, &title, view.0 + 8.0, top + ty + 17.0, 12.0, TEXT2);
        }
        let (cell_px, row_px) = ((44.0 * c.s).round() as u32, (34.0 * c.s).round() as u32);
        for (i, entry) in self.entries.clone().into_iter().enumerate() {
            let (ex, ey, ew, eh) = (view.0 + entry.rect.0, top + entry.rect.1, entry.rect.2, entry.rect.3);
            if ey + eh < view.1 || ey > view.1 + view.3 {
                continue;
            }
            if self.selected == Some(i) {
                c.rrect(ex, ey, ew, eh, 12.0, HOVER);
            }
            let (key, icon, label) = match &entry.target {
                Target::App(index) => {
                    let app = &self.apps[*index];
                    (app.id.clone(), app.icon.clone(), self.name_of(app))
                }
                Target::Url(url) => (String::new(), String::new(), fill(self.t(Msg::OpenUrl), &[url])),
            };
            if entry.cell {
                let tile = self.icons.tile(&key, &icon, true, &label, cell_px, &mut self.fonts);
                c.image(ex + (ew - 44.0) / 2.0, ey + 9.0, &tile);
                c.text_centered(&mut self.fonts, &label, ex + ew / 2.0, ey + 9.0 + 44.0 + 6.0 + 8.0, 11.5, ew - 6.0, TEXT);
            } else {
                if matches!(entry.target, Target::Url(_)) {
                    c.circle(ex + 10.0 + 17.0, ey + 24.0, 17.0, ACCENT);
                    c.text_centered(&mut self.fonts, "↗", ex + 27.0, ey + 24.0, 17.0, 34.0, ON_ACCENT);
                } else {
                    let tile = self.icons.tile(&key, &icon, true, &label, row_px, &mut self.fonts);
                    c.image(ex + 10.0, ey + 7.0, &tile);
                }
                let label = self.fonts.fit(&label, 14.0, ew - 56.0 - 12.0);
                c.text(&mut self.fonts, &label, ex + 56.0, ey + 24.0, 14.0, TEXT);
            }
        }
        if self.entries.is_empty() {
            let text = self.t(Msg::NoMatch);
            c.text(&mut self.fonts, text, view.0 + 8.0, view.1 + 20.0, 13.0, TEXT2);
        }
        c.set_clip(None);

        // Scroll position.
        if self.content_h > view.3 {
            let thumb = (view.3 * view.3 / self.content_h).max(24.0);
            let offset = (view.3 - thumb) * self.scroll / self.max_scroll().max(1.0);
            c.rrect(x + w - 8.0, view.1 + offset, 3.0, thumb, 1.5, CHIP);
        }
    }

    // ---------- quick panel ----------

    fn quick_layout(&self) -> QuickLayout {
        let (pw, ph) = self.panel_size;
        let width = 384.0f32.min(pw - 20.0);
        let sliders = self.brightness.is_some() as u32 + self.volume.is_some() as u32;
        // The language row, its choices while it is open, and its note while there is one.
        let note_h = if self.lang_note.is_empty() {
            0.0
        } else {
            let button = if self.lang_note_button() { 8.0 + NOTE_BUTTON } else { 0.0 };
            10.0 + self.lang_note.len() as f32 * NOTE_LINE + button + 10.0
        };
        let lang_h = 6.0 + 40.0 + if self.lang_open { 6.0 + 40.0 } else { 0.0 } + if note_h > 0.0 { 8.0 + note_h } else { 0.0 };
        let height = 14.0 + 34.0 + 10.0 + 48.0 + 10.0 + 48.0 + 10.0 + sliders as f32 * 40.0 + lang_h + 6.0 + 40.0 + 15.0 + 24.0 + 10.0;
        let (x, y) = (pw - 10.0 - width, ph - 8.0 - height);
        let (left, inner) = (x + 14.0, width - 28.0);
        let pills_y = y + 14.0 + 34.0 + 10.0;
        let half = (inner - 8.0) / 2.0;
        // 隔空传送: one wide switch; while it is on, "quick receive" takes the right half.
        let drop_y = pills_y + 48.0 + 10.0;
        let (drop, drop_quick) = if self.drop_quick_shown() {
            ((left, drop_y, half, 48.0), Some((left + half + 8.0, drop_y, half, 48.0)))
        } else {
            ((left, drop_y, inner, 48.0), None)
        };
        let mut row_y = drop_y + 48.0 + 10.0;
        let mut brightness = None;
        if self.brightness.is_some() {
            brightness = Some((left + 62.0, row_y, inner - 62.0, 40.0));
            row_y += 40.0;
        }
        let (mut volume, mut mute) = (None, None);
        if self.volume.is_some() {
            volume = Some((left + 62.0, row_y, inner - 62.0 - 42.0, 40.0));
            mute = Some((left + inner - 34.0, row_y + 3.0, 34.0, 34.0));
            row_y += 40.0;
        }
        // Language: the row, then the three choices, then the note.
        let lang = (left, row_y + 6.0, inner, 40.0);
        row_y += 6.0 + 40.0;
        let mut lang_choices = None;
        if self.lang_open {
            let third = (inner - 16.0) / 3.0;
            lang_choices = Some([0.0, 1.0, 2.0].map(|i| (left + i * (third + 8.0), row_y + 6.0, third, 40.0)));
            row_y += 6.0 + 40.0;
        }
        let mut lang_note = None;
        if note_h > 0.0 {
            let note = (left, row_y + 8.0, inner, note_h);
            let button = self.lang_note_button().then_some((left + 12.0, note.1 + note_h - 10.0 - NOTE_BUTTON, inner - 24.0, NOTE_BUTTON));
            lang_note = Some((note, button));
            row_y += 8.0 + note_h;
        }
        // Power row.
        let power_y = row_y + 6.0;
        row_y = power_y + 40.0;
        QuickLayout {
            rect: (x, y, width, height),
            wifi: (left, pills_y, half, 48.0),
            mac: (left + half + 8.0, pills_y, half, 48.0),
            drop,
            drop_quick,
            poweroff: (left, power_y, half, 40.0),
            reboot: (left + half + 8.0, power_y, half, 40.0),
            brightness,
            volume,
            mute,
            lang,
            lang_choices,
            lang_note,
            footer_y: row_y + 15.0,
        }
    }

    /// Whether the language note offers to restart the desktop: only when that would change
    /// anything, that is while the session still runs in another language.
    fn lang_note_button(&self) -> bool {
        lang::current() != self.session_lang
    }

    fn quick_hit(&self, x: f32, y: f32) -> Option<QuickHit> {
        let layout = self.quick_layout();
        if inside(layout.wifi, x, y) {
            Some(QuickHit::Wifi)
        } else if inside(layout.mac, x, y) {
            Some(QuickHit::Mac)
        } else if inside(layout.drop, x, y) {
            Some(QuickHit::Drop)
        } else if layout.drop_quick.is_some_and(|r| inside(r, x, y)) {
            Some(QuickHit::DropQuick)
        } else if inside(layout.poweroff, x, y) {
            Some(QuickHit::Poweroff)
        } else if inside(layout.reboot, x, y) {
            Some(QuickHit::Reboot)
        } else if layout.mute.is_some_and(|r| inside(r, x, y)) {
            Some(QuickHit::Mute)
        } else if inside(layout.lang, x, y) {
            Some(QuickHit::Lang)
        } else if let Some(choice) = layout.lang_choices.and_then(|rects| rects.iter().position(|r| inside(*r, x, y))) {
            Some(QuickHit::LangPick(Lang::ALL[choice]))
        } else if layout.lang_note.and_then(|(_, button)| button).is_some_and(|r| inside(r, x, y)) {
            Some(QuickHit::RestartDesktop)
        } else {
            None
        }
    }

    /// Moves a slider to the pointer's x and applies the value.
    fn drag_to(&mut self, slider: Slider, x: f32, finished: bool) {
        let layout = self.quick_layout();
        let Some(track) = (if slider == Slider::Brightness { layout.brightness } else { layout.volume }) else { return };
        let position = ((x - (track.0 + 8.0)) / (track.2 - 16.0)).clamp(0.0, 1.0);
        match slider {
            Slider::Brightness => {
                let min = sys::MIN_BRIGHTNESS as f32;
                let percent = (min + position * (100.0 - min)).round() as u32;
                if self.brightness != Some(percent) {
                    self.brightness = Some(percent);
                    sys::set_brightness(percent);
                    self.dirty_panel = true;
                }
            }
            Slider::Volume => {
                let percent = (position * 100.0).round() as u32;
                let changed = self.volume != Some(Volume { percent, muted: false });
                self.volume = Some(Volume { percent, muted: false });
                // The volume script is a process start each time: not for every pointer event.
                if finished || (changed && self.volume_sent.elapsed() > Duration::from_millis(120)) {
                    sys::set_volume(percent);
                    self.volume_sent = Instant::now();
                }
                self.dirty_panel |= changed;
            }
        }
    }

    fn quick_press(&mut self, x: f32, y: f32) -> Act {
        let layout = self.quick_layout();
        if layout.brightness.is_some_and(|r| inside(r, x, y)) {
            self.drag = Some(Slider::Brightness);
            self.drag_to(Slider::Brightness, x, false);
        } else if layout.volume.is_some_and(|r| inside(r, x, y)) {
            self.drag = Some(Slider::Volume);
            self.drag_to(Slider::Volume, x, false);
        }
        let hit = self.quick_hit(x, y);
        // A click anywhere else withdraws a pending confirmation.
        if self.armed.is_some() && self.armed != hit {
            self.armed = None;
            self.dirty_panel = true;
        }
        match (hit, hit.and_then(QuickHit::mode)) {
            (Some(QuickHit::Mute), _) => {
                sys::toggle_mute();
                self.volume = sys::volume();
                self.dirty_panel = true;
            }
            (Some(QuickHit::Wifi), _) => self.show_wifi(),
            (Some(QuickHit::Drop), _) => self.drop_tile_click(),
            (Some(QuickHit::DropQuick), _) => self.drop_quick_click(),
            (Some(QuickHit::Lang), _) => {
                self.lang_open = !self.lang_open;
                self.dirty_panel = true;
            }
            (Some(QuickHit::LangPick(language)), _) => self.set_language(language),
            (Some(button), Some(mode)) => {
                if self.armed == Some(button) && self.armed_demo {
                    // Armed by a test command: nothing ends.
                    (self.armed, self.armed_demo, self.dirty_panel) = (None, false, true);
                    return Act::None;
                }
                if self.armed == Some(button) {
                    if let Err(e) = sys::spawn(&["/bin/sh", "-c", &end_session_command(mode)]) {
                        eprintln!("shabnam-shell: cannot end the session ({mode}): {e}");
                    }
                    return Act::Close;
                }
                // It ends the desktop session, so it takes a second click to confirm.
                (self.armed, self.armed_demo) = (Some(button), false);
                self.dirty_panel = true;
            }
            _ => {}
        }
        Act::None
    }

    /// A one-line pill for the power row; orange with a confirmation text while armed.
    fn power_pill(&mut self, c: &mut Canvas, rect: Rect, hit: QuickHit, label: &str, confirm: &str) {
        let armed = self.armed == Some(hit);
        c.rrect(rect.0, rect.1, rect.2, rect.3, 20.0, if armed { WARN } else { CHIP });
        if !armed && self.quick_hover == Some(hit) {
            c.rrect(rect.0, rect.1, rect.2, rect.3, 20.0, HOVER);
        }
        let (text, ink) = if armed { (confirm, ON_ACCENT) } else { (label, TEXT) };
        self.label_centered(c, text, rect.0 + rect.2 / 2.0, rect.1 + rect.3 / 2.0, 13.0, rect.2 - 20.0, ink);
    }

    fn pill(&mut self, c: &mut Canvas, rect: Rect, title: &str, sub: &str, fill: Color, ink: Color, sub_ink: Color, hover: bool) {
        c.rrect(rect.0, rect.1, rect.2, rect.3, 24.0, fill);
        if hover {
            c.rrect(rect.0, rect.1, rect.2, rect.3, 24.0, HOVER);
        }
        // A title or line that is long in this language gets smaller before it gets cut.
        let room = rect.2 - 30.0;
        let (title_size, sub_size) = (self.size_to_fit(title, 13.0, room), self.size_to_fit(sub, 11.0, room).max(10.0));
        let title = self.fonts.fit(title, title_size, room);
        let sub = self.fonts.fit(sub, sub_size, room);
        c.text(&mut self.fonts, &title, rect.0 + 18.0, rect.1 + 16.0, title_size, ink);
        c.text(&mut self.fonts, &sub, rect.0 + 18.0, rect.1 + 33.0, sub_size, sub_ink);
    }

    fn slider(&mut self, c: &mut Canvas, track: Rect, label: &str, position: f32, live: bool) {
        let mid = track.1 + track.3 / 2.0;
        let size = self.size_to_fit(label, 12.0, 64.0);
        c.text(&mut self.fonts, label, track.0 - 62.0, mid, size, TEXT2);
        let (start, length) = (track.0 + 8.0, track.2 - 16.0);
        let knob = start + length * position.clamp(0.0, 1.0);
        let color = if live { ACCENT } else { TEXT2 };
        c.rrect(start, mid - 2.0, length, 4.0, 2.0, CHIP);
        c.rrect(start, mid - 2.0, knob - start, 4.0, 2.0, color);
        c.circle(knob, mid, 8.0, color);
    }

    fn draw_quick(&mut self, c: &mut Canvas) {
        let layout = self.quick_layout();
        let (x, y, w, h) = layout.rect;
        c.rrect(x, y, w, h, 20.0, PANEL);
        let (left, right) = (x + 14.0, x + w - 14.0);

        // Date on the left, battery on the right.
        let mid = y + 14.0 + 17.0;
        c.text(&mut self.fonts, &self.date.clone(), left + 4.0, mid, 15.0, TEXT);
        if let Some(battery) = self.battery.clone() {
            let state = if battery.full {
                Msg::BatteryFull
            } else if battery.charging {
                Msg::BatteryCharging
            } else {
                Msg::Battery
            };
            let text = fill(self.t(state), &[&battery.percent.to_string()]);
            let width = self.fonts.width(&text, 12.0);
            c.text(&mut self.fonts, &text, right - 4.0 - width, mid, 12.0, TEXT2);
        }

        // Wi-Fi (opens the network list) and the second-screen button.
        let wifi_up = self.net.interface.as_deref().is_some_and(|i| i.starts_with("wl"));
        let sub = match (&self.net.ssid, wifi_up, &self.net.interface) {
            (Some(ssid), _, _) => ssid.clone(),
            (None, true, _) => self.t(Msg::Connected).to_string(),
            (None, false, Some(other)) => fill(self.t(Msg::WiredOnly), &[other]),
            (None, false, None) => self.t(Msg::NotConnected).to_string(),
        };
        let (fill, ink, sub_ink) = if wifi_up { (ACCENT, ON_ACCENT, ON_ACCENT) } else { (CHIP, TEXT, TEXT2) };
        self.pill(c, layout.wifi, "Wi-Fi", &sub, fill, ink, sub_ink, !wifi_up && self.quick_hover == Some(QuickHit::Wifi));
        if wifi_up && self.quick_hover == Some(QuickHit::Wifi) {
            let (px, py, pw, ph) = layout.wifi;
            c.rrect(px, py, pw, ph, 24.0, argb(0x33FFFFFF));
        }
        // A chevron: the pill leads somewhere.
        c.shape(layout.wifi.0 + layout.wifi.2 - 26.0, layout.wifi.1 + 16.0, 10.0, 16.0, ink, |gx, gy| {
            let d = (gy - 8.0).abs();
            d <= 5.5 && (gx - (7.0 - d)).abs() <= 0.95
        });
        let hover = self.quick_hover == Some(QuickHit::Mac);
        if self.armed == Some(QuickHit::Mac) {
            let (title, sub) = (self.t(Msg::ConfirmTitle), self.t(Msg::ConfirmSub));
            self.pill(c, layout.mac, title, sub, WARN, ON_ACCENT, ON_ACCENT, false);
        } else {
            let (title, sub) = (self.t(Msg::MacTitle), self.t(Msg::MacSub));
            self.pill(c, layout.mac, title, sub, CHIP, TEXT, TEXT2, hover);
        }

        self.draw_drop_row(c, &layout);

        // Sliders.
        if let (Some(track), Some(percent)) = (layout.brightness, self.brightness) {
            let min = sys::MIN_BRIGHTNESS as f32;
            let label = self.t(Msg::Brightness);
            self.slider(c, track, label, (percent as f32 - min) / (100.0 - min), true);
        }
        if let (Some(track), Some(volume), Some(mute)) = (layout.volume, self.volume, layout.mute) {
            let label = self.t(Msg::Volume);
            self.slider(c, track, label, volume.percent as f32 / 100.0, !volume.muted);
            let (cx, cy) = (mute.0 + 17.0, mute.1 + 17.0);
            c.circle(cx, cy, 17.0, if volume.muted { ACCENT } else { CHIP });
            if self.quick_hover == Some(QuickHit::Mute) {
                c.circle(cx, cy, 17.0, HOVER);
            }
            let muted = volume.muted;
            c.shape(cx - 9.0, cy - 9.0, 18.0, 18.0, if muted { ON_ACCENT } else { TEXT }, move |gx, gy| speaker_shape(gx, gy, muted));
        }

        self.draw_language(c, &layout);

        // Power row.
        let (label, confirm) = (self.t(Msg::PowerOff), self.t(Msg::PowerOffConfirm));
        self.power_pill(c, layout.poweroff, QuickHit::Poweroff, label, confirm);
        let (label, confirm) = (self.t(Msg::Restart), self.t(Msg::RestartConfirm));
        self.power_pill(c, layout.reboot, QuickHit::Reboot, label, confirm);

        // Network line.
        c.rrect(left, layout.footer_y - 7.0, w - 28.0, 1.0, 0.0, CHIP);
        let line = match (&self.net.interface, &self.net.address) {
            (Some(interface), Some(address)) => format!("{interface} · {address}"),
            (Some(interface), None) => interface.clone(),
            _ => self.t(Msg::NoNetwork).to_string(),
        };
        c.text(&mut self.fonts, &line, left + 4.0, layout.footer_y + 12.0, 11.5, TEXT2);
    }

    /// The language row (label, the current language, a chevron), the three choices while it
    /// is open — each written in its own language — and the note about programs still running
    /// in the previous language, with its button to restart the desktop.
    fn draw_language(&mut self, c: &mut Canvas, layout: &QuickLayout) {
        let (x, y, w, h) = layout.lang;
        let mid = y + h / 2.0;
        c.rrect(x, y, w, h, 20.0, CHIP);
        if self.quick_hover == Some(QuickHit::Lang) || self.lang_open {
            c.rrect(x, y, w, h, 20.0, HOVER);
        }
        let label = self.t(Msg::Language);
        c.text(&mut self.fonts, label, x + 18.0, mid, 13.0, TEXT);
        let name = lang::current().own_name();
        let width = self.fonts.width(name, 13.0);
        c.text(&mut self.fonts, name, x + w - 36.0 - width, mid, 13.0, TEXT2);
        // The chevron points down while the choices are showing.
        let open = self.lang_open;
        c.shape(x + w - 29.0, mid - 8.0, 16.0, 16.0, TEXT, move |gx, gy| {
            let (along, across) = if open { (gx - 8.0, gy - 5.5) } else { (gy - 8.0, gx - 5.5) };
            let d = along.abs();
            d <= 5.5 && (across - (5.5 - d)).abs() <= 0.95
        });
        if let Some(rects) = layout.lang_choices {
            for (language, (rx, ry, rw, rh)) in Lang::ALL.into_iter().zip(rects) {
                let chosen = language == lang::current();
                c.rrect(rx, ry, rw, rh, 20.0, if chosen { ACCENT } else { CHIP });
                if self.quick_hover == Some(QuickHit::LangPick(language)) {
                    c.rrect(rx, ry, rw, rh, 20.0, if chosen { argb(0x33FFFFFF) } else { HOVER });
                }
                self.label_centered(c, language.own_name(), rx + rw / 2.0, ry + rh / 2.0, 13.0, rw - 16.0, if chosen { ON_ACCENT } else { TEXT });
            }
        }
        if let Some(((nx, ny, nw, nh), button)) = layout.lang_note {
            c.rrect(nx, ny, nw, nh, 14.0, argb(0x14FFFFFF));
            let ink = if self.lang_error { WARN } else { TEXT2 };
            for (i, line) in self.lang_note.clone().iter().enumerate() {
                c.text(&mut self.fonts, line, nx + 12.0, ny + 10.0 + (i as f32 + 0.5) * NOTE_LINE, 12.0, ink);
            }
            if let Some((bx, by, bw, bh)) = button {
                let armed = self.armed == Some(QuickHit::RestartDesktop);
                c.rrect(bx, by, bw, bh, 15.0, if armed { WARN } else { CHIP });
                if !armed && self.quick_hover == Some(QuickHit::RestartDesktop) {
                    c.rrect(bx, by, bw, bh, 15.0, HOVER);
                }
                let (text, ink) = if armed { (self.t(Msg::RestartDesktopConfirm), ON_ACCENT) } else { (self.t(Msg::RestartDesktop), TEXT) };
                self.label_centered(c, text, bx + bw / 2.0, by + bh / 2.0, 12.5, bw - 20.0, ink);
            }
        }
    }

    // ---------- input ----------

    pub fn key(&mut self, key: Key) -> Act {
        match (self.open, key) {
            // While a menu is up the keyboard only dismisses it.
            (Some(Panel::Menu), Key::Escape) => Act::Close,
            (Some(_), Key::Escape) if self.menu.is_some() => {
                self.menu = None;
                self.dirty_panel = true;
                Act::None
            }
            (Some(_), _) if self.menu.is_some() => Act::None,
            (Some(Panel::Launcher), key) => self.launcher_key(key),
            (Some(Panel::Quick), key) => self.quick_key(key),
            _ => Act::None,
        }
    }

    pub fn pointer_move(&mut self, place: Where, x: f32, y: f32) {
        match (place, self.open) {
            (Where::Shelf, _) => {
                let hit = self.shelf_hit(x, y);
                if hit != self.shelf_hover {
                    self.shelf_hover = hit;
                    self.dirty_shelf = true;
                }
            }
            (Where::Panel, Some(_)) if self.menu.is_some() => {
                let hover = self.menu_item_at(x, y);
                if let Some(menu) = self.menu.as_mut().filter(|m| m.hover != hover) {
                    menu.hover = hover;
                    self.dirty_panel = true;
                }
            }
            (Where::Notice, _) => self.notice_move(x, y),
            (Where::Panel, Some(Panel::Quick)) if self.quick_view != QuickView::Main => {
                let hit = self.wifi_hit(x, y);
                if hit != self.wifi.hover {
                    self.wifi.hover = hit;
                    self.dirty_panel = true;
                }
            }
            (Where::Panel, Some(Panel::Launcher)) => {
                if let Some(entry) = self.launcher_entry_at(x, y) {
                    if self.selected != Some(entry) {
                        self.selected = Some(entry);
                        self.dirty_panel = true;
                    }
                }
            }
            (Where::Panel, Some(Panel::Quick)) => {
                if let Some(slider) = self.drag {
                    self.drag_to(slider, x, false);
                }
                let hit = self.quick_hit(x, y);
                if hit != self.quick_hover {
                    self.quick_hover = hit;
                    // Moving off an armed button withdraws the pending confirmation.
                    if self.armed != hit {
                        self.armed = None;
                    }
                    self.dirty_panel = true;
                }
            }
            _ => {}
        }
    }

    pub fn pointer_leave(&mut self, place: Where) {
        match place {
            Where::Shelf => {
                if self.shelf_hover.take().is_some() {
                    self.dirty_shelf = true;
                }
            }
            Where::Osd => {}
            Where::Notice => self.notice_leave(),
            Where::Panel => {
                let menu_hover = self.menu.as_mut().and_then(|m| m.hover.take());
                if self.quick_hover.take().is_some() || self.armed.take().is_some() || self.wifi.hover.take().is_some() || menu_hover.is_some() {
                    self.dirty_panel = true;
                }
            }
        }
    }

    pub fn press(&mut self, place: Where, x: f32, y: f32, button: u32) -> Act {
        match (place, self.open) {
            (Where::Shelf, _) => self.shelf_click(x, y, button),
            (Where::Notice, _) => {
                if button == BTN_LEFT {
                    self.notice_press(x, y);
                }
                Act::None
            }
            (Where::Panel, Some(_)) if self.menu.is_some() => self.menu_press(x, y, button),
            (Where::Panel, Some(Panel::Menu)) => Act::Close,
            (Where::Panel, Some(Panel::Launcher)) => {
                if !inside(self.launcher_rect(), x, y) {
                    return Act::Close;
                }
                match (self.launcher_entry_at(x, y).map(|i| self.entries[i].target.clone()), button) {
                    (Some(target), BTN_LEFT) => {
                        self.start(target);
                        Act::Close
                    }
                    // Right click: the entry's menu (open, pin, hide).
                    (Some(Target::App(app)), BTN_RIGHT) => {
                        let title = self.name_of(&self.apps[app]);
                        self.menu = Some(self.build_menu(title, Some(app), &[], (x, y), false));
                        self.dirty_panel = true;
                        Act::None
                    }
                    _ => Act::None,
                }
            }
            (Where::Panel, Some(Panel::Quick)) => {
                let rect = if self.quick_view == QuickView::Main { self.quick_layout().rect } else { self.wifi_layout().rect };
                if !inside(rect, x, y) {
                    return Act::Close;
                }
                match (button, self.quick_view) {
                    (BTN_LEFT, QuickView::Main) => self.quick_press(x, y),
                    (BTN_LEFT, _) => {
                        self.wifi_press(x, y);
                        Act::None
                    }
                    _ => Act::None,
                }
            }
            _ => Act::None,
        }
    }

    pub fn release(&mut self, x: f32) {
        if let Some(slider) = self.drag.take() {
            self.drag_to(slider, x, true);
        }
    }

    /// `steps` is in wheel notches, positive downwards.
    pub fn scroll(&mut self, place: Where, steps: f32) {
        if place == Where::Panel && self.open == Some(Panel::Launcher) {
            let scroll = (self.scroll + steps * 60.0).clamp(0.0, self.max_scroll());
            if scroll != self.scroll {
                self.scroll = scroll;
                self.dirty_panel = true;
            }
        }
        if place == Where::Panel && self.open == Some(Panel::Quick) && self.quick_view == QuickView::Wifi {
            let most = (self.wifi.rows.len() as f32 * WIFI_ROW - self.wifi_layout().body.3).max(0.0);
            let scroll = (self.wifi.scroll + steps * 48.0).clamp(0.0, most);
            if scroll != self.wifi.scroll {
                self.wifi.scroll = scroll;
                self.dirty_panel = true;
            }
        }
    }
}

// ---------- quick panel: Wi-Fi ----------

impl Ui {
    /// Gives the panel its line to the Wi-Fi worker thread.
    pub fn set_wifi(&mut self, requests: mpsc::Sender<wifi::Request>) {
        self.wifi_requests = Some(requests);
    }

    fn wifi_send(&mut self, request: wifi::Request) -> bool {
        self.wifi_requests.as_ref().is_some_and(|r| r.send(request).is_ok())
    }

    /// Switches the quick panel to the network list and starts a scan.
    fn show_wifi(&mut self) {
        self.quick_view = QuickView::Wifi;
        self.armed = None;
        self.drag = None;
        self.wifi.scroll = 0.0;
        self.wifi.hover = None;
        if !self.wifi.busy {
            self.wifi.message = None;
        }
        self.wifi_send(wifi::Request::Refresh);
        self.wifi_scan();
        self.dirty_panel = true;
    }

    fn wifi_scan(&mut self) {
        if !self.wifi.scanning && !self.wifi.busy && self.wifi_send(wifi::Request::Scan) {
            self.wifi.scanning = true;
            self.dirty_panel = true;
        }
    }

    fn rebuild_wifi_rows(&mut self) {
        let snapshot = &self.wifi.snapshot;
        let current = if snapshot.status.connected() { snapshot.status.ssid.clone() } else { None };
        let mut rows: Vec<WifiRow> = snapshot
            .nets
            .iter()
            .map(|net| WifiRow {
                name: wifi::display_ssid(&net.ssid),
                signal: net.signal,
                security: net.security,
                known: snapshot.known.iter().find(|k| k.ssid == net.ssid).map(|k| k.id),
                current: current.as_ref() == Some(&net.ssid),
                ssid: net.ssid.clone(),
            })
            .collect();
        // The connected network first, the rest by strength (the order they arrive in).
        rows.sort_by_key(|row| !row.current);
        self.wifi.rows = rows;
        let most = (self.wifi.rows.len() as f32 * WIFI_ROW - self.wifi_layout().body.3).max(0.0);
        self.wifi.scroll = self.wifi.scroll.clamp(0.0, most);
    }

    /// News from the Wi-Fi worker thread.
    pub fn wifi_event(&mut self, event: wifi::Event) {
        match event {
            wifi::Event::Snapshot(snapshot) => {
                if snapshot == self.wifi.snapshot {
                    return;
                }
                self.wifi.snapshot = snapshot;
                self.rebuild_wifi_rows();
            }
            wifi::Event::ScanDone => self.wifi.scanning = false,
            wifi::Event::Associated => {
                self.wifi.message = Some((self.t(Msg::WifiGettingAddress).to_string(), false));
            }
            wifi::Event::Address(address) => {
                self.wifi.busy = false;
                self.wifi.message = Some(match address {
                    Some(address) => (fill(self.t(Msg::WifiConnectedAt), &[&address]), false),
                    None => (self.t(Msg::WifiNoAddress).to_string(), true),
                });
                self.net = sys::network_detail();
            }
            wifi::Event::Failed => {
                self.wifi.busy = false;
                self.wifi.message = Some((self.t(Msg::WifiFailed).to_string(), true));
                self.net = sys::network_detail();
            }
        }
        self.dirty_panel |= self.open == Some(Panel::Quick);
    }

    fn wifi_layout(&self) -> WifiLayout {
        let (pw, ph) = self.panel_size;
        let width = 384.0f32.min(pw - 20.0);
        let height = 440.0f32.min(ph - 16.0);
        let (x, y) = (pw - 10.0 - width, ph - 8.0 - height);
        let (left, inner) = (x + 14.0, width - 28.0);
        let top = y + 14.0 + 34.0 + 10.0;
        let footer_y = y + height - 10.0 - 24.0;
        let field = (left, top + 40.0, inner, 40.0);
        let buttons_y = field.1 + field.3 + 36.0;
        // "Показать" is longer than "显示" or "Show".
        let reveal_w = if lang::current() == Lang::Ru { 80.0 } else { 56.0 };
        WifiLayout {
            rect: (x, y, width, height),
            back: (left, y + 14.0, 34.0, 34.0),
            rescan: (left + inner - 34.0, y + 14.0, 34.0, 34.0),
            body: (left, top, inner, footer_y - 15.0 - top),
            field,
            reveal: (left + inner - 5.0 - reveal_w, field.1 + 5.0, reveal_w, 30.0),
            cancel: (left + inner - 96.0 * 2.0 - 8.0, buttons_y, 96.0, 36.0),
            connect: (left + inner - 96.0, buttons_y, 96.0, 36.0),
            footer_y,
        }
    }

    fn wifi_hit(&self, x: f32, y: f32) -> Option<WifiHit> {
        let layout = self.wifi_layout();
        if inside(layout.back, x, y) {
            return Some(WifiHit::Back);
        }
        if self.quick_view == QuickView::Password {
            return [(layout.reveal, WifiHit::Reveal), (layout.cancel, WifiHit::Cancel), (layout.connect, WifiHit::Connect)]
                .into_iter()
                .find(|(rect, _)| inside(*rect, x, y))
                .map(|(_, hit)| hit);
        }
        if inside(layout.rescan, x, y) {
            return Some(WifiHit::Rescan);
        }
        if inside(layout.body, x, y) {
            let row = ((y - layout.body.1 + self.wifi.scroll) / WIFI_ROW).floor() as usize;
            return (row < self.wifi.rows.len()).then_some(WifiHit::Row(row));
        }
        None
    }

    /// Back from the password form to the list.
    fn leave_password(&mut self) {
        self.wifi.wipe_password();
        self.wifi.target = None;
        self.wifi.demo = false;
        self.wifi.hover = None;
        self.quick_view = QuickView::Wifi;
        self.dirty_panel = true;
    }

    /// Back from the list to the panel's controls.
    fn leave_wifi(&mut self) {
        self.leave_password();
        self.quick_view = QuickView::Main;
        self.quick_hover = None;
        self.net = sys::network_detail();
    }

    fn wifi_connect(&mut self, target: wifi::Target, name: &str) {
        if self.wifi_send(wifi::Request::Connect(target)) {
            self.wifi.busy = true;
            self.wifi.message = Some((fill(self.t(Msg::WifiConnecting), &[name]), false));
        }
        self.dirty_panel = true;
    }

    fn wifi_choose(&mut self, row: usize) {
        if self.wifi.busy {
            return;
        }
        let Some(row) = self.wifi.rows.get(row) else { return };
        let (ssid, name, security, known, current) = (row.ssid.clone(), row.name.clone(), row.security, row.known, row.current);
        self.dirty_panel = true;
        if current {
            self.wifi.message = Some((self.t(Msg::WifiAlready).to_string(), false));
            return;
        }
        match (known, security) {
            (Some(id), _) => self.wifi_connect(wifi::Target::Known(id), &name),
            (None, Security::Open) => self.wifi_connect(wifi::Target::New { ssid, security, password: None }, &name),
            (None, Security::Unsupported) => {
                self.wifi.message = Some((self.t(Msg::WifiUnsupported).to_string(), true));
            }
            (None, _) => {
                self.wifi.wipe_password();
                self.wifi.target = Some((ssid, security));
                self.wifi.demo = false;
                self.wifi.hover = None;
                self.quick_view = QuickView::Password;
            }
        }
    }

    /// The password form's connect button (or Enter).
    fn wifi_submit(&mut self) {
        self.dirty_panel = true;
        if !wifi::valid_passphrase(&self.wifi.password) {
            self.wifi.bad_password = true;
            return;
        }
        let Some((ssid, security)) = self.wifi.target.take() else { return };
        let password = std::mem::take(&mut self.wifi.password);
        let demo = self.wifi.demo;
        self.leave_password();
        if demo {
            // Shown by a test command: nothing is sent anywhere.
            let mut bytes = password.into_bytes();
            bytes.fill(0);
            return;
        }
        let name = wifi::display_ssid(&ssid);
        self.wifi_connect(wifi::Target::New { ssid, security, password: Some(password) }, &name);
    }

    fn wifi_press(&mut self, x: f32, y: f32) {
        match self.wifi_hit(x, y) {
            Some(WifiHit::Back) if self.quick_view == QuickView::Password => self.leave_password(),
            Some(WifiHit::Back) => self.leave_wifi(),
            Some(WifiHit::Rescan) => self.wifi_scan(),
            Some(WifiHit::Row(row)) => self.wifi_choose(row),
            Some(WifiHit::Reveal) => {
                self.wifi.reveal = !self.wifi.reveal;
                self.dirty_panel = true;
            }
            Some(WifiHit::Cancel) => self.leave_password(),
            Some(WifiHit::Connect) => self.wifi_submit(),
            None => {}
        }
    }

    fn quick_key(&mut self, key: Key) -> Act {
        match (self.quick_view, key) {
            (QuickView::Main, Key::Escape) => return Act::Close,
            (QuickView::Main, _) => return Act::None,
            (QuickView::Wifi, Key::Escape) => self.leave_wifi(),
            (QuickView::Password, Key::Escape) => self.leave_password(),
            (QuickView::Password, Key::Enter) => self.wifi_submit(),
            (QuickView::Password, Key::Backspace) => {
                self.wifi.password.pop();
                self.wifi.bad_password = false;
            }
            (QuickView::Password, Key::ClearLine) => self.wifi.wipe_password(),
            (QuickView::Password, Key::Text(text)) => {
                if self.wifi.password.len() + text.len() <= 63 {
                    self.wifi.password.push_str(&text);
                    self.wifi.bad_password = false;
                }
            }
            _ => {}
        }
        self.dirty_panel = true;
        Act::None
    }

    fn draw_wifi(&mut self, c: &mut Canvas) {
        let layout = self.wifi_layout();
        let (x, y, w, h) = layout.rect;
        c.rrect(x, y, w, h, 20.0, PANEL);
        let (left, inner) = (x + 14.0, w - 28.0);
        let hover = self.wifi.hover;

        // Header: back arrow, title, rescan.
        let round = |c: &mut Canvas, rect: Rect, lit: bool| {
            c.circle(rect.0 + 17.0, rect.1 + 17.0, 17.0, CHIP);
            if lit {
                c.circle(rect.0 + 17.0, rect.1 + 17.0, 17.0, HOVER);
            }
        };
        round(c, layout.back, hover == Some(WifiHit::Back));
        c.shape(layout.back.0 + 9.0, layout.back.1 + 9.0, 16.0, 16.0, TEXT, |gx, gy| {
            let d = (gy - 8.0).abs();
            (d <= 5.0 && (gx - (3.0 + d)).abs() <= 1.0) || (d <= 0.85 && (3.0..=13.5).contains(&gx))
        });
        let mid = layout.back.1 + 17.0;
        c.text(&mut self.fonts, "Wi-Fi", layout.back.0 + 34.0 + 10.0, mid, 15.0, TEXT);
        if self.quick_view == QuickView::Wifi {
            round(c, layout.rescan, hover == Some(WifiHit::Rescan));
            let ink = if self.wifi.scanning || self.wifi.busy { TEXT2 } else { TEXT };
            c.shape(layout.rescan.0 + 8.0, layout.rescan.1 + 8.0, 18.0, 18.0, ink, |gx, gy| {
                // Three quarters of a ring with an arrow head where it ends.
                let (dx, dy) = (gx - 9.0, gy - 9.0);
                let d = (dx * dx + dy * dy).sqrt();
                let ring = (5.0..=6.6).contains(&d) && !(dx > 0.0 && dy < 0.0);
                let head = (9.0..=13.2).contains(&gx) && (gy - 3.2).abs() <= (13.2 - gx) * 0.75;
                ring || head
            });
            if self.wifi.scanning {
                let text = self.t(Msg::Scanning);
                let width = self.fonts.width(text, 12.0);
                c.text(&mut self.fonts, text, layout.rescan.0 - 10.0 - width, mid, 12.0, TEXT2);
            }
        }

        if self.quick_view == QuickView::Password {
            self.draw_password(c, &layout);
        } else {
            self.draw_wifi_rows(c, &layout);
        }

        // Bottom line: what is going on, else the present connection.
        c.rrect(left, layout.footer_y - 7.0, inner, 1.0, 0.0, CHIP);
        let (line, warn) = match &self.wifi.message {
            Some((text, warn)) => (text.clone(), *warn),
            None => {
                let status = &self.wifi.snapshot.status;
                match (status.connected(), &status.ssid, &status.ip) {
                    (true, Some(ssid), Some(ip)) => (format!("{} · {ip}", wifi::display_ssid(ssid)), false),
                    (true, Some(ssid), None) => (wifi::display_ssid(ssid), false),
                    _ => (self.t(Msg::NotConnected).to_string(), false),
                }
            }
        };
        let line = self.fonts.fit(&line, 11.5, inner - 8.0);
        c.text(&mut self.fonts, &line, left + 4.0, layout.footer_y + 12.0, 11.5, if warn { WARN } else { TEXT2 });
    }

    fn draw_wifi_rows(&mut self, c: &mut Canvas, layout: &WifiLayout) {
        let body = layout.body;
        if self.wifi.rows.is_empty() {
            let text = if self.wifi.scanning { self.t(Msg::LookingForNetworks) } else { self.t(Msg::NoNetworksFound) };
            c.text(&mut self.fonts, text, body.0 + 8.0, body.1 + 22.0, 13.0, TEXT2);
            return;
        }
        c.set_clip(Some(body));
        for i in 0..self.wifi.rows.len() {
            let ry = body.1 + i as f32 * WIFI_ROW - self.wifi.scroll;
            if ry + WIFI_ROW < body.1 || ry > body.1 + body.3 {
                continue;
            }
            let row = &self.wifi.rows[i];
            let (name, signal, security, known, current) = (row.name.clone(), row.signal, row.security, row.known, row.current);
            if current {
                c.rrect(body.0, ry + 2.0, body.2, WIFI_ROW - 4.0, 12.0, argb(0x2412C8C4));
            }
            if self.wifi.hover == Some(WifiHit::Row(i)) && !self.wifi.busy {
                c.rrect(body.0, ry + 2.0, body.2, WIFI_ROW - 4.0, 12.0, HOVER);
            }
            // Signal strength: four bars.
            let level = wifi::bars(signal);
            let ink = if current { ACCENT } else { TEXT };
            for bar in 0..4u32 {
                let height = 4.0 + bar as f32 * 3.0;
                let mut color = ink;
                if bar >= level {
                    color[3] = 60;
                }
                c.rrect(body.0 + 12.0 + bar as f32 * 4.5, ry + 31.0 - height, 3.0, height, 1.0, color);
            }
            let sub = match (current, known, security) {
                (true, _, _) => self.t(Msg::Connected),
                (_, Some(_), _) => self.t(Msg::Saved),
                (_, _, Security::Open) => self.t(Msg::OpenNetwork),
                (_, _, Security::Unsupported) => self.t(Msg::UnsupportedSecurity),
                _ => self.t(Msg::Secured),
            };
            let room = body.2 - 44.0 - 40.0;
            let name = self.fonts.fit(&name, 13.5, room);
            c.text(&mut self.fonts, &name, body.0 + 44.0, ry + 17.0, 13.5, if current { ACCENT } else { TEXT });
            c.text(&mut self.fonts, sub, body.0 + 44.0, ry + 33.0, 11.0, TEXT2);
            if security != Security::Open {
                // A padlock.
                c.shape(body.0 + body.2 - 28.0, ry + 16.0, 14.0, 16.0, TEXT2, |gx, gy| {
                    let body = (2.0..=12.0).contains(&gx) && (7.0..=14.5).contains(&gy);
                    let d = ((gx - 7.0).powi(2) + (gy - 6.5).powi(2)).sqrt();
                    let shackle = gy <= 7.0 && (3.0..=4.5).contains(&d);
                    body || shackle
                });
            }
        }
        c.set_clip(None);
        let content = self.wifi.rows.len() as f32 * WIFI_ROW;
        if content > body.3 {
            let thumb = (body.3 * body.3 / content).max(24.0);
            let offset = (body.3 - thumb) * self.wifi.scroll / (content - body.3).max(1.0);
            c.rrect(layout.rect.0 + layout.rect.2 - 8.0, body.1 + offset, 3.0, thumb, 1.5, CHIP);
        }
    }

    fn draw_password(&mut self, c: &mut Canvas, layout: &WifiLayout) {
        let body = layout.body;
        let name = self.wifi.target.as_ref().map(|(ssid, _)| wifi::display_ssid(ssid)).unwrap_or_default();
        let title = fill(self.t(Msg::ConnectTo), &[&name]);
        let title = self.fonts.fit(&title, 13.5, body.2 - 8.0);
        c.text(&mut self.fonts, &title, body.0 + 4.0, body.1 + 18.0, 13.5, TEXT);

        // The field: dots unless the owner asked to see the text.
        let (fx, fy, fw, fh) = layout.field;
        c.rrect(fx, fy, fw, fh, 20.0, CHIP);
        let mid = fy + fh / 2.0;
        let room = fw - 16.0 - layout.reveal.2 - 14.0;
        let caret = if self.wifi.password.is_empty() {
            let hint = self.t(Msg::Password);
            c.text(&mut self.fonts, hint, fx + 21.0, mid, 14.0, TEXT2);
            fx + 16.0
        } else {
            let mut shown: String =
                if self.wifi.reveal { self.wifi.password.clone() } else { "•".repeat(self.wifi.password.chars().count()) };
            // Too long for the field: show its end, where the typing happens.
            while self.fonts.width(&shown, 14.0) > room {
                shown.remove(0);
            }
            let width = c.text(&mut self.fonts, &shown, fx + 16.0, mid, 14.0, TEXT);
            let mut bytes = shown.into_bytes();
            bytes.fill(0);
            fx + 16.0 + width + 1.0
        };
        c.rrect(caret, mid - 9.0, 1.4, 18.0, 0.0, ACCENT);
        let (rx, ry, rw, rh) = layout.reveal;
        c.rrect(rx, ry, rw, rh, 15.0, if self.wifi.reveal { ACCENT } else { CHIP });
        if self.wifi.hover == Some(WifiHit::Reveal) {
            c.rrect(rx, ry, rw, rh, 15.0, HOVER);
        }
        let label = if self.wifi.reveal { self.t(Msg::Hide) } else { self.t(Msg::Show) };
        self.label_centered(c, label, rx + rw / 2.0, ry + rh / 2.0, 12.0, rw - 8.0, if self.wifi.reveal { ON_ACCENT } else { TEXT });

        let (hint, ink) = if self.wifi.bad_password {
            (self.t(Msg::BadPassword), WARN)
        } else {
            (self.t(Msg::PasswordHint), TEXT2)
        };
        c.text(&mut self.fonts, hint, fx + 4.0, fy + fh + 16.0, 11.5, ink);

        for (rect, hit, label, fill, ink) in [
            (layout.cancel, WifiHit::Cancel, self.t(Msg::Cancel), CHIP, TEXT),
            (layout.connect, WifiHit::Connect, self.t(Msg::Connect), ACCENT, ON_ACCENT),
        ] {
            c.rrect(rect.0, rect.1, rect.2, rect.3, 18.0, fill);
            if self.wifi.hover == Some(hit) {
                c.rrect(rect.0, rect.1, rect.2, rect.3, 18.0, if hit == WifiHit::Connect { argb(0x33FFFFFF) } else { HOVER });
            }
            self.label_centered(c, label, rect.0 + rect.2 / 2.0, rect.1 + rect.3 / 2.0, 13.0, rect.2 - 12.0, ink);
        }
    }

    /// Test command: the quick panel (already open) shows the network list and scans.
    pub fn test_wifi(&mut self) {
        if self.open == Some(Panel::Quick) {
            self.show_wifi();
        }
    }

    /// Test command: shows the password form without input. Its connect button does nothing.
    pub fn test_wifi_password(&mut self) {
        if self.open != Some(Panel::Quick) {
            return;
        }
        self.show_wifi();
        let row = self.wifi.rows.iter().find(|r| r.known.is_none() && matches!(r.security, Security::Psk | Security::Sae));
        self.wifi.target = Some(match row {
            Some(row) => (row.ssid.clone(), row.security),
            None => (self.t(Msg::ExampleNetwork).as_bytes().to_vec(), Security::Psk),
        });
        self.wifi.demo = true;
        self.quick_view = QuickView::Password;
    }
}

// ---------- volume / brightness indicator ----------

impl Ui {
    /// A value changed outside the shell (the volume or brightness keys): show it, and bring
    /// the quick panel's slider up to date.
    pub fn show_osd(&mut self, kind: OsdKind, percent: u32) {
        let percent = percent.min(100);
        self.osd = Some((kind, percent));
        self.dirty_osd = true;
        match kind {
            // The volume command reports 0 for "muted" as well as for "turned all the way down".
            OsdKind::Volume => self.volume = Some(Volume { percent, muted: percent == 0 }),
            OsdKind::Brightness => self.brightness = Some(percent),
        }
        self.dirty_panel |= self.open == Some(Panel::Quick);
    }

    pub fn draw_osd(&mut self, c: &mut Canvas) {
        self.dirty_osd = false;
        c.clear();
        let Some((kind, percent)) = self.osd else { return };
        let (w, h) = (OSD_SIZE.0 as f32, OSD_SIZE.1 as f32);
        c.rrect(0.0, 0.0, w, h, h / 2.0, argb(0xF2202124));
        let mid = h / 2.0;
        let muted = kind == OsdKind::Volume && percent == 0;
        c.shape(17.0, mid - 9.0, 18.0, 18.0, TEXT, move |gx, gy| match kind {
            OsdKind::Volume => speaker_shape(gx, gy, muted),
            OsdKind::Brightness => sun_shape(gx, gy),
        });
        let text = format!("{percent}%");
        let (start, length) = (46.0, w - 46.0 - 58.0);
        c.rrect(start, mid - 2.5, length, 5.0, 2.5, CHIP);
        if percent > 0 {
            c.rrect(start, mid - 2.5, (length * percent as f32 / 100.0).max(5.0), 5.0, 2.5, ACCENT);
        }
        let width = self.fonts.width(&text, 13.0);
        c.text(&mut self.fonts, &text, w - 16.0 - width, mid, 13.0, TEXT);
    }
}

// ---------- context menus ----------

impl Ui {
    fn build_menu(&mut self, title: String, app: Option<usize>, windows: &[u64], anchor: (f32, f32), from_shelf: bool) -> Menu {
        let mut items: Vec<(String, MenuAct)> = Vec::new();
        if let Some(index) = app {
            let id = self.apps[index].id.clone();
            let first = if from_shelf { self.t(Msg::NewWindow) } else { self.t(Msg::Open) };
            items.push((first.to_string(), MenuAct::Start(id.clone())));
            let pin = if self.pins.contains(&id) { self.t(Msg::Unpin) } else { self.t(Msg::Pin) };
            items.push((pin.to_string(), MenuAct::TogglePin(id.clone())));
            if !from_shelf {
                let hide =
                    if self.hidden.contains(&id) { self.t(Msg::ShowInLauncher) } else { self.t(Msg::HideFromLauncher) };
                items.push((hide.to_string(), MenuAct::ToggleHidden(id)));
            }
        }
        if !windows.is_empty() {
            let close = if windows.len() > 1 {
                Count::CloseWindows.text(lang::current(), windows.len() as u64, &[])
            } else {
                self.t(Msg::CloseWindow).to_string()
            };
            items.push((close, MenuAct::CloseWindows(windows.to_vec())));
        }
        let mut width = self.fonts.width(&title, 12.0);
        for (label, _) in &items {
            width = width.max(self.fonts.width(label, 13.0));
        }
        Menu { title, items, anchor, from_shelf, width: (width + 40.0).ceil().clamp(150.0, 320.0), hover: None }
    }

    fn shelf_menu(&mut self, item: usize) -> Menu {
        let x = self.shelf_layout().cells.get(item).copied().unwrap_or(0.0) + 22.0;
        let item = &self.shelf_items[item];
        let (title, app, windows) = (item.label.clone(), item.app, item.windows.clone());
        self.build_menu(title, app, &windows, (x, 0.0), true)
    }

    fn menu_rect(&self, menu: &Menu) -> Rect {
        let (pw, ph) = self.panel_size;
        let (w, h) = (menu.width, 6.0 + MENU_HEAD + menu.items.len() as f32 * MENU_ITEM + 6.0);
        let (x, y) = if menu.from_shelf {
            (menu.anchor.0 - w / 2.0, ph - 6.0 - h)
        } else if menu.anchor.1 + h > ph - 8.0 {
            (menu.anchor.0, menu.anchor.1 - h)
        } else {
            menu.anchor
        };
        (x.min(pw - w - 8.0).max(8.0), y.min(ph - h - 6.0).max(8.0), w, h)
    }

    fn menu_item_at(&self, x: f32, y: f32) -> Option<usize> {
        let menu = self.menu.as_ref()?;
        let rect = self.menu_rect(menu);
        if !inside(rect, x, y) || y < rect.1 + 6.0 + MENU_HEAD {
            return None;
        }
        let index = ((y - rect.1 - 6.0 - MENU_HEAD) / MENU_ITEM).floor() as usize;
        (index < menu.items.len()).then_some(index)
    }

    fn draw_menu(&mut self, c: &mut Canvas) {
        let Some(menu) = &self.menu else { return };
        let (x, y, w, h) = self.menu_rect(menu);
        let (title, hover) = (menu.title.clone(), menu.hover);
        let labels: Vec<String> = menu.items.iter().map(|(label, _)| label.clone()).collect();
        c.rrect(x - 1.0, y - 1.0, w + 2.0, h + 2.0, 15.0, argb(0x30FFFFFF));
        c.rrect(x, y, w, h, 14.0, MENU);
        let title = self.fonts.fit(&title, 12.0, w - 32.0);
        c.text(&mut self.fonts, &title, x + 16.0, y + 6.0 + MENU_HEAD / 2.0 - 1.0, 12.0, TEXT2);
        c.rrect(x + 10.0, y + 6.0 + MENU_HEAD - 3.0, w - 20.0, 1.0, 0.0, CHIP);
        for (i, label) in labels.iter().enumerate() {
            let iy = y + 6.0 + MENU_HEAD + i as f32 * MENU_ITEM;
            if hover == Some(i) {
                c.rrect(x + 6.0, iy + 1.0, w - 12.0, MENU_ITEM - 2.0, 9.0, HOVER);
            }
            c.text(&mut self.fonts, label, x + 16.0, iy + MENU_ITEM / 2.0, 13.0, TEXT);
        }
    }

    /// A press while a menu is up: on an entry it carries the entry out, anywhere else it only
    /// dismisses the menu.
    fn menu_press(&mut self, x: f32, y: f32, button: u32) -> Act {
        let hit = self.menu_item_at(x, y);
        let Some(menu) = self.menu.take() else { return Act::None };
        self.dirty_panel = true;
        let alone = self.open == Some(Panel::Menu);
        let dismissed = if alone { Act::Close } else { Act::None };
        if inside(self.menu_rect(&menu), x, y) && (hit.is_none() || button != BTN_LEFT) {
            // On the menu's own frame or heading, or not the left button: the menu stays.
            self.menu = Some(menu);
            return Act::None;
        }
        let Some(act) = hit.map(|i| menu.items[i].1.clone()) else { return dismissed };
        let app = |ui: &Ui, id: &str| ui.apps.iter().position(|a| a.id == id);
        match &act {
            MenuAct::Start(id) => {
                if let Some(index) = app(self, id) {
                    self.start(Target::App(index));
                }
                return Act::Close;
            }
            MenuAct::TogglePin(id) => {
                if let Some(index) = app(self, id) {
                    self.toggle_pin(index);
                }
            }
            MenuAct::ToggleHidden(id) => {
                match self.hidden.iter().position(|h| h == id) {
                    Some(position) => {
                        self.hidden.remove(position);
                    }
                    None => self.hidden.push(id.clone()),
                }
                sys::write_list("hidden", &self.hidden);
                if self.open == Some(Panel::Launcher) {
                    let scroll = self.scroll;
                    self.layout_launcher();
                    self.scroll = scroll.clamp(0.0, self.max_scroll());
                }
            }
            MenuAct::CloseWindows(ids) => ids.iter().for_each(|id| sys::close_window(*id)),
        }
        dismissed
    }

    /// Test command: prepares the menu of the shelf icon of this app id (as a right click would);
    /// true when there is such an icon and the menu panel should be opened.
    pub fn test_shelf_menu(&mut self, app_id: &str) -> bool {
        let wanted = app_id.to_lowercase();
        let position = self.shelf_items.iter().position(|i| i.key.to_lowercase() == wanted || i.key.to_lowercase() == format!("window:{wanted}"));
        if let Some(item) = position {
            self.menu_request = Some(self.shelf_menu(item));
        }
        position.is_some()
    }

    /// Test command: with the launcher open, shows the menu of this app's entry.
    pub fn test_launcher_menu(&mut self, app_id: &str) {
        if self.open != Some(Panel::Launcher) {
            return;
        }
        let found = self.entries.iter().position(|e| matches!(&e.target, Target::App(i) if self.apps[*i].id.eq_ignore_ascii_case(app_id)));
        let Some(entry) = found else { return };
        self.select(entry);
        let (view, rect) = (self.launcher_view(), self.entries[entry].rect);
        let anchor = (view.0 + rect.0 + rect.2 / 2.0, view.1 + rect.1 - self.scroll + rect.3 / 2.0);
        let Target::App(app) = self.entries[entry].target.clone() else { return };
        let title = self.name_of(&self.apps[app]);
        self.menu = Some(self.build_menu(title, Some(app), &[], anchor, false));
        self.dirty_panel = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_end_words() {
        assert_eq!(end_session_command("poweroff"), "echo poweroff > /run/mode; niri msg action quit --skip-confirmation");
        assert_eq!(QuickHit::Poweroff.mode(), Some("poweroff"));
        assert_eq!(QuickHit::Reboot.mode(), Some("reboot"));
        assert_eq!(QuickHit::Mac.mode(), Some("mac"));
        assert_eq!(QuickHit::Wifi.mode(), None);
        assert_eq!(QuickHit::Mute.mode(), None);
        // Restarting the desktop is the same mechanism with the word that means "start it again":
        // the session script then goes round its loop and exports the new LANG.
        assert_eq!(QuickHit::RestartDesktop.mode(), Some("desktop"));
        assert_eq!(end_session_command("desktop"), "echo desktop > /run/mode; niri msg action quit --skip-confirmation");
        assert_eq!(QuickHit::Lang.mode(), None);
        assert_eq!(QuickHit::LangPick(Lang::Ru).mode(), None);
    }
}
