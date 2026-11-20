//! shabnam-filechooser — the "Open file / Save as / Choose folder" window of the third-party programs.
//! Installed as /usr/local/bin/shabnam-filechooser, with /usr/local/bin/zenity as a link to it.
//!
//! Why it exists: PdfCraft, GridCraft, WordCraft, Simp, Gem Player and Ferrite all ask for a file
//! through the Rust crate rfd (its "xdg-portal" backend). rfd first asks the desktop portal over the
//! D-Bus session bus; when there is no bus it at once runs the program `zenity` instead and reads
//! the chosen path from its output. The user "shabnam" has no session bus on this system (and needs
//! none), so that second step is the one used — but the real zenity (and yad) depend on WebKitGTK,
//! a 120 MB second browser engine. This program is the part of zenity's command line those programs
//! use, drawn with the GTK 3 file chooser that is already installed (Chromium shows the same one).
//! Favnyr's "choose a program" also calls `zenity --file-selection`.
//!
//! Nothing stays running: the program lives as long as its window. It runs as whoever called it
//! (the unprivileged user), so it shows exactly the files that user can see. It talks to no bus.
//! GTK 3 is opened by name at run time (libgtk-3.so.0); nothing else is needed.
//!
//! Understood (zenity's own syntax, "--name value" or "--name=value"):
//!   --file-selection [--save] [--directory] [--multiple] [--confirm-overwrite] [--filename PATH]
//!                    [--file-filter "Name | *.a *.b"]... [--title T] [--separator S]
//!       prints the chosen path(s), separated by "|"; exit 0. Cancelled: prints nothing, exit 1.
//!   --info | --warning | --error | --question  [--title T] [--text T] [--ok-label L]
//!                    [--cancel-label L] [--extra-button L]...
//!       exit 0 for OK / Yes, 1 for Cancel / No / closed; an extra button prints its label, exit 1.
//! Anything else: a message on stderr and exit 255, like zenity after a wrong option.
//!
//! Where the window starts: the folder of --filename if it names one; else the calling program's
//! working directory if that is not just the home folder (rfd 0.18 passes its start folder this
//! way); else the folder last chosen from; else the home folder.
//! Language: the window's own words come from GTK's catalogs (LANG); the three titles are below.
//! GTK's own choices (hidden files shown or not, sort order, window size) are kept in
//! ~/.config/glib-2.0/settings/keyfile.

use std::env;
use std::ffi::{c_char, c_int, c_void, CStr, CString, OsStr, OsString};
use std::io::Write;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::exit;

#[derive(Debug, PartialEq, Clone, Copy)]
enum Mode {
    Files,
    Info,
    Warning,
    Error,
    Question,
}

#[derive(Debug, Default, PartialEq)]
struct Request {
    mode: Option<Mode>,
    save: bool,
    directory: bool,
    multiple: bool,
    filename: Option<OsString>,
    filters: Vec<String>,
    title: Option<String>,
    separator: Option<String>,
    text: Option<String>,
    ok_label: Option<String>,
    cancel_label: Option<String>,
    extra_buttons: Vec<String>,
}

/// Options that take a value and are used.
const VALUES: &[&str] =
    &["--filename", "--file-filter", "--title", "--separator", "--text", "--ok-label", "--cancel-label", "--extra-button"];
/// Options that take a value and are accepted but not used.
const UNUSED_VALUES: &[&str] =
    &["--width", "--height", "--icon", "--icon-name", "--window-icon", "--timeout", "--class", "--name", "--attach"];
/// Options without a value that are accepted but change nothing (text is never read as markup).
const UNUSED_FLAGS: &[&str] = &["--no-markup", "--no-wrap", "--modal", "--ellipsize", "--default-cancel", "--confirm-overwrite"];

impl Request {
    fn set(&mut self, name: &str, value: OsString) {
        let text = value.to_string_lossy().into_owned();
        match name {
            "--filename" => self.filename = Some(value),
            "--file-filter" => self.filters.push(text),
            "--title" => self.title = Some(text),
            "--separator" => self.separator = Some(text),
            "--text" => self.text = Some(text),
            "--ok-label" => self.ok_label = Some(text),
            "--cancel-label" => self.cancel_label = Some(text),
            "--extra-button" => self.extra_buttons.push(text),
            _ => {}
        }
    }
}

