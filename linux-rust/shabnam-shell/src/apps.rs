//! Installed applications: reading .desktop files, filtering, and turning Exec into a command line.
//! Depends on std only, so `rustc --test src/apps.rs` checks it on any machine.

use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct App {
    /// File name without ".desktop".
    pub id: String,
    pub name: String,
    /// Name[zh_CN] (or Name[zh]) when the file has one that differs from `name`.
    pub name_zh: Option<String>,
    /// GenericName, else Comment.
    pub hint: String,
    /// Exec split into program and arguments, %-codes removed. Started directly, never through a shell.
    pub argv: Vec<String>,
    /// `argv` joined with spaces, for searching and messages.
    pub command: String,
    pub terminal: bool,
    pub dir: Option<String>,
    /// Icon= as written: a name to look up in the icon directories, or a full path.
    pub icon: String,
    /// StartupWMClass=, which is often what a running window reports as its app id.
    pub wm_class: String,
    try_exec: Option<String>,
}

fn truthy(v: &str) -> bool {
    v.eq_ignore_ascii_case("true") || v == "1"
}

/// Splits a desktop-entry Exec value into program and arguments, following the specification's
/// quoting (double quotes, backslash escapes inside them) and dropping the field codes:
/// "%f", "%U", "%i" ... disappear, "%%" becomes "%".
pub fn parse_exec(exec: &str) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let mut current = String::new();
    let (mut quoted, mut started) = (false, false);
    let mut it = exec.chars();
    while let Some(c) = it.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => {
                current.extend(it.next());
            }
            '%' => match it.next() {
                Some('%') => current.push('%'),
                // Any other code (and a stray "%" at the end) is dropped.
                _ => {}
            },
            c if c.is_whitespace() && !quoted => {
                if started || !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
                started = false;
            }
            c => current.push(c),
        }
    }
    if started || !current.is_empty() {
        args.push(current);
    }
    // `"%f"` in quotes would leave an empty argument: drop those.
    args.retain(|a| !a.is_empty());
    args
}

/// Parses one .desktop file. None means "do not show": not an application, hidden, or nothing to run.
pub fn parse_desktop(id: &str, text: &str) -> Option<App> {
    let mut app = App { id: id.to_string(), ..Default::default() };
    let (mut in_entry, mut hidden, mut generic, mut comment) = (false, false, String::new(), String::new());
    let (mut zh_cn, mut zh) = (None, None);
    let mut kind = String::from("Application");
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "Type" => kind = value.to_string(),
            "Name" => app.name = value.to_string(),
            "Name[zh_CN]" => zh_cn = Some(value.to_string()),
            "Name[zh]" => zh = Some(value.to_string()),
            "GenericName" => generic = value.to_string(),
            "Comment" => comment = value.to_string(),
            "Exec" => app.argv = parse_exec(value),
            "TryExec" => app.try_exec = Some(value.to_string()),
            "Icon" => app.icon = value.to_string(),
            "StartupWMClass" => app.wm_class = value.to_string(),
            "Path" if !value.is_empty() => app.dir = Some(value.to_string()),
            "Terminal" => app.terminal = truthy(value),
            "NoDisplay" | "Hidden" => hidden |= truthy(value),
            _ => {}
        }
    }
    if hidden || kind != "Application" || app.argv.is_empty() {
        return None;
    }
    app.command = app.argv.join(" ");
    if app.name.is_empty() {
        app.name = id.to_string();
    }
    app.name_zh = zh_cn.or(zh).filter(|z| !z.is_empty() && *z != app.name);
    app.hint = if generic.is_empty() { comment } else { generic };
    Some(app)
}

impl App {
    /// The program's file name: "/usr/bin/hx --foo" gives "hx".
    pub fn program(&self) -> &str {
        let first = self.argv.first().map(String::as_str).unwrap_or("");
        first.rsplit('/').next().unwrap_or(first)
    }
}

/// Finds the app a running window belongs to, from the app id the window reports.
pub fn match_window(apps: &[App], window_app_id: &str) -> Option<usize> {
    let w = window_app_id.to_lowercase();
    if w.is_empty() {
        return None;
    }
    let exact = |a: &App| {
        a.id.to_lowercase() == w || a.wm_class.to_lowercase() == w || a.program() == w || a.icon.to_lowercase() == w
    };
    // "chromium-browser" for chromium, "org.example.Foo" for foo.desktop and the like.
    let loose = |a: &App| {
        let (id, program) = (a.id.to_lowercase(), a.program().to_lowercase());
        (!program.is_empty() && w.starts_with(&format!("{program}-")))
            || w.ends_with(&format!(".{id}"))
            || id.ends_with(&format!(".{w}"))
    };
    apps.iter().position(exact).or_else(|| apps.iter().position(loose))
}

/// A readable name for a window whose app id no desktop file claims: the last meaningful part
/// of the id, its words capitalised. "org.gnome.Nautilus" gives "Nautilus",
/// "org.freedesktop.weston.wayland-terminal" gives "Wayland Terminal", "foot" gives "Foot".
pub fn name_from_app_id(app_id: &str) -> String {
    let part = app_id
        .rsplit('.')
        .map(str::trim)
        .find(|p| !p.is_empty() && !matches!(p.to_lowercase().as_str(), "desktop" | "exe" | "bin"))
        .unwrap_or("");
    let words: Vec<String> = part
        .split(|c: char| c == '-' || c == '_' || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
        })
        .collect();
    words.join(" ")
}

/// True when `program` (a path, or a name looked up in `path_var`) is an existing file.
pub fn program_exists(program: &str, path_var: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    path_var.split(':').filter(|d| !d.is_empty()).any(|d| Path::new(d).join(program).is_file())
}

