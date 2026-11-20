//! The system's three interface languages and every text the shell shows, in one table.
//!
//! One process-wide language (`current`), read at start from /etc/shabnam/locale (the word the
//! language switch in quick settings writes; the session script turns the same word into LANG)
//! and changed while running by `set_current`.
//!
//! A text cannot exist in fewer than three languages: `messages!` and `counted!` only accept
//! entries that give all of them, so a missing translation does not compile. The tests check
//! what the compiler cannot: nothing empty, the same places to fill in every language.
//! Depends on std only, so `rustc --test src/lang.rs` checks it on any machine.

use std::sync::atomic::{AtomicU8, Ordering};

/// Where the system language is kept: one word, "zh", "ru" or "en".
pub const LOCALE_FILE: &str = "/etc/shabnam/locale";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Zh,
    Ru,
    En,
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// The language the shell speaks right now.
pub fn current() -> Lang {
    Lang::ALL[CURRENT.load(Ordering::Relaxed) as usize % 3]
}

pub fn set_current(lang: Lang) {
    CURRENT.store(Lang::ALL.iter().position(|l| *l == lang).unwrap_or(0) as u8, Ordering::Relaxed);
}

impl Lang {
    /// In the order the language switch offers them.
    pub const ALL: [Lang; 3] = [Lang::Zh, Lang::Ru, Lang::En];

    /// The word kept in the locale file.
    pub fn code(self) -> &'static str {
        match self {
            Lang::Zh => "zh",
            Lang::Ru => "ru",
            Lang::En => "en",
        }
    }

    /// What the session script exports as LANG for this word (device/session.sh).
    pub fn locale(self) -> &'static str {
        match self {
            Lang::Zh => "zh_CN.UTF-8",
            Lang::Ru => "ru_RU.UTF-8",
            Lang::En => "en_US.UTF-8",
        }
    }

    /// The language's name, always written in the language itself.
    pub fn own_name(self) -> &'static str {
        match self {
            Lang::Zh => "中文",
            Lang::Ru => "Русский",
            Lang::En => "English",
        }
    }

    /// The contents of the locale file: exactly one of the three words.
    pub fn from_code(word: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.code() == word.trim())
    }

    /// From a LANG-style value: "ru_RU.UTF-8", "zh_CN", "en" …
    pub fn from_locale(value: &str) -> Option<Lang> {
        let language = value.split(['_', '.', '@', '-']).next().unwrap_or("").to_ascii_lowercase();
        Lang::ALL.into_iter().find(|l| l.code() == language)
    }

    /// The bracketed parts of desktop-file keys to try, best first: Name[zh_CN], then Name[zh];
    /// the plain key comes after these.
    pub fn desktop_keys(self) -> &'static [&'static str] {
        match self {
            Lang::Zh => &["zh_CN", "zh"],
            Lang::Ru => &["ru_RU", "ru"],
            Lang::En => &["en_US", "en"],
        }
    }
}

/// The language to start in: the locale file's word, else what LANG says, else Chinese
/// (the same order and the same default as the session script).
pub fn startup(file: Option<&str>, lang_var: Option<&str>) -> Lang {
    file.and_then(Lang::from_code).or_else(|| lang_var.and_then(Lang::from_locale)).unwrap_or(Lang::Zh)
}

/// Writes the word to the locale file: a new file (mode 644) renamed over the old one, so the
/// session script never reads half a word.
pub fn save(lang: Lang) -> std::io::Result<()> {
    save_to(LOCALE_FILE, lang)
}

fn save_to(path: &str, lang: Lang) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let new = format!("{path}.new");
    let _ = std::fs::remove_file(&new);
    let written = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o644).open(&new)?;
        file.write_all(format!("{}\n", lang.code()).as_bytes())?;
        // Whatever the process's umask is: readable by all, writable by the owner only.
        file.set_permissions(std::fs::Permissions::from_mode(0o644))?;
        file.sync_all()?;
        std::fs::rename(&new, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&new);
    }
    written
}