fn parse(args: Vec<OsString>) -> Result<Request, String> {
    let mut req = Request::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let bytes = arg.as_bytes();
        let (name, inline) = match bytes.iter().position(|&b| b == b'=') {
            Some(at) if bytes.starts_with(b"--") => (&bytes[..at], Some(OsString::from_vec(bytes[at + 1..].to_vec()))),
            _ => (bytes, None),
        };
        let name = String::from_utf8_lossy(name).into_owned();
        let mode = match name.as_str() {
            "--file-selection" => Some(Mode::Files),
            "--info" => Some(Mode::Info),
            "--warning" => Some(Mode::Warning),
            "--error" => Some(Mode::Error),
            "--question" => Some(Mode::Question),
            _ => None,
        };
        if let (Some(mode), None) = (mode, &inline) {
            if req.mode.is_some_and(|m| m != mode) {
                return Err("two kinds of window asked for".into());
            }
            req.mode = Some(mode);
            continue;
        }
        let takes_value = VALUES.contains(&name.as_str()) || UNUSED_VALUES.contains(&name.as_str());
        match (name.as_str(), inline) {
            ("--save", None) => req.save = true,
            ("--directory", None) => req.directory = true,
            ("--multiple", None) => req.multiple = true,
            (n, None) if UNUSED_FLAGS.contains(&n) => {}
            (_, Some(value)) if takes_value => req.set(&name, value),
            (_, None) if takes_value => match args.next() {
                Some(value) => req.set(&name, value),
                None => return Err(format!("{name} needs a value")),
            },
            _ => return Err(format!("option not supported: {}", arg.to_string_lossy())),
        }
    }
    if req.mode.is_none() {
        return Err("no window asked for (--file-selection, --info, --warning, --error or --question)".into());
    }
    Ok(req)
}

/// "Pictures | *.png *.jpg" -> the name and the patterns, each also in lower and upper case:
/// GTK compares letter for letter, and PHOTO.JPG must pass a "*.jpg" filter too.
fn filter(spec: &str) -> Option<(String, Vec<String>)> {
    let (name, patterns) = spec.split_once('|').unwrap_or((spec, spec));
    let mut all: Vec<String> = Vec::new();
    for pattern in patterns.split_whitespace() {
        // rfd 0.17 writes "every file" as "*.*", which would hide names without a dot.
        let pattern = if pattern == "*.*" { "*" } else { pattern };
        for variant in [pattern.to_string(), pattern.to_lowercase(), pattern.to_uppercase()] {
            if !all.contains(&variant) {
                all.push(variant);
            }
        }
    }
    if all.is_empty() {
        return None;
    }
    let name = name.trim();
    Some((if name.is_empty() { patterns.trim().to_string() } else { name.to_string() }, all))
}

/// The folder the window starts in and, from --filename, the file name to put in it.
fn start(filename: Option<&OsStr>, cwd: &Path, home: &Path, remembered: Option<PathBuf>) -> (PathBuf, Option<OsString>) {
    let mut folder = None;
    let mut name = None;
    if let Some(given) = filename.map(Path::new) {
        if given.is_dir() {
            folder = Some(given.to_path_buf());
        } else {
            name = given.file_name().map(OsStr::to_os_string);
            folder = given.parent().filter(|p| !p.as_os_str().is_empty() && p.is_dir()).map(Path::to_path_buf);
        }
    }
    let same = |a: &Path, b: &Path| a.canonicalize().ok().is_some_and(|a| Some(a) == b.canonicalize().ok());
    let folder = folder
        .or_else(|| (!same(cwd, home)).then(|| cwd.to_path_buf()))
        .or_else(|| remembered.filter(|p| p.is_dir()))
        .unwrap_or_else(|| home.to_path_buf());
    let folder = if folder.is_absolute() { folder } else { cwd.join(folder) };
    (folder, name)
}

fn language() -> &'static str {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|v| env::var(v).ok().filter(|s| !s.is_empty()));
    match locale.as_deref().map(|l| l.get(..2).unwrap_or("")) {
        Some("zh") => "zh",
        Some("ru") => "ru",
        _ => "en",
    }
}

