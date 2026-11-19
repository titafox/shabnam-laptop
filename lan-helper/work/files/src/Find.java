package local.cloud.launcher;

import android.content.Context;
import android.content.Intent;
import android.graphics.drawable.Drawable;
import android.os.Environment;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.text.SpannableString;
import android.text.Spanned;
import android.text.style.ForegroundColorSpan;
import android.text.style.RelativeSizeSpan;
import android.util.Log;
import android.view.View;
import android.widget.ImageView;
import android.widget.PopupMenu;
import android.widget.TextView;
import java.io.File;
import java.util.*;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * 启动器搜索框的「统一搜索」：除了应用，再给出几条系统设置入口和共享存储里名字匹配的文件。
 * 设置项是即时的；文件在后台线程里找（有数量和时间上限），找到后追加到结果列表末尾。
 */
final class Find {
    static final String TAG = "Find";
    static final int SETTING = 1, FILE = 2;
    static final int MAX_FILES = 8, MAX_SETTINGS = 4;

    static final class Hit {
        int kind;
        String title, sub;
        Intent intent;
        File file;
        boolean dir;
    }
    interface Stop { boolean stop(); }

    static final ExecutorService ex = Executors.newSingleThreadExecutor();
    static final Handler ui = new Handler(Looper.getMainLooper());
    static volatile int gen;

    // ================= 接到 Desk 上 =================
    /** Desk.refreshRows() 在搜索时调用：设置项立刻加进 items，文件稍后追加。 */
    static void add(final Desk d, final String q) {
        final int g = ++gen;
        try {
            d.items.addAll(settings(d.a, q));
        } catch (Throwable e) { Log.w(TAG, "settings: " + e); }
        if (q.length() < 2 || Desk.looksLikeUrl(q) && q.contains("/")) return;
        ex.execute(() -> {
            try {
                Thread.sleep(140); // 连续打字时只找最后一次
                if (g != gen) return;
                final List<Hit> hits = files(q, () -> g != gen);
                if (g != gen || hits.isEmpty()) return;
                ui.post(() -> {
                    try { append(d, q, g, hits); } catch (Throwable e) { Log.w(TAG, "append: " + e); }
                });
            } catch (Throwable e) { Log.w(TAG, "files: " + e); }
        });
    }
    private static void append(Desk d, String q, int g, List<Hit> hits) {
        if (g != gen || d.a.desk != d || d.list == null || !d.searching) return;
        if (!q.equals(d.search.getText().toString().trim())) return;
        if (d.items.isEmpty()) d.rows.clear(); // 去掉「没有匹配」那一行
        int base = d.items.size();
        d.items.addAll(hits);
        for (int i = 0; i < hits.size(); i++) {
            Desk.Row r = new Desk.Row();
            r.type = 2; r.base = base + i; r.count = 1;
            d.rows.add(r);
        }
        if (d.sel < 0) d.sel = 0;
        d.adapter.notifyDataSetChanged();
    }