/// Puts `values` into the `{}` places of a text, in order.
pub fn fill(template: &str, values: &[&str]) -> String {
    let mut out = String::new();
    let mut parts = template.split("{}");
    out.push_str(parts.next().unwrap_or(""));
    for (i, part) in parts.enumerate() {
        out.push_str(values.get(i).copied().unwrap_or(""));
        out.push_str(part);
    }
    out
}

macro_rules! messages {
    ($($id:ident: $zh:literal, $ru:literal, $en:literal;)*) => {
        /// A text without a number in it. `{}` stands for a name, filled in by `fill`.
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum Msg { $($id),* }

        impl Msg {
            /// Every text, for the tests.
            #[allow(dead_code)]
            pub const ALL: &'static [Msg] = &[$(Msg::$id),*];

            pub fn text(self, lang: Lang) -> &'static str {
                let (zh, ru, en) = match self { $(Msg::$id => ($zh, $ru, $en)),* };
                match lang {
                    Lang::Zh => zh,
                    Lang::Ru => ru,
                    Lang::En => en,
                }
            }
        }
    };
}

macro_rules! counted {
    ($($id:ident: $zh:literal, [$one:literal, $few:literal, $many:literal], [$en_one:literal, $en_other:literal];)*) => {
        /// A text about a number of things. `{n}` is the number; `{}` places are for names.
        /// Russian has three forms (1 файл, 2 файла, 5 файлов), English two, Chinese one.
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum Count { $($id),* }

        impl Count {
            #[allow(dead_code)]
            pub const ALL: &'static [Count] = &[$(Count::$id),*];

            pub fn template(self, lang: Lang, n: u64) -> &'static str {
                let (zh, ru, en) = match self { $(Count::$id => ($zh, [$one, $few, $many], [$en_one, $en_other])),* };
                match lang {
                    Lang::Zh => zh,
                    Lang::Ru => ru[russian_form(n)],
                    Lang::En => en[(n != 1) as usize],
                }
            }

            /// The finished text: the number and the names put in.
            pub fn text(self, lang: Lang, n: u64, values: &[&str]) -> String {
                fill(&self.template(lang, n).replace("{n}", &n.to_string()), values)
            }
        }
    };
}

/// Which Russian form goes with `n`: 0 for 1, 21, 101 … (файл); 1 for 2–4, 22–24 … (файла);
/// 2 for everything else, 11–14 included (файлов).
pub fn russian_form(n: u64) -> usize {
    let (tens, ones) = (n % 100, n % 10);
    if ones == 1 && tens != 11 {
        0
    } else if (2..=4).contains(&ones) && !(12..=14).contains(&tens) {
        1
    } else {
        2
    }
}