fn title(kind: &str) -> &'static str {
    match (kind, language()) {
        ("open", "zh") => "打开文件",
        ("open", "ru") => "Открыть файл",
        ("open", _) => "Open file",
        ("save", "zh") => "另存为",
        ("save", "ru") => "Сохранить как",
        ("save", _) => "Save as",
        (_, "zh") => "选择文件夹",
        (_, "ru") => "Выбрать папку",
        _ => "Choose folder",
    }
}

fn fail(text: &str) -> ! {
    eprintln!("shabnam-filechooser: {text}");
    exit(255)
}

fn c(text: impl AsRef<OsStr>) -> CString {
    // A NUL cannot be inside a command-line argument or a path; cut there if one ever is.
    let bytes = text.as_ref().as_bytes();
    CString::new(&bytes[..bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len())]).unwrap()
}

extern "C" {
    fn dlopen(name: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}
const RTLD_NOW_GLOBAL: c_int = 0x2 | 0x100;

type Widget = *mut c_void;
type Str = *const c_char;

/// One GTK function, looked up by name in the opened library (or in what it depends on: GLib).
macro_rules! gtk {
    ($lib:expr, $name:literal, fn($($arg:ty),*) $(-> $ret:ty)?) => {{
        let p = dlsym($lib, concat!($name, "\0").as_ptr().cast());
        if p.is_null() {
            fail(concat!("GTK 3 has no ", $name));
        }
        std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($arg),*) $(-> $ret)?>(p)
    }};
}

const RESPONSE_ACCEPT: c_int = -3;
const RESPONSE_OK: c_int = -5;
const RESPONSE_CANCEL: c_int = -6;
const FIRST_EXTRA: c_int = 100;

/// GdkGeometry: four sizes we set, then five more integers, two doubles and a gravity we leave zero
/// (kept as nine 8-byte words, which is at least their size and has their alignment).
#[repr(C)]
struct GdkGeometry {
    min_width: c_int,
    min_height: c_int,
    max_width: c_int,
    max_height: c_int,
    rest: [u64; 9],
}
const HINT_MIN_SIZE: c_int = 1 << 1;
const HINT_MAX_SIZE: c_int = 1 << 2;

#[repr(C)]
struct GSList {
    data: *mut c_void,
    next: *mut GSList,
}

/// Opens GTK and connects to the display. Returns the library handle.
unsafe fn gtk_start() -> *mut c_void {
    // No accessibility bus, no portal: this user has no D-Bus session and the window must not wait
    // for one. Settings in a plain file instead of dconf (which is a D-Bus service).
    env::set_var("NO_AT_BRIDGE", "1");
    env::set_var("GTK_USE_PORTAL", "0");
    if env::var_os("GDK_BACKEND").is_none() {
        env::set_var("GDK_BACKEND", "wayland");
    }
    if env::var_os("GSETTINGS_BACKEND").is_none() {
        env::set_var("GSETTINGS_BACKEND", "keyfile");
    }
    let lib = dlopen(c"libgtk-3.so.0".as_ptr(), RTLD_NOW_GLOBAL);
    if lib.is_null() {
        fail("libgtk-3.so.0 (package gtk+3) cannot be opened");
    }
    // The Wayland app id (for a window rule in niri.kdl).
    gtk!(lib, "g_set_prgname", fn(Str))(c"shabnam-filechooser".as_ptr());
    if gtk!(lib, "gtk_init_check", fn(*mut c_int, *mut c_void) -> c_int)(std::ptr::null_mut(), std::ptr::null_mut()) == 0 {
        fail("cannot open a window (no display)");
    }
    lib
}

/// A word of GTK's own, in the session's language ("_Open" -> "打开(_O)").
unsafe fn word(lib: *mut c_void, text: &CStr) -> Str {
    gtk!(lib, "g_dgettext", fn(Str, Str) -> Str)(c"gtk30".as_ptr(), text.as_ptr())
}

