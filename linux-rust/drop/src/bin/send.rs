//! shabnam-send FILE... — sends files to a LocalSend-compatible device on the home Wi-Fi.
//! The device is chosen on the screen: the desktop shell shows the devices that were found, with
//! name, address and certificate fingerprint, and the owner clicks one. This program only hands
//! the files over and reports how it went; started from the file manager's 「发送到附近的设备…」
//! it needs no terminal.
//!
//!   shabnam-send FILE...                        choose the device on the screen
//!   shabnam-send --to NAME-or-ADDRESS FILE...   root only (a terminal, a script): send to that
//!                                               device after showing it and asking y/N
//!   shabnam-send --yes --to NAME-or-ADDRESS FILE...   the same without the question
//!
//! How: the files are opened here, with the rights of the desktop's unprivileged user "shabnam"
//! (started by root, this program first becomes that user for good — root's files are never
//! opened), and the open descriptors are passed to the 隔空传送 daemon over its submit socket
//! (../fdpass.rs). The daemon does the network part as its own user. If it is not running, the
//! shell is asked — by a bare connection to its wake socket — to start it for this request.
//!
//! This program cannot say where the files go: the submit socket has no message for it. The one
//! exception is root's `--to`, which the daemon accepts only on a connection that root made
//! (the kernel records who connected; the connection is made before root is given up).

use shabnam_drop::ipc::{self, Device, FromSubmit, SendOutcome, SubmitRefusal, ToSubmit};
use shabnam_drop::policy::{self, Lang};
use shabnam_drop::{fdpass, net};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// Everything this program says, in the system's three languages (zh, ru, en).
#[derive(Clone, Copy)]
enum Say {
    Usage,
    NotAFile,
    ToNeedsRoot,
    NoDesktop,
    NotStarted,
    ChooseOnScreen,
    Searching,
    Target,
    Fingerprint,
    Unencrypted,
    Confirm,
    Waiting,
    Sent,
    Declined,
    Busy,
    Cancelled,
    PinNeeded,
    Unreachable,
    FileChanged,
    NoAnswer,
    NoSuchDevice,
    Failed,
    RefusedBusy,
    RefusedTooMany,
    RefusedTooBig,
    RefusedNoScreen,
}