messages! {
    // ----- shelf and launcher -----
    Window: "窗口", "Окно", "Window";
    Recent: "最近使用", "Недавние", "Recent";
    AllApps: "全部应用", "Все приложения", "All apps";
    SearchHint: "搜索应用、网址…", "Поиск приложений и сайтов…", "Search apps, web addresses…";
    OpenUrl: "打开网址 {}", "Открыть {}", "Open {}";
    NoMatch: "没有匹配的应用", "Ничего не найдено", "No matching apps";
    // ----- context menus -----
    NewWindow: "新窗口", "Новое окно", "New window";
    Open: "打开", "Открыть", "Open";
    Unpin: "取消固定", "Открепить", "Unpin";
    Pin: "固定到底栏", "Закрепить на панели", "Pin to shelf";
    ShowInLauncher: "在启动器中显示", "Показывать в меню приложений", "Show in launcher";
    HideFromLauncher: "从启动器隐藏", "Скрыть из меню приложений", "Hide from launcher";
    CloseWindow: "关闭窗口", "Закрыть окно", "Close window";
    // ----- quick settings -----
    Battery: "电量 {}%", "Батарея {}%", "Battery {}%";
    BatteryFull: "电量 {}% · 已充满", "Батарея {}% · заряжена", "Battery {}% · full";
    BatteryCharging: "电量 {}% · 充电中", "Батарея {}% · заряжается", "Battery {}% · charging";
    Connected: "已连接", "Подключено", "Connected";
    WiredOnly: "未连接 · 有线 {}", "Проводная сеть · {}", "Off · wired {}";
    NotConnected: "未连接", "Не подключено", "Not connected";
    ConfirmTitle: "再点一次确认", "Нажмите ещё раз", "Click again to confirm";
    ConfirmSub: "将结束当前桌面会话", "Сеанс будет завершён", "Ends this desktop session";
    MacTitle: "Mac 副屏", "Второй экран для Mac", "Second screen for Mac";
    MacSub: "切换为 Mac 的第二屏幕", "Как монитор для Mac", "Use as a Mac display";
    Brightness: "亮度", "Яркость", "Brightness";
    Volume: "音量", "Громкость", "Volume";
    PowerOff: "关机", "Выключить", "Power off";
    PowerOffConfirm: "再点一次确认关机", "Ещё раз — выключить", "Click again: power off";
    Restart: "重启", "Перезагрузить", "Restart";
    RestartConfirm: "再点一次确认重启", "Ещё раз — перезагрузить", "Click again: restart";
    NoNetwork: "网络未连接", "Нет сети", "No network";
    // ----- the language switch -----
    Language: "语言", "Язык", "Language";
    LanguageNotice: "其他应用将在重启桌面后切换语言", "Другие приложения сменят язык после перезапуска рабочего стола", "Other apps switch language after the desktop restarts";
    RestartDesktop: "立即重启桌面", "Перезапустить сейчас", "Restart desktop now";
    RestartDesktopConfirm: "再点一次：所有窗口将关闭", "Ещё раз — все окна закроются", "Click again: all windows close";
    LanguageNotSaved: "无法保存语言设置，重启后将恢复原语言", "Не удалось сохранить язык: после перезапуска вернётся прежний", "Could not save the language: it reverts after a restart";
    // ----- Wi-Fi -----
    WifiGettingAddress: "已连接，正在获取地址…", "Подключено, получение адреса…", "Connected, getting an address…";
    WifiConnectedAt: "已连接 · {}", "Подключено · {}", "Connected · {}";
    WifiNoAddress: "已连接，尚未获得地址", "Подключено, адрес ещё не получен", "Connected, no address yet";
    WifiFailed: "密码错误或无法连接", "Неверный пароль или не удалось подключиться", "Wrong password or cannot connect";
    WifiConnecting: "正在连接 {}…", "Подключение к {}…", "Connecting to {}…";
    WifiAlready: "已连接到此网络", "Уже подключено к этой сети", "Already connected to this network";
    WifiUnsupported: "暂不支持这种加密方式", "Этот тип защиты не поддерживается", "This kind of security is not supported";
    Scanning: "正在扫描…", "Поиск…", "Scanning…";
    LookingForNetworks: "正在查找网络…", "Поиск сетей…", "Looking for networks…";
    NoNetworksFound: "没有找到网络", "Сети не найдены", "No networks found";
    Saved: "已保存", "Сохранена", "Saved";
    OpenNetwork: "开放网络", "Открытая сеть", "Open";
    UnsupportedSecurity: "不支持的加密方式", "Защита не поддерживается", "Unsupported security";
    Secured: "加密", "Защищена", "Secured";
    ConnectTo: "连接到 {}", "Подключение к {}", "Connect to {}";
    Password: "密码", "Пароль", "Password";
    Hide: "隐藏", "Скрыть", "Hide";
    Show: "显示", "Показать", "Show";
    BadPassword: "密码须为 8–63 个字符，且不能含双引号", "От 8 до 63 символов, без двойных кавычек", "8 to 63 characters, no double quote";
    PasswordHint: "输入密码后按回车或点「连接」", "Введите пароль и нажмите Enter или «Подключить»", "Type the password, then Enter or Connect";
    Cancel: "取消", "Отмена", "Cancel";
    Connect: "连接", "Подключить", "Connect";
    ExampleNetwork: "示例网络", "Пример сети", "Example network";
    // ----- 隔空传送 -----
    DropTitle: "隔空传送", "Передача файлов", "Nearby share";
    DropOff: "已关闭 · 点按开启接收", "Выключено · нажмите, чтобы принимать", "Off · click to receive";
    DropStarting: "正在开启…", "Включение…", "Starting…";
    DropNoWifi: "等待 Wi-Fi…", "Ожидание Wi-Fi…", "Waiting for Wi-Fi…";
    DropOn: "接收中 · 本机名「{}」", "Приём · имя «{}»", "Receiving · as “{}”";
    DropQuickTitle: "快速接收", "Быстрый приём", "Quick receive";
    DropQuickOff: "已关闭 · 每次询问", "Выкл. · спрашивать", "Off · always ask";
    DropQuickOn: "已开启 · 不再询问", "Вкл. · без вопросов", "On · no asking";
    DropAndMore: "{} 等", "{} и другие", "{} and more";
    DropAccept: "接受", "Принять", "Accept";
    DropDecline: "拒绝", "Отклонить", "Decline";
    DropReceiving: "正在接收「{}」的文件", "Приём файлов от «{}»", "Receiving from “{}”";
    DropReceivingPlain: "正在接收文件", "Приём файлов", "Receiving files";
    DropNothingSaved: "传送未完成", "Передача не завершена", "The transfer did not complete";
    DropShowInFolder: "在文件夹中显示", "Показать в папке", "Show in folder";
    DropClose: "知道了", "Понятно", "OK";
    DropRefused: "已拒绝「{}」的传送", "Передача от «{}» отклонена", "Declined the transfer from “{}”";
    DropWhyTooBig: "超过单次 8 GB 的上限", "Больше 8 ГБ за один раз", "More than 8 GB at once";
    DropWhyNoSpace: "磁盘空间不足", "Недостаточно места на диске", "Not enough disk space";
    DropWhyTooMany: "文件数量超过上限（500 个）", "Больше 500 файлов", "More than 500 files";
    DropMessageFrom: "「{}」发来消息", "Сообщение от «{}»", "Message from “{}”";
    DropStartFailed: "无法开启隔空传送", "Не удалось включить приём", "Could not start receiving";
    DropStartFailedWhy: "接收程序没有启动", "Программа приёма не запустилась", "The receiver did not start";
    DropFrom: "来自「{}」", "от «{}»", "from “{}”";
    // ----- 隔空传送: sending (the picker, progress, result) -----
    SendChoose: "选择接收设备", "Выберите устройство", "Choose a device";
    SendSearching: "正在查找附近的设备…", "Поиск устройств поблизости…", "Looking for nearby devices…";
    SendEmpty: "没有找到设备。请在对方设备上打开 LocalSend，并连接到同一个 Wi-Fi。", "Устройства не найдены. Откройте LocalSend на другом устройстве и подключите его к той же сети Wi-Fi.", "No devices found. Open LocalSend on the other device and connect it to the same Wi-Fi.";
    SendNoWifi: "Wi-Fi 未连接，无法发送。请先连接 Wi-Fi。", "Wi-Fi не подключён, отправить нельзя. Сначала подключитесь к сети.", "Wi-Fi is not connected, so nothing can be sent. Connect to Wi-Fi first.";
    SendRefresh: "重新查找", "Искать снова", "Search again";
    SendKnown: "已发送过", "Уже отправляли", "Sent before";
    SendUnencrypted: "未加密，无法确认对方身份", "Без шифрования: устройство нельзя проверить", "Not encrypted: the device cannot be verified";
    SendPlain: "未加密", "без шифрования", "not encrypted";
    SendToTitle: "发送到「{}」？", "Отправить на «{}»?", "Send to “{}”?";
    SendFingerprint: "证书指纹 {}", "Отпечаток сертификата: {}", "Certificate fingerprint {}";
    SendFirstTime: "还没有向这台设备发送过。请核对名称、地址和指纹，确认是你要发送的设备。", "На это устройство вы ещё не отправляли. Сверьте имя, адрес и отпечаток: это должно быть нужное устройство.", "You have not sent to this device before. Check its name, address and fingerprint: it should be the device you mean.";
    SendBack: "返回", "Назад", "Back";
    SendGo: "发送", "Отправить", "Send";
    SendPinTitle: "「{}」需要 PIN 码", "«{}» требует PIN-код", "“{}” asks for a PIN";
    SendPinField: "PIN 码", "PIN-код", "PIN";
    SendPinHint: "输入对方设备上设置的 PIN 码，然后按回车", "Введите PIN-код, заданный на том устройстве, и нажмите Enter", "Type the PIN set on that device, then Enter";
    SendPinWrong: "PIN 码不正确，请重试", "Неверный PIN-код, попробуйте ещё раз", "Wrong PIN, try again";
    SendWaiting: "等待「{}」接受…", "Ожидание ответа от «{}»…", "Waiting for “{}” to accept…";
    SendWaitingSub: "请在对方设备上点接受", "Подтвердите приём на том устройстве", "Accept it on the other device";
    SendSending: "正在发送到「{}」", "Отправка на «{}»", "Sending to “{}”";
    SendSentTo: "已送达「{}」", "Получатель: «{}»", "Delivered to “{}”";
    SendDeclined: "「{}」拒绝了传送", "«{}»: передача отклонена", "“{}” declined the transfer";
    SendBusy: "「{}」正忙，请稍后再试", "«{}» сейчас занято, попробуйте позже", "“{}” is busy, try again later";
    SendFailed: "未能发送到「{}」", "Не удалось отправить на «{}»", "Could not send to “{}”";
    SendNothingSent: "没有发送任何文件", "Ничего не отправлено", "Nothing was sent";
    SendWhyUnreachable: "无法连接到对方设备", "Не удалось подключиться к устройству", "Could not connect to the device";
    SendWhyFile: "读取文件时出错", "Ошибка чтения файла", "A file could not be read";
    SendWhyNoAnswer: "对方没有回应", "Устройство не ответило", "The device did not answer";
    SendWhyInterrupted: "传送中断", "Передача прервана", "The transfer was interrupted";
    SendWhyPin: "PIN 码未通过验证", "PIN-код не принят", "The PIN was not accepted";
    KindPhone: "手机", "Телефон", "Phone";
    KindComputer: "电脑", "Компьютер", "Computer";
    KindBrowser: "浏览器", "Браузер", "Browser";
    KindDevice: "设备", "Устройство", "Device";
}