unsafe fn message(req: &Request, mode: Mode) -> i32 {
    let lib = gtk_start();
    let kind: c_int = match mode {
        Mode::Info => 0,
        Mode::Warning => 1,
        Mode::Question => 2,
        _ => 3,
    };
    let p = dlsym(lib, c"gtk_message_dialog_new".as_ptr());
    if p.is_null() {
        fail("GTK 3 has no gtk_message_dialog_new");
    }
    let new = std::mem::transmute::<*mut c_void, unsafe extern "C" fn(Widget, c_int, c_int, c_int, Str, ...) -> Widget>(p);
    let text = c(req.text.as_deref().unwrap_or(""));
    let dialog = new(std::ptr::null_mut(), 0, kind, 0, c"%s".as_ptr(), text.as_ptr());
    gtk!(lib, "gtk_window_set_title", fn(Widget, Str))(dialog, c(req.title.as_deref().unwrap_or("")).as_ptr());
    let add = gtk!(lib, "gtk_dialog_add_button", fn(Widget, Str, c_int) -> Widget);
    for (n, label) in req.extra_buttons.iter().enumerate() {
        add(dialog, c(label).as_ptr(), FIRST_EXTRA + n as c_int);
    }
    let label = |given: &Option<String>, own: &CStr| match given {
        Some(text) => c(text),
        None => CStr::from_ptr(word(lib, own)).to_owned(),
    };
    if mode == Mode::Question {
        add(dialog, label(&req.cancel_label, c"_No").as_ptr(), RESPONSE_CANCEL);
        add(dialog, label(&req.ok_label, c"_Yes").as_ptr(), RESPONSE_OK);
    } else {
        add(dialog, label(&req.ok_label, c"_OK").as_ptr(), RESPONSE_OK);
    }
    gtk!(lib, "gtk_dialog_set_default_response", fn(Widget, c_int))(dialog, RESPONSE_OK);
    let answer = gtk!(lib, "gtk_dialog_run", fn(Widget) -> c_int)(dialog);
    gtk!(lib, "gtk_widget_destroy", fn(Widget))(dialog);
    if answer == RESPONSE_OK {
        return 0;
    }
    if let Some(label) = usize::try_from(answer - FIRST_EXTRA).ok().and_then(|n| req.extra_buttons.get(n)) {
        println!("{label}");
    }
    1
}