fn say(lang: Lang, what: Say) -> &'static str {
    let (zh, ru, en) = match what {
        Say::Usage => ("用法：shabnam-send [[--yes] --to 设备名或地址] 文件…", "Использование: shabnam-send [[--yes] --to ИМЯ-или-АДРЕС] ФАЙЛ…", "usage: shabnam-send [[--yes] --to NAME-or-ADDRESS] FILE..."),
        Say::NotAFile => ("不是普通文件，已跳过", "не обычный файл, пропущен", "not a regular file, skipped"),
        Say::ToNeedsRoot => ("--to 只能由 root 使用；请直接运行 shabnam-send 文件…，然后在屏幕上选择设备。", "--to доступен только root. Запустите shabnam-send ФАЙЛ… и выберите устройство на экране.", "--to is for root only; run shabnam-send FILE... and choose the device on the screen."),
        Say::NoDesktop => ("隔空传送没有运行，桌面也没有应答（root 可先运行 shabnam-drop start send）。", "Передача файлов не запущена, и рабочий стол не отвечает (root может выполнить shabnam-drop start send).", "Nearby share is not running and the desktop does not answer (root can run shabnam-drop start send)."),
        Say::NotStarted => ("隔空传送没有启动。", "Передача файлов не запустилась.", "Nearby share did not start."),
        Say::ChooseOnScreen => ("请在屏幕上选择接收设备…", "Выберите устройство на экране…", "Choose the device on the screen…"),
        Say::Searching => ("正在查找设备…", "Поиск устройства…", "Looking for the device…"),
        Say::Target => ("发送到", "Получатель", "Sending to"),
        Say::Fingerprint => ("证书指纹", "отпечаток сертификата", "certificate fingerprint"),
        Say::Unencrypted => ("未加密，无法确认对方身份", "без шифрования, устройство нельзя проверить", "not encrypted, the device cannot be verified"),
        Say::Confirm => ("确认发送？输入 y 后回车：", "Отправить? Введите y и Enter: ", "Send? Type y, then Enter: "),
        Say::Waiting => ("等待对方接受…", "Ожидание подтверждения…", "Waiting for the other side to accept…"),
        Say::Sent => ("已发送", "Отправлено", "Sent"),
        Say::Declined => ("对方拒绝了。", "Получатель отклонил передачу.", "The other side declined."),
        Say::Busy => ("对方正忙。", "Получатель занят.", "The other side is busy."),
        Say::Cancelled => ("已取消。", "Отменено.", "Cancelled."),
        Say::PinNeeded => ("对方要求 PIN：请不带 --to 运行，在屏幕上输入。", "Получатель требует PIN-код: запустите без --to и введите его на экране.", "The other side wants a PIN: run without --to and type it on the screen."),
        Say::Unreachable => ("无法连接到对方设备。", "Не удалось подключиться к устройству.", "Could not connect to the device."),
        Say::FileChanged => ("读取文件时出错。", "Ошибка чтения файла.", "A file could not be read."),
        Say::NoAnswer => ("对方没有回应。", "Устройство не ответило.", "The device did not answer."),
        Say::NoSuchDevice => ("找不到这台设备。", "Устройство не найдено.", "No such device."),
        Say::Failed => ("发送失败。", "Не удалось отправить.", "Sending failed."),
        Say::RefusedBusy => ("正在进行另一次传送，请稍后再试。", "Идёт другая передача, попробуйте позже.", "Another transfer is going on, try again later."),
        Say::RefusedTooMany => ("文件数量超过上限（500 个）。", "Больше 500 файлов.", "More than 500 files."),
        Say::RefusedTooBig => ("超过单次 8 GB 的上限。", "Больше 8 ГБ за один раз.", "More than 8 GB at once."),
        Say::RefusedNoScreen => ("桌面没有运行，无法选择设备（root 可用 --to）。", "Рабочий стол не запущен, устройство выбрать негде (root может указать --to).", "The desktop is not running, so no device can be chosen (root can use --to)."),
    };
    match lang {
        Lang::Zh => zh,
        Lang::Ru => ru,
        Lang::En => en,
    }
}

fn main() {
    let lang = policy::lang();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // Accepted and ignored: the old launcher entry ran this program in a terminal with --hold.
    args.retain(|a| a != "--hold");
    let yes = args.first().is_some_and(|a| a == "--yes");
    if yes {
        args.remove(0);
    }
    let mut to = None;
    if args.first().is_some_and(|a| a == "--to") && args.len() >= 2 {
        to = Some(args.remove(1));
        args.remove(0);
    }
    match run(lang, yes, to, args) {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("shabnam-send: {e}");
            std::process::exit(1);
        }
    }
}

/// Connects to the daemon's submit socket — only if the other end really is the daemon's user.
/// When the daemon is not running, the shell is asked to start it and the socket is tried again.
fn connect(lang: Lang) -> Result<UnixStream, String> {
    let daemon = net::user(ipc::DAEMON_USER).map(|(uid, _, _)| uid).filter(|uid| *uid != 0).ok_or_else(|| format!("user {} is missing", ipc::DAEMON_USER))?;
    let attempt = || UnixStream::connect(ipc::submit_path()).ok().filter(|stream| net::peer_uid(stream) == Some(daemon));
    if let Some(stream) = attempt() {
        return Ok(stream);
    }
    // The wake socket is root's (the shell). Nothing is said on it: the connection is the request.
    let _wake = UnixStream::connect(ipc::wake_path()).ok().filter(|stream| net::peer_uid(stream) == Some(0)).ok_or_else(|| say(lang, Say::NoDesktop).to_string())?;
    for _ in 0..60 {
        std::thread::sleep(Duration::from_millis(200));
        if let Some(stream) = attempt() {
            return Ok(stream);
        }
    }
    Err(say(lang, Say::NotStarted).into())
}