counted! {
    CloseWindows: "关闭 {n} 个窗口",
        ["Закрыть {n} окно", "Закрыть {n} окна", "Закрыть {n} окон"],
        ["Close {n} window", "Close {n} windows"];
    DropOffer: "「{}」想发送 {n} 个文件",
        ["«{}» хочет отправить {n} файл", "«{}» хочет отправить {n} файла", "«{}» хочет отправить {n} файлов"],
        ["“{}” wants to send {n} file", "“{}” wants to send {n} files"];
    DropReceived: "已收到 {n} 个文件",
        ["Получен {n} файл", "Получено {n} файла", "Получено {n} файлов"],
        ["{n} file received", "{n} files received"];
    DropFailed: "{n} 个未完成",
        ["{n} файл не получен", "{n} файла не получено", "{n} файлов не получено"],
        ["{n} not completed", "{n} not completed"];
    SendFiles: "发送 {n} 个文件",
        ["Отправить {n} файл", "Отправить {n} файла", "Отправить {n} файлов"],
        ["Send {n} file", "Send {n} files"];
    SendSent: "已发送 {n} 个文件",
        ["Отправлен {n} файл", "Отправлено {n} файла", "Отправлено {n} файлов"],
        ["{n} file sent", "{n} files sent"];
    SendSentPart: "中断前已发送 {n} 个文件",
        ["До сбоя отправлен {n} файл", "До сбоя отправлено {n} файла", "До сбоя отправлено {n} файлов"],
        ["{n} file was sent before that", "{n} files were sent before that"];
    SendFileOf: "第 {n} 个，共 {} 个",
        ["Файл {n} из {}", "Файл {n} из {}", "Файл {n} из {}"],
        ["File {n} of {}", "File {n} of {}"];
}