unsafe fn files(req: &Request) -> i32 {
    let home = PathBuf::from(env::var_os("HOME").filter(|h| !h.is_empty()).unwrap_or_else(|| "/".into()));
    let memo = env::var_os("XDG_CACHE_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".cache"))
        .join("shabnam-filechooser-folder");
    let remembered = std::fs::read(&memo).ok().map(|b| PathBuf::from(OsString::from_vec(b.trim_ascii_end().to_vec())));
    let cwd = env::current_dir().unwrap_or_else(|_| home.clone());
    let (folder, name) = start(req.filename.as_deref(), &cwd, &home, remembered);

    let lib = gtk_start();
    let (action, kind, accept): (c_int, &str, &CStr) = if req.directory {
        (2, "folder", c"_Select")
    } else if req.save {
        (1, "save", c"_Save")
    } else {
        (0, "open", c"_Open")
    };
    let p = dlsym(lib, c"gtk_file_chooser_dialog_new".as_ptr());
    if p.is_null() {
        fail("GTK 3 has no gtk_file_chooser_dialog_new");
    }
    let new = std::mem::transmute::<*mut c_void, unsafe extern "C" fn(Str, Widget, c_int, Str, ...) -> Widget>(p);
    let heading = c(req.title.as_deref().unwrap_or(title(kind)));
    let dialog = new(heading.as_ptr(), std::ptr::null_mut(), action, std::ptr::null::<c_char>());
    let add = gtk!(lib, "gtk_dialog_add_button", fn(Widget, Str, c_int) -> Widget);
    add(dialog, word(lib, c"_Cancel"), RESPONSE_CANCEL);
    add(dialog, word(lib, accept), RESPONSE_ACCEPT);
    gtk!(lib, "gtk_dialog_set_default_response", fn(Widget, c_int))(dialog, RESPONSE_ACCEPT);
    gtk!(lib, "gtk_file_chooser_set_local_only", fn(Widget, c_int))(dialog, 1);
    gtk!(lib, "gtk_file_chooser_set_select_multiple", fn(Widget, c_int))(dialog, (req.multiple && !req.save) as c_int);
    gtk!(lib, "gtk_file_chooser_set_do_overwrite_confirmation", fn(Widget, c_int))(dialog, 1);
    // A window of one fixed size (smallest = largest = 900x600): niri opens such a window floating
    // in the middle of the screen, as a dialog should be, instead of as one more column (the window
    // has no parent to float over: it belongs to another process). 900x600 fits the laptop's
    // 1280x720 points with the shelf.
    let size = GdkGeometry { min_width: 900, min_height: 600, max_width: 900, max_height: 600, rest: [0; 9] };
    gtk!(lib, "gtk_window_set_default_size", fn(Widget, c_int, c_int))(dialog, 900, 600);
    gtk!(lib, "gtk_window_set_geometry_hints", fn(Widget, Widget, *const GdkGeometry, c_int))(
        dialog,
        std::ptr::null_mut(),
        &size,
        HINT_MIN_SIZE | HINT_MAX_SIZE,
    );
    // The user's folders, one click away in the side bar (GTK itself lists only home and desktop).
    for shortcut in ["Documents", "Downloads", "Pictures", "Music", "Videos"] {
        if home.join(shortcut).is_dir() {
            gtk!(lib, "gtk_file_chooser_add_shortcut_folder", fn(Widget, Str, *mut c_void) -> c_int)(
                dialog,
                c(home.join(shortcut)).as_ptr(),
                std::ptr::null_mut(),
            );
        }
    }
    for (name, patterns) in req.filters.iter().filter_map(|spec| filter(spec)) {
        let rule = gtk!(lib, "gtk_file_filter_new", fn() -> *mut c_void)();
        gtk!(lib, "gtk_file_filter_set_name", fn(*mut c_void, Str))(rule, c(&name).as_ptr());
        for pattern in &patterns {
            gtk!(lib, "gtk_file_filter_add_pattern", fn(*mut c_void, Str))(rule, c(pattern).as_ptr());
        }
        gtk!(lib, "gtk_file_chooser_add_filter", fn(Widget, *mut c_void))(dialog, rule);
    }
    gtk!(lib, "gtk_file_chooser_set_current_folder", fn(Widget, Str) -> c_int)(dialog, c(&folder).as_ptr());
    if let Some(name) = &name {
        if req.save {
            // The name is shown as text: GTK wants UTF-8 here.
            gtk!(lib, "gtk_file_chooser_set_current_name", fn(Widget, Str))(dialog, c(name.to_string_lossy().as_ref()).as_ptr());
        } else if folder.join(name).exists() {
            gtk!(lib, "gtk_file_chooser_set_filename", fn(Widget, Str) -> c_int)(dialog, c(folder.join(name)).as_ptr());
        }
    }

    let answer = gtk!(lib, "gtk_dialog_run", fn(Widget) -> c_int)(dialog);
    let free = gtk!(lib, "g_free", fn(*mut c_void));
    let mut chosen: Vec<OsString> = Vec::new();
    if answer == RESPONSE_ACCEPT {
        let list = gtk!(lib, "gtk_file_chooser_get_filenames", fn(Widget) -> *mut GSList)(dialog);
        let mut item = list;
        while !item.is_null() {
            let path = (*item).data as *mut c_char;
            if !path.is_null() {
                chosen.push(OsString::from_vec(CStr::from_ptr(path).to_bytes().to_vec()));
                free(path.cast());
            }
            item = (*item).next;
        }
        gtk!(lib, "g_slist_free", fn(*mut GSList))(list);
    }
    let current = gtk!(lib, "gtk_file_chooser_get_current_folder", fn(Widget) -> *mut c_char)(dialog);
    let current = (!current.is_null()).then(|| {
        let path = PathBuf::from(OsString::from_vec(CStr::from_ptr(current).to_bytes().to_vec()));
        free(current.cast());
        path
    });
    gtk!(lib, "gtk_widget_destroy", fn(Widget))(dialog);
    chosen.retain(|p| !p.is_empty());
    if chosen.is_empty() {
        return 1;
    }
    // Remembered for the next window: a file of the user's own, replaced in one step.
    let first = Path::new(&chosen[0]);
    if let Some(folder) = current.or_else(|| if req.directory { Some(first.to_path_buf()) } else { first.parent().map(Path::to_path_buf) }) {
        remember(&memo, &folder);
    }
    let separator = req.separator.as_deref().unwrap_or("|").as_bytes();
    let mut out = chosen.iter().map(|p| p.as_bytes()).collect::<Vec<_>>().join(separator);
    out.push(b'\n');
    if std::io::stdout().write_all(&out).is_err() {
        return 1;
    }
    0
}

fn remember(memo: &Path, folder: &Path) {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let Some(dir) = memo.parent() else { return };
    let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir);
    let new = memo.with_extension("new");
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&new)
        .and_then(|mut f| f.write_all(&[folder.as_os_str().as_bytes(), b"\n"].concat()));
    if written.is_ok() {
        let _ = std::fs::rename(&new, memo);
    } else {
        let _ = std::fs::remove_file(&new);
    }
}