/// Started by root: becomes the desktop's unprivileged user for good, before any file is opened.
fn become_desktop_user() -> Result<(), String> {
    if unsafe { libc::geteuid() } != 0 {
        return Ok(());
    }
    let (uid, gid, home) = net::user(ipc::DESKTOP_USER).filter(|(uid, _, _)| *uid != 0).ok_or_else(|| format!("user {} is missing", ipc::DESKTOP_USER))?;
    let name = std::ffi::CString::new(ipc::DESKTOP_USER).unwrap();
    let done = unsafe { libc::initgroups(name.as_ptr(), gid as _) == 0 && libc::setgid(gid) == 0 && libc::setuid(uid) == 0 };
    // It must not be possible to get root back.
    if !done || unsafe { libc::setuid(0) } == 0 || unsafe { libc::getuid() } != uid || unsafe { libc::geteuid() } != uid {
        return Err("could not give up root".into());
    }
    std::env::set_var("HOME", &home);
    std::env::set_var("USER", ipc::DESKTOP_USER);
    Ok(())
}

/// The line that says where the files are about to go.
fn describe(lang: Lang, device: &Device) -> String {
    let proof = if device.https { format!("{} {}", say(lang, Say::Fingerprint), ipc::short_fingerprint(&device.fingerprint)) } else { say(lang, Say::Unencrypted).to_string() };
    format!("{}: {}  {}:{}  {proof}", say(lang, Say::Target), device.alias, device.host, device.port)
}

/// Only a clear yes counts.
fn is_yes(answer: &str) -> bool {
    matches!(answer.trim(), "y" | "Y" | "yes" | "YES" | "д" | "Д" | "да" | "是")
}

fn outcome_text(lang: Lang, outcome: SendOutcome) -> &'static str {
    say(lang, match outcome {
        SendOutcome::Sent => Say::Sent,
        SendOutcome::Declined => Say::Declined,
        SendOutcome::Busy => Say::Busy,
        SendOutcome::Cancelled => Say::Cancelled,
        SendOutcome::PinNeeded => Say::PinNeeded,
        SendOutcome::Unreachable => Say::Unreachable,
        SendOutcome::FileChanged => Say::FileChanged,
        SendOutcome::NoAnswer => Say::NoAnswer,
        SendOutcome::NoSuchDevice => Say::NoSuchDevice,
        SendOutcome::Failed => Say::Failed,
    })
}

fn refusal_text(lang: Lang, why: SubmitRefusal) -> &'static str {
    say(lang, match why {
        SubmitRefusal::Busy => Say::RefusedBusy,
        SubmitRefusal::TooMany => Say::RefusedTooMany,
        SubmitRefusal::TooBig => Say::RefusedTooBig,
        SubmitRefusal::NotAFile => Say::NotAFile,
        SubmitRefusal::NotAllowed => Say::ToNeedsRoot,
        SubmitRefusal::NoScreen => Say::RefusedNoScreen,
        SubmitRefusal::Bad => Say::Failed,
    })
}

/// The daemon's answers, a line at a time.
struct Answers<'a> {
    stream: &'a UnixStream,
    lines: ipc::Lines,
    ready: std::collections::VecDeque<FromSubmit>,
}

impl Answers<'_> {
    /// None: the daemon closed the connection (or said something that is not an answer).
    fn next(&mut self) -> Option<FromSubmit> {
        let mut chunk = [0u8; 2048];
        loop {
            if let Some(answer) = self.ready.pop_front() {
                return Some(answer);
            }
            let n = match (&*self.stream).read(&mut chunk) {
                Ok(n) if n > 0 => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                _ => return None,
            };
            self.ready.extend(self.lines.push(&chunk[..n]).ok()?.iter().filter_map(|line| ipc::parse_from_submit(line)));
        }
    }
}