/// The date line of the quick panel: "11月20日 周五", "Пт, 20 ноября", "Fri, 20 Nov".
/// `weekday` counts from Sunday = 0, `month` from January = 0.
pub fn date(lang: Lang, weekday: usize, month: usize, day: u32) -> String {
    const ZH_DAYS: [&str; 7] = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];
    const RU_DAYS: [&str; 7] = ["Вс", "Пн", "Вт", "Ср", "Чт", "Пт", "Сб"];
    // After a day number Russian puts the month in the genitive.
    const RU_MONTHS: [&str; 12] =
        ["января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря"];
    const EN_DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const EN_MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let (weekday, month) = (weekday.min(6), month.min(11));
    match lang {
        Lang::Zh => format!("{}月{}日 {}", month + 1, day, ZH_DAYS[weekday]),
        Lang::Ru => format!("{}, {} {}", RU_DAYS[weekday], day, RU_MONTHS[month]),
        Lang::En => format!("{}, {} {}", EN_DAYS[weekday], day, EN_MONTHS[month]),
    }
}

/// "12.4 MB"; in Russian "12,4 МБ" (decimal comma, Cyrillic units).
pub fn human_size(lang: Lang, bytes: u64) -> String {
    let units: [&str; 5] = match lang {
        Lang::Ru => ["Б", "КБ", "МБ", "ГБ", "ТБ"],
        _ => ["B", "KB", "MB", "GB", "TB"],
    };
    if bytes < 1000 {
        return format!("{bytes} {}", units[0]);
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 1;
    while value >= 999.95 && unit < units.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    let number = format!("{value:.1}");
    let number = if lang == Lang::Ru { number.replace('.', ",") } else { number };
    format!("{number} {}", units[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places(text: &str) -> usize {
        text.matches("{}").count()
    }

    #[test]
    fn every_text_exists_in_three_languages() {
        assert!(Msg::ALL.len() >= 80);
        for msg in Msg::ALL {
            let texts = Lang::ALL.map(|lang| msg.text(lang));
            for text in texts {
                assert!(!text.trim().is_empty(), "{msg:?} is empty in some language");
                assert!(!text.contains("{n}"), "{msg:?}: a number belongs in a `counted!` text");
            }
            assert!(places(texts[0]) == places(texts[2]) && places(texts[1]) == places(texts[2]), "{msg:?}: places differ");
        }
        for count in Count::ALL {
            // Every form of every language: one number, and the same places for names.
            let en = count.template(Lang::En, 2);
            for lang in Lang::ALL {
                for n in [1, 2, 5, 11, 21, 22] {
                    let text = count.template(lang, n);
                    assert_eq!(text.matches("{n}").count(), 1, "{count:?}");
                    assert_eq!(places(text), places(en), "{count:?}");
                }
            }
        }
    }

    #[test]
    fn chinese_and_russian_texts_are_translated() {
        // A text left in English by mistake has no Chinese / Cyrillic letter. The few that are
        // rightly without one are named here.
        let cjk = |t: &str| t.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
        let cyrillic = |t: &str| t.chars().any(|c| ('\u{0400}'..='\u{04ff}').contains(&c));
        for msg in Msg::ALL {
            assert!(cjk(msg.text(Lang::Zh)) || matches!(msg, Msg::DropNoWifi), "{msg:?} has no Chinese text");
            assert!(cyrillic(msg.text(Lang::Ru)), "{msg:?} has no Russian text");
            assert!(!cjk(msg.text(Lang::Ru)) && !cjk(msg.text(Lang::En)), "{msg:?}: Chinese outside the Chinese column");
            assert!(!cyrillic(msg.text(Lang::Zh)) && !cyrillic(msg.text(Lang::En)), "{msg:?}: Russian outside the Russian column");
        }
        for count in Count::ALL {
            for n in [1, 3, 7] {
                assert!(cjk(count.template(Lang::Zh, n)) && cyrillic(count.template(Lang::Ru, n)), "{count:?}");
                assert!(!cjk(count.template(Lang::En, n)) && !cyrillic(count.template(Lang::En, n)), "{count:?}");
            }
        }
    }

    #[test]
    fn russian_plurals() {
        let form = |n: u64| russian_form(n);
        for n in [1, 21, 31, 101, 121, 1001] {
            assert_eq!(form(n), 0, "{n}");
        }
        for n in [2, 3, 4, 22, 23, 24, 102, 104, 1002] {
            assert_eq!(form(n), 1, "{n}");
        }
        for n in [0, 5, 6, 9, 10, 11, 12, 13, 14, 15, 19, 20, 25, 100, 111, 112, 113, 114, 211, 1011] {
            assert_eq!(form(n), 2, "{n}");
        }
        assert_eq!(Count::DropReceived.text(Lang::Ru, 1, &[]), "Получен 1 файл");
        assert_eq!(Count::DropReceived.text(Lang::Ru, 2, &[]), "Получено 2 файла");
        assert_eq!(Count::DropReceived.text(Lang::Ru, 5, &[]), "Получено 5 файлов");
        assert_eq!(Count::DropReceived.text(Lang::Ru, 11, &[]), "Получено 11 файлов");
        assert_eq!(Count::DropReceived.text(Lang::Ru, 21, &[]), "Получен 21 файл");
        assert_eq!(Count::DropOffer.text(Lang::Ru, 3, &["Телефон"]), "«Телефон» хочет отправить 3 файла");
        assert_eq!(Count::CloseWindows.text(Lang::Ru, 2, &[]), "Закрыть 2 окна");
        assert_eq!(Count::CloseWindows.text(Lang::Ru, 12, &[]), "Закрыть 12 окон");
        assert_eq!(Count::DropFailed.text(Lang::Ru, 1, &[]), "1 файл не получен");
        assert_eq!(Count::SendFiles.text(Lang::Ru, 1, &[]), "Отправить 1 файл");
        assert_eq!(Count::SendFiles.text(Lang::Ru, 3, &[]), "Отправить 3 файла");
        assert_eq!(Count::SendFiles.text(Lang::Ru, 12, &[]), "Отправить 12 файлов");
        assert_eq!(Count::SendSent.text(Lang::Ru, 1, &[]), "Отправлен 1 файл");
        assert_eq!(Count::SendSent.text(Lang::Ru, 22, &[]), "Отправлено 22 файла");
        assert_eq!(Count::SendSent.text(Lang::Ru, 5, &[]), "Отправлено 5 файлов");
        assert_eq!(Count::SendFileOf.text(Lang::Ru, 2, &["3"]), "Файл 2 из 3");
        assert_eq!(Count::SendFileOf.text(Lang::Zh, 2, &["3"]), "第 2 个，共 3 个");
        assert_eq!(Count::SendFileOf.text(Lang::En, 2, &["3"]), "File 2 of 3");
    }

    #[test]
    fn counts_in_chinese_and_english() {
        assert_eq!(Count::DropOffer.text(Lang::Zh, 3, &["A"]), "「A」想发送 3 个文件");
        assert_eq!(Count::DropOffer.text(Lang::Zh, 1, &["A"]), "「A」想发送 1 个文件");
        assert_eq!(Count::DropOffer.text(Lang::En, 1, &["A"]), "“A” wants to send 1 file");
        assert_eq!(Count::DropOffer.text(Lang::En, 3, &["A"]), "“A” wants to send 3 files");
        assert_eq!(Count::DropReceived.text(Lang::En, 1, &[]), "1 file received");
        assert_eq!(Count::CloseWindows.text(Lang::En, 2, &[]), "Close 2 windows");
        assert_eq!(Count::CloseWindows.text(Lang::Zh, 2, &[]), "关闭 2 个窗口");
        // A name that itself contains braces is put in as it is.
        assert_eq!(Count::DropOffer.text(Lang::En, 2, &["{n} {}"]), "“{n} {}” wants to send 2 files");
    }

    #[test]
    fn filling_in() {
        assert_eq!(fill("「{}」想发送 {} 个文件", &["A", "3"]), "「A」想发送 3 个文件");
        assert_eq!(fill("no places", &["x"]), "no places");
        assert_eq!(fill("{} and {}", &["one"]), "one and ");
        assert_eq!(fill(Msg::Battery.text(Lang::Ru), &["85"]), "Батарея 85%");
    }

    #[test]
    fn languages_and_locales() {
        assert_eq!(Lang::from_code("ru\n"), Some(Lang::Ru));
        assert_eq!(Lang::from_code("zh"), Some(Lang::Zh));
        assert_eq!(Lang::from_code("en"), Some(Lang::En));
        assert_eq!(Lang::from_code("ru_RU"), None);
        assert_eq!(Lang::from_code(""), None);
        assert_eq!(Lang::from_locale("ru_RU.UTF-8"), Some(Lang::Ru));
        assert_eq!(Lang::from_locale("zh_CN.UTF-8"), Some(Lang::Zh));
        assert_eq!(Lang::from_locale("en_US.UTF-8"), Some(Lang::En));
        assert_eq!(Lang::from_locale("en"), Some(Lang::En));
        assert_eq!(Lang::from_locale("C"), None);
        assert_eq!(Lang::from_locale("POSIX"), None);
        // The file wins over LANG; with neither, Chinese — as in the session script.
        assert_eq!(startup(Some("ru"), Some("zh_CN.UTF-8")), Lang::Ru);
        assert_eq!(startup(Some("garbage"), Some("en_US.UTF-8")), Lang::En);
        assert_eq!(startup(None, Some("ru_RU.UTF-8")), Lang::Ru);
        assert_eq!(startup(None, None), Lang::Zh);
        assert_eq!(startup(None, Some("C")), Lang::Zh);
        for lang in Lang::ALL {
            assert_eq!(Lang::from_code(lang.code()), Some(lang));
            assert_eq!(Lang::from_locale(lang.locale()), Some(lang));
            assert!(lang.desktop_keys().contains(&lang.code()));
        }
        assert_eq!(Lang::ALL.map(Lang::own_name), ["中文", "Русский", "English"]);
    }

    #[test]
    fn dates() {
        assert_eq!(date(Lang::Zh, 5, 10, 20), "11月20日 周五");
        assert_eq!(date(Lang::Ru, 5, 10, 20), "Пт, 20 ноября");
        assert_eq!(date(Lang::En, 5, 10, 20), "Fri, 20 Nov");
        assert_eq!(date(Lang::Ru, 0, 0, 1), "Вс, 1 января");
        assert_eq!(date(Lang::Ru, 3, 4, 9), "Ср, 9 мая");
        assert_eq!(date(Lang::En, 99, 99, 31), "Sat, 31 Dec");
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(Lang::En, 0), "0 B");
        assert_eq!(human_size(Lang::Zh, 999), "999 B");
        assert_eq!(human_size(Lang::En, 12_400_000), "12.4 MB");
        assert_eq!(human_size(Lang::En, 3_000_067), "3.0 MB");
        assert_eq!(human_size(Lang::En, 999_999_999), "1.0 GB");
        assert_eq!(human_size(Lang::En, 8 << 30), "8.6 GB");
        assert_eq!(human_size(Lang::Ru, 12_400_000), "12,4 МБ");
        assert_eq!(human_size(Lang::Ru, 512), "512 Б");
    }

    #[test]
    fn saving_the_word() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("shabnam-lang-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("locale");
        let path = path.to_str().unwrap();
        for lang in [Lang::Ru, Lang::En, Lang::Zh] {
            save_to(path, lang).unwrap();
            let text = std::fs::read_to_string(path).unwrap();
            assert_eq!(text, format!("{}\n", lang.code()));
            assert_eq!(Lang::from_code(&text), Some(lang));
            assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o644);
            assert!(!std::path::Path::new(&format!("{path}.new")).exists());
        }
        // A directory that is not there: an error, nothing left behind.
        assert!(save_to(dir.join("missing/locale").to_str().unwrap(), Lang::Ru).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