/// Reads every *.desktop in `dirs`; an id found in an earlier directory hides the same id later on.
pub fn load(dirs: &[String], path_var: &str) -> Vec<App> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for entry in entries.flatten() {
            let file = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = file.strip_suffix(".desktop") else { continue };
            if !seen.insert(id.to_string()) {
                continue;
            }
            let Ok(bytes) = std::fs::read(entry.path()) else { continue };
            let Some(app) = parse_desktop(id, &String::from_utf8_lossy(&bytes)) else { continue };
            if app.try_exec.as_deref().is_some_and(|t| !program_exists(t, path_var)) {
                continue;
            }
            apps.push(app);
        }
    }
    apps.sort_by_key(|a| (a.name.to_lowercase(), a.id.clone()));
    apps
}

/// Indices of the apps matching `query` (case-insensitive substring), best matches first:
/// name starts with it, name contains it, then anything else (Chinese name, hint, command, id).
pub fn filter(apps: &[App], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..apps.len()).collect();
    }
    let mut hits: Vec<(u8, usize)> = Vec::new();
    for (i, a) in apps.iter().enumerate() {
        let name = a.name.to_lowercase();
        let rank = if name.starts_with(&q) {
            0
        } else if name.contains(&q) {
            1
        } else if a.name_zh.as_deref().is_some_and(|z| z.to_lowercase().contains(&q))
            || a.hint.to_lowercase().contains(&q)
            || a.command.to_lowercase().contains(&q)
            || a.id.to_lowercase().contains(&q)
        {
            2
        } else {
            continue;
        };
        hits.push((rank, i));
    }
    hits.sort();
    hits.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_lines() {
        assert_eq!(parse_exec("yazi %f"), ["yazi"]);
        assert_eq!(parse_exec("wezterm start --cwd ."), ["wezterm", "start", "--cwd", "."]);
        assert_eq!(parse_exec("sh -c \"echo 100%% \\\"done\\\"\" %U --x"), ["sh", "-c", "echo 100% \"done\"", "--x"]);
        assert_eq!(parse_exec("app --file=%f \"%F\" %"), ["app", "--file="]);
        assert_eq!(parse_exec("\"/opt/My App/run\"   a\tb"), ["/opt/My App/run", "a", "b"]);
        assert!(parse_exec("  %u ").is_empty());
    }

    #[test]
    fn parse() {
        let text = "[Desktop Entry]\nName=Yazi File Manager\nName[zh_CN]=文件管理器\nTerminal=true\n\
                    Exec=yazi %f\nType=Application\nComment=Fast\n[Desktop Action New]\nName=Other\nExec=other\n";
        let a = parse_desktop("yazi", text).unwrap();
        assert_eq!(a.name, "Yazi File Manager");
        assert_eq!(a.name_zh.as_deref(), Some("文件管理器"));
        assert_eq!(a.command, "yazi");
        assert!(a.terminal);
        assert_eq!(a.hint, "Fast");
        assert!(parse_desktop("x", "[Desktop Entry]\nName=X\nExec=x\nNoDisplay=true\n").is_none());
        assert!(parse_desktop("x", "[Desktop Entry]\nName=X\nType=Link\nExec=x\n").is_none());
        assert!(parse_desktop("x", "[Desktop Entry]\nName=X\n").is_none());
    }

    #[test]
    fn windows() {
        let mk = |id: &str, c: &str, wm: &str| App { id: id.into(), name: id.into(), argv: parse_exec(c), command: c.into(), wm_class: wm.into(), ..Default::default() };
        let apps = vec![mk("chromium", "chromium", "chromium"), mk("Helix", "hx", ""), mk("org.wezfurlong.wezterm", "wezterm start --cwd .", "org.wezfurlong.wezterm"), mk("rio", "rio", "Rio")];
        assert_eq!(match_window(&apps, "chromium-browser"), Some(0));
        assert_eq!(match_window(&apps, "rio"), Some(3));
        assert_eq!(match_window(&apps, "org.wezfurlong.wezterm"), Some(2));
        assert_eq!(match_window(&apps, "hx"), Some(1));
        assert_eq!(match_window(&apps, "org.freedesktop.weston.wayland-terminal"), None);
        assert_eq!(match_window(&apps, ""), None);
    }

    #[test]
    fn names_from_app_ids() {
        assert_eq!(name_from_app_id("org.gnome.Nautilus"), "Nautilus");
        assert_eq!(name_from_app_id("org.freedesktop.weston.wayland-terminal"), "Wayland Terminal");
        assert_eq!(name_from_app_id("foot"), "Foot");
        assert_eq!(name_from_app_id("my_app.desktop"), "My App");
        assert_eq!(name_from_app_id("mpv"), "Mpv");
        assert_eq!(name_from_app_id(""), "");
        assert_eq!(name_from_app_id("..."), "");
    }

    #[test]
    fn filtering() {
        let mk = |n: &str, c: &str| App { id: n.into(), name: n.into(), command: c.into(), ..Default::default() };
        let mut apps = vec![mk("Alacritty", "alacritty"), mk("Helix", "hx"), mk("Rio", "rio"), mk("WezTerm", "wezterm start")];
        apps[1].name_zh = Some("编辑器".into());
        assert_eq!(filter(&apps, ""), vec![0, 1, 2, 3]);
        assert_eq!(filter(&apps, "RI"), vec![2, 0]); // prefix match before substring match
        assert_eq!(filter(&apps, "hx"), vec![1]);
        assert_eq!(filter(&apps, "编辑"), vec![1]);
        assert_eq!(filter(&apps, "start"), vec![3]);
        assert!(filter(&apps, "zzz").is_empty());
    }
}