/// Ok(true): everything was sent.
fn run(lang: Lang, yes: bool, to: Option<String>, paths: Vec<String>) -> Result<bool, String> {
    if paths.is_empty() || paths.iter().any(|p| p.starts_with("--")) {
        return Err(say(lang, Say::Usage).into());
    }
    if to.is_some() && unsafe { libc::geteuid() } != 0 {
        return Err(say(lang, Say::ToNeedsRoot).into());
    }
    // First the connection (as whoever started us: root's is what lets --to through), then
    // root is given up, and only then are the files opened — never with root's rights.
    let stream = connect(lang)?;
    become_desktop_user()?;
    let mut files = Vec::new();
    for path in &paths {
        use std::os::unix::fs::OpenOptionsExt;
        // Non-blocking, so that a pipe someone put in place of a file cannot hold us up.
        let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY).open(path).map_err(|e| format!("{path}: {e}"))?;
        if !file.metadata().map_err(|e| format!("{path}: {e}"))?.is_file() {
            eprintln!("{path}: {}", say(lang, Say::NotAFile));
            continue;
        }
        let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
        files.push((name, file));
    }
    if files.is_empty() {
        return Err(say(lang, Say::Usage).into());
    }
    let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(20)));
    let mut answers = Answers { stream: &stream, lines: ipc::Lines::default(), ready: Default::default() };
    let gone = || say(lang, Say::Failed).to_string();
    for (name, file) in &files {
        fdpass::send_line(&stream, &ToSubmit::File { name: name.clone() }.encode(), Some(file.as_raw_fd())).map_err(|e| e.to_string())?;
        match answers.next().ok_or_else(gone)? {
            FromSubmit::Ok => {}
            FromSubmit::Refused(why) => return Err(format!("{name}: {}", refusal_text(lang, why))),
            _ => return Err(gone()),
        }
    }
    // The daemon holds the files now.
    drop(files);
    let auto = to.is_some();
    (&stream).write_all(ToSubmit::Go { to }.encode().as_bytes()).map_err(|e| e.to_string())?;
    let _ = stream.set_read_timeout(None);
    if auto {
        println!("{}", say(lang, Say::Searching));
    }
    let mut shown = u64::MAX;
    loop {
        match answers.next().ok_or_else(gone)? {
            FromSubmit::Queued => println!("{}", say(lang, Say::ChooseOnScreen)),
            FromSubmit::Refused(why) => return Err(refusal_text(lang, why).into()),
            FromSubmit::Target(device) => {
                println!("{}", describe(lang, &device));
                if !yes {
                    print!("{}", say(lang, Say::Confirm));
                    let _ = std::io::stdout().flush();
                    let mut answer = String::new();
                    if std::io::stdin().read_line(&mut answer).is_err() || !is_yes(&answer) {
                        // Closing the connection withdraws the request.
                        println!("{}", say(lang, Say::Cancelled));
                        return Ok(false);
                    }
                }
                (&stream).write_all(ToSubmit::Confirm.encode().as_bytes()).map_err(|e| e.to_string())?;
                println!("{}", say(lang, Say::Waiting));
            }
            FromSubmit::Progress { done, total } => {
                // One step per 5 %: enough to see it move, readable in a log.
                let percent = (done.saturating_mul(100)).checked_div(total).unwrap_or(100).min(100);
                if percent / 5 != shown {
                    shown = percent / 5;
                    eprint!("\r{percent:3}%");
                }
            }
            FromSubmit::Result { outcome, sent, to } => {
                if shown != u64::MAX {
                    eprintln!();
                }
                return match outcome {
                    SendOutcome::Sent => {
                        println!("{} {sent} → {to}", say(lang, Say::Sent));
                        Ok(true)
                    }
                    SendOutcome::Cancelled => {
                        println!("{}", say(lang, Say::Cancelled));
                        Ok(false)
                    }
                    outcome => Err(outcome_text(lang, outcome).into()),
                };
            }
            FromSubmit::Ok => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shabnam_drop::ipc::DeviceKind;

    #[test]
    fn the_target_is_shown_in_full() {
        let device = Device { alias: "Phone".into(), fingerprint: "AB12CD34EF56AB78FFFF0000".into(), host: "192.0.2.7".into(), port: 53317, https: true, kind: DeviceKind::Mobile };
        let line = describe(Lang::En, &device);
        assert!(line.contains("Phone") && line.contains("192.0.2.7:53317") && line.contains("AB12 CD34 EF56 AB78"), "{line}");
        let plain = describe(Lang::En, &Device { https: false, ..device });
        assert!(plain.contains("not encrypted") && !plain.contains("AB12"));
    }

    #[test]
    fn only_yes_confirms() {
        for answer in ["y\n", "Y", " yes ", "да", "是"] {
            assert!(is_yes(answer), "{answer}");
        }
        for answer in ["", "\n", "n", "no", "1", "yy", "ok"] {
            assert!(!is_yes(answer), "{answer}");
        }
    }
}
