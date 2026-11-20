//! File names for what arrives: a sender chooses the name, so it is rewritten into one plain
//! file name before anything is created, and an existing file is never replaced.

use std::path::{Path, PathBuf};

/// Longest name kept, in bytes (ext4's limit is 255; this leaves room for " (99)" and ".part").
const MAX_BYTES: usize = 200;

/// Turns a sender's file name (which may be a path, as the protocol allows for folders) into a
/// single, harmless file name:
/// only the last path component survives (so "../../etc/passwd" becomes "passwd"); control
/// characters and the characters other systems cannot store are replaced; a leading dot is
/// replaced, so nothing hidden (".bashrc", ".config") can be planted; the length is capped with
/// the extension kept; an empty result becomes "file".
pub fn sanitize(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).find(|part| !part.trim().is_empty()).unwrap_or("");
    let mut clean: String = last
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') { '_' } else { c })
        .collect();
    clean = clean.trim().trim_end_matches(['.', ' ']).to_string();
    if clean.starts_with('.') {
        clean.replace_range(..1, "_");
    }
    if clean.is_empty() || clean.chars().all(|c| c == '_' || c == '.') {
        return "file".into();
    }
    if clean.len() > MAX_BYTES {
        let (stem, extension) = split(&clean);
        let extension: String = if extension.len() <= 20 { extension.to_string() } else { String::new() };
        let mut keep = MAX_BYTES - extension.len();
        while !stem.is_char_boundary(keep) {
            keep -= 1;
        }
        clean = format!("{}{extension}", stem[..keep].trim_end());
    }
    clean
}

/// "photo.tar.gz" -> ("photo.tar", ".gz"); a name without a dot (or only a leading one) has no extension.
fn split(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    }
}

/// The n-th name to try for `name`: itself, then "name (2).ext", "name (3).ext", …
pub fn candidate(name: &str, attempt: u32) -> String {
    if attempt <= 1 {
        return name.to_string();
    }
    let (stem, extension) = split(name);
    format!("{stem} ({attempt}){extension}")
}

/// Gives the finished file `part` its final name in `dir` without ever replacing a file that is
/// already there: a hard link fails when the name exists, so the next candidate is tried.
/// Returns the name it got. `part` is removed either way.
pub fn publish(dir: &Path, part: &Path, wanted: &str) -> std::io::Result<String> {
    let mut outcome = Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no free name"));
    for attempt in 1..=999 {
        let name = candidate(wanted, attempt);
        match std::fs::hard_link(part, dir.join(&name)) {
            Ok(()) => {
                outcome = Ok(name);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                outcome = Err(e);
                break;
            }
        }
    }
    let _ = std::fs::remove_file(part);
    outcome
}

/// Where a file is written while it arrives: a hidden name in the same directory that no sender
/// can choose, created only if it does not exist.
pub fn part_path(dir: &Path, session: &str, number: u32) -> PathBuf {
    let short: String = session.chars().filter(|c| c.is_ascii_alphanumeric()).take(12).collect();
    dir.join(format!(".shabnam-drop-{short}-{number}.part"))
}

/// Removes what an interrupted run left behind.
pub fn remove_stale_parts(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(".shabnam-drop-") && name.ends_with(".part") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_path_components() {
        assert_eq!(sanitize("../../etc/passwd"), "passwd");
        assert_eq!(sanitize("/etc/shadow"), "shadow");
        assert_eq!(sanitize("..\\..\\Windows\\win.ini"), "win.ini");
        assert_eq!(sanitize("folder/sub/照片 1.jpg"), "照片 1.jpg");
        assert_eq!(sanitize("a/b/"), "b");
        assert_eq!(sanitize(".."), "file");
        assert_eq!(sanitize("."), "file");
        assert_eq!(sanitize(""), "file");
        assert_eq!(sanitize("///"), "file");
        assert_eq!(sanitize("   "), "file");
        for name in ["../x", "a/../../b", "/abs", "c:\\x\\y", "..", "x/..", "\0", "a\nb"] {
            let clean = sanitize(name);
            assert!(!clean.contains('/') && !clean.contains('\\') && clean != ".." && clean != "." && !clean.is_empty(), "{name:?} -> {clean:?}");
            assert!(!clean.chars().any(char::is_control));
        }
    }

    #[test]
    fn nothing_hidden_or_odd() {
        assert_eq!(sanitize(".bashrc"), "_bashrc");
        assert_eq!(sanitize("dir/.config"), "_config");
        assert_eq!(sanitize("..hidden"), "_.hidden");
        assert_eq!(sanitize("a\u{1b}[31mb.txt"), "a_[31mb.txt");
        assert_eq!(sanitize("what?.txt"), "what_.txt");
        assert_eq!(sanitize("trailing. . "), "trailing");
        // Right-to-left override, used to make "exe" look like "txt".
        assert_eq!(sanitize("doc\u{202e}txt.exe"), "doc_txt.exe");
    }

    #[test]
    fn long_names_keep_their_extension() {
        let long = format!("{}.jpeg", "图".repeat(200));
        let clean = sanitize(&long);
        assert!(clean.len() <= MAX_BYTES && clean.ends_with(".jpeg") && clean.starts_with("图图"));
        let clean = sanitize(&"a".repeat(1000));
        assert_eq!(clean.len(), MAX_BYTES);
    }

    #[test]
    fn candidates() {
        assert_eq!(candidate("a.txt", 1), "a.txt");
        assert_eq!(candidate("a.txt", 2), "a (2).txt");
        assert_eq!(candidate("archive.tar.gz", 3), "archive.tar (3).gz");
        assert_eq!(candidate("README", 2), "README (2)");
        assert_eq!(candidate("_bashrc", 2), "_bashrc (2)");
    }

    #[test]
    fn never_overwrites() {
        let dir = std::env::temp_dir().join(format!("shabnam-drop-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "old").unwrap();
        for (number, expected) in [(1, "a (2).txt"), (2, "a (3).txt")] {
            let part = part_path(&dir, "1234-abcd", number);
            std::fs::write(&part, format!("new{number}")).unwrap();
            assert_eq!(publish(&dir, &part, "a.txt").unwrap(), expected);
            assert!(!part.exists());
        }
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(dir.join("a (3).txt")).unwrap(), "new2");
        std::fs::write(part_path(&dir, "x", 9), "left over").unwrap();
        remove_stale_parts(&dir);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