    /** 结果行的显示与点击。row 是 Desk 的结果行，iv / t 是它的图标和标题。 */
    static void bind(final Desk d, View row, ImageView iv, TextView t, final Hit h) {
        Drawable icon = h.kind == SETTING ? new FI.Glyph(FI.GEAR, T.accent) : FI.icon(h.title, h.dir);
        iv.setImageDrawable(icon);
        if (h.sub == null || h.sub.isEmpty()) t.setText(h.title);
        else {
            SpannableString s = new SpannableString(h.title + "   " + h.sub);
            int from = h.title.length();
            s.setSpan(new ForegroundColorSpan(T.text2), from, s.length(), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
            s.setSpan(new RelativeSizeSpan(0.8f), from, s.length(), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
            t.setText(s);
        }
        row.setOnClickListener(x -> open(d, h));
        View.OnLongClickListener menu = x -> {
            if (h.kind != FILE) return false;
            try {
                PopupMenu m = new PopupMenu(d.themed(), x);
                m.getMenu().add(0, 1, 0, L.t("打开", "Открыть", "Open"));
                m.getMenu().add(0, 2, 1, L.t("在文件夹中显示", "Показать в папке", "Show in folder"));
                m.setOnMenuItemClickListener(mi -> {
                    if (mi.getItemId() == 1) open(d, h);
                    else { d.hidePanels(); Files.start(d.a, h.file.getParent(), h.file.getAbsolutePath()); }
                    return true;
                });
                m.show();
            } catch (Throwable e) { Log.w(TAG, "menu: " + e); }
            return true;
        };
        row.setOnLongClickListener(menu);
        row.setOnContextClickListener(menu::onLongClick);
    }
    static void open(Desk d, Hit h) {
        try {
            d.hidePanels();
            if (h.kind == SETTING) d.open(new Intent(h.intent));
            else if (h.dir) Files.start(d.a, h.file.getAbsolutePath(), null);
            else Files.view(d.a, h.file, false);
        } catch (Throwable e) { Log.w(TAG, "open: " + e); }
    }

    // ================= 设置入口 =================
    /** { 中文, 俄文, 英文, 其他关键词, 意图动作 } */
    static final String[][] SETTINGS = {
        {"Wi‑Fi 网络", "Wi‑Fi", "Wi‑Fi", "wifi wlan 无线 网络 сеть вайфай интернет network internet wangluo wuxian", "android.settings.WIFI_SETTINGS"},
        {"蓝牙", "Bluetooth", "Bluetooth", "bluetooth 蓝牙 блютуз lanya", "android.settings.BLUETOOTH_SETTINGS"},
        {"显示", "Экран", "Display", "display screen 屏幕 显示 亮度 分辨率 экран яркость дисплей brightness xianshi pingmu liangdu", "android.settings.DISPLAY_SETTINGS"},
        {"语言", "Язык", "Language", "language locale 语言 язык языки yuyan", "android.settings.LOCALE_SETTINGS"},
        {"应用管理", "Приложения", "Apps", "apps applications 应用 程序 卸载 приложения программы uninstall yingyong", "android.settings.MANAGE_APPLICATIONS_SETTINGS"},
        {"关于本机", "Об устройстве", "About device", "about device 关于 本机 版本 型号 устройство о телефоне version guanyu", "android.settings.DEVICE_INFO_SETTINGS"},
        {"声音", "Звук", "Sound", "sound volume audio 声音 音量 звук громкость shengyin yinliang", "android.settings.SOUND_SETTINGS"},
        {"日期和时间", "Дата и время", "Date & time", "date time clock 日期 时间 时区 дата время часы часовой пояс riqi shijian", "android.settings.DATE_SETTINGS"},
        {"存储", "Хранилище", "Storage", "storage disk space 存储 空间 磁盘 память хранилище диск cunchu", "android.settings.INTERNAL_STORAGE_SETTINGS"},
        {"键盘与输入法", "Клавиатура и ввод", "Keyboard & input", "keyboard input ime 键盘 输入法 клавиатура ввод раскладка jianpan shurufa", "android.settings.INPUT_METHOD_SETTINGS"},
        {"电池", "Батарея", "Battery", "battery power 电池 电量 省电 батарея аккумулятор питание dianchi", "android.intent.action.POWER_USAGE_SUMMARY"},
        {"开发者选项", "Для разработчиков", "Developer options", "developer debug adb 开发者 调试 разработчик отладка kaifazhe", "android.settings.APPLICATION_DEVELOPMENT_SETTINGS"},
    };
    static List<Hit> settings(Context c, String query) {
        List<Hit> out = new ArrayList<>();
        String q = query.toLowerCase(Locale.ROOT).trim();
        if (q.isEmpty()) return out;
        String tail = L.t("系统设置", "Настройки системы", "System settings");
        for (String[] s : SETTINGS) {
            boolean hit = false;
            for (int i = 0; i < 3 && !hit; i++) {
                String low = s[i].toLowerCase(Locale.ROOT).replace('‑', '-');
                hit = low.startsWith(q) || q.length() >= 2 && low.contains(q) || low.replace("-", "").startsWith(q);
            }
            if (!hit) for (String k : s[3].split(" ")) if (k.startsWith(q) && (q.length() >= 2 || k.length() == 1)) { hit = true; break; }
            if (!hit) continue;
            Intent i = new Intent(s[4]);
            try {
                // 这台机器上泛用的设置意图可能被定制的「设置」接走：能点名原生设置就点名
                Intent named = new Intent(s[4]).setPackage("com.android.settings");
                if (c.getPackageManager().resolveActivity(named, 0) != null) i = named;
                else if (c.getPackageManager().resolveActivity(i, 0) == null) continue;
            } catch (Throwable ignored) {}
            Hit h = new Hit();
            h.kind = SETTING; h.title = L.t(s[0], s[1], s[2]); h.sub = tail; h.intent = i;
            out.add(h);
            if (out.size() >= MAX_SETTINGS) break;
        }
        return out;
    }

    // ================= 文件 =================
    static List<Hit> files(String q, Stop stop) {
        File home = Environment.getExternalStorageDirectory();
        List<File> found = walk(home, q, 30000, 60, 1500, false, stop);
        final String low = q.toLowerCase(Locale.ROOT).trim();
        final int base = home.getAbsolutePath().length();
        Collections.sort(found, (x, y) -> {
            int rx = rank(x, low), ry = rank(y, low);
            if (rx != ry) return rx - ry;
            return Integer.compare(x.getAbsolutePath().length(), y.getAbsolutePath().length());
        });
        List<Hit> out = new ArrayList<>();
        for (File f : found) {
            Hit h = new Hit();
            h.kind = FILE; h.file = f; h.title = f.getName(); h.dir = f.isDirectory();
            String parent = f.getParent() == null ? "" : f.getParent();
            h.sub = parent.length() > base ? parent.substring(base + 1) : L.t("主目录", "Домашняя папка", "Home");
            out.add(h);
            if (out.size() >= MAX_FILES) break;
        }
        return out;
    }
    private static int rank(File f, String low) {
        String n = f.getName().toLowerCase(Locale.ROOT);
        return n.equals(low) ? 0 : n.startsWith(low) ? 1 : 2;
    }

    /**
     * 按名字找文件（不区分大小写，空格分开的每个词都要出现）。逐层往下找，浅层的先出来。
     * 有三道上限：最多看 maxVisit 个条目、最多 maxHits 个结果、最多 maxMs 毫秒。
     */
    static List<File> walk(File root, String query, int maxVisit, int maxHits, long maxMs, boolean hiddenToo, Stop stop) {
        List<File> out = new ArrayList<>();
        String[] words = query.toLowerCase(Locale.ROOT).trim().split("\\s+");
        if (words.length == 0 || words[0].isEmpty()) return out;
        long end = SystemClock.uptimeMillis() + maxMs;
        ArrayDeque<File> queue = new ArrayDeque<>();
        queue.add(root);
        int seen = 0;
        boolean top = true;
        while (!queue.isEmpty()) {
            File d = queue.poll();
            String[] names = d.list();
            boolean isTop = top;
            top = false;
            if (names == null) continue;
            for (String n : names) {
                if (++seen > maxVisit || out.size() >= maxHits) return out;
                if ((seen & 63) == 0 && (SystemClock.uptimeMillis() > end || stop != null && stop.stop())) return out;
                if (!hiddenToo && n.startsWith(".")) continue;
                File f = new File(d, n);
                String low = n.toLowerCase(Locale.ROOT);
                boolean all = true;
                for (String w : words) if (!low.contains(w)) { all = false; break; }
                if (all) out.add(f);
                // 应用私有数据目录又深又多，而且读不到，不进去
                if (isTop && n.equals("Android")) continue;
                if (f.isDirectory()) queue.add(f);
            }
        }
        return out;
    }
}