fn main() {
    let req = parse(env::args_os().skip(1).collect()).unwrap_or_else(|e| fail(&e));
    let code = unsafe {
        match req.mode {
            Some(Mode::Files) => files(&req),
            Some(mode) => message(&req, mode),
            None => 255,
        }
    };
    let _ = std::io::stdout().flush();
    exit(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn what_rfd_sends() {
        // rfd 0.17: zenity --no-markup --file-selection --save --confirm-overwrite --file-filter .. --filename ..
        let r = parse(args(&[
            "--no-markup", "--file-selection", "--save", "--confirm-overwrite", "--file-filter", "PDF | *.pdf",
            "--file-filter", "All | *", "--filename", "a b.pdf",
        ]))
        .unwrap();
        assert_eq!(r.mode, Some(Mode::Files));
        assert!(r.save && !r.directory && !r.multiple);
        assert_eq!(r.filters, ["PDF | *.pdf", "All | *"]);
        assert_eq!(r.filename.as_deref(), Some(OsStr::new("a b.pdf")));
        let r = parse(args(&["--no-markup", "--file-selection", "--directory", "--multiple"])).unwrap();
        assert!(r.directory && r.multiple);
    }

    #[test]
    fn what_favnyr_sends() {
        let r = parse(args(&["--file-selection", "--title=选择程序"])).unwrap();
        assert_eq!(r.title.as_deref(), Some("选择程序"));
    }

    #[test]
    fn questions() {
        let r = parse(args(&[
            "--no-markup", "--question", "--title", "t", "--text", "a=b", "--ok-label", "Y", "--cancel-label", "C", "--extra-button", "N",
        ]))
        .unwrap();
        assert_eq!(r.mode, Some(Mode::Question));
        assert_eq!((r.text.as_deref(), r.ok_label.as_deref(), r.cancel_label.as_deref()), (Some("a=b"), Some("Y"), Some("C")));
        assert_eq!(r.extra_buttons, ["N"]);
    }

    #[test]
    fn refused() {
        assert!(parse(args(&["--calendar"])).is_err());
        assert!(parse(args(&["--file-selection", "--frob"])).is_err());
        assert!(parse(args(&["--file-selection", "--info"])).is_err());
        assert!(parse(args(&["--file-selection", "--filename"])).is_err());
        assert!(parse(args(&["--save"])).is_err());
        assert!(parse(args(&[])).is_err());
    }

    #[test]
    fn filters() {
        assert_eq!(filter("Pictures | *.png *.JPG"), Some(("Pictures".into(), vec!["*.png".into(), "*.PNG".into(), "*.JPG".into(), "*.jpg".into()])));
        assert_eq!(filter("*.txt"), Some(("*.txt".into(), vec!["*.txt".into(), "*.TXT".into()])));
        assert_eq!(filter("All | *"), Some(("All".into(), vec!["*".into()])));
        assert_eq!(filter("All files | *.*"), Some(("All files".into(), vec!["*".into()])));
        assert_eq!(filter("Nothing | "), None);
    }

    #[test]
    fn where_it_starts() {
        let tmp = env::temp_dir();
        let home = Path::new("/");
        // --filename with a folder that exists: that folder and the name.
        let given = tmp.join("x.pdf");
        assert_eq!(start(Some(given.as_os_str()), home, home, None), (tmp.clone(), Some("x.pdf".into())));
        // only a name: the working directory when it is not the home folder, else what was remembered, else home.
        assert_eq!(start(Some(OsStr::new("x.pdf")), &tmp, home, None), (tmp.clone(), Some("x.pdf".into())));
        assert_eq!(start(None, home, home, Some(tmp.clone())), (tmp.clone(), None));
        assert_eq!(start(None, home, home, Some("/no/such/folder".into())), (home.to_path_buf(), None));
        assert_eq!(start(Some(tmp.as_os_str()), home, home, None), (tmp, None));
    }
}
