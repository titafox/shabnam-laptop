package local.cloud.launcher;

import android.app.DownloadManager;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.ContentResolver;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.res.ColorStateList;
import android.database.Cursor;
import android.graphics.PixelFormat;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.LayerDrawable;
import android.media.MediaScannerConnection;
import android.net.Uri;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.provider.DocumentsContract;
import android.provider.OpenableColumns;
import android.text.InputType;
import android.text.TextUtils;
import android.util.DisplayMetrics;
import android.util.Log;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.View;
import android.view.WindowManager;
import android.view.inputmethod.EditorInfo;
import android.widget.*;
import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.util.*;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * 互传的界面与对外入口。
 *   Send.open(context)              打开「互传」面板（附近设备列表）
 *   Send.sendFiles(context, uris)   带着选好的文件打开面板，点一台设备即发送
 * 面板和右下角的提示卡片都是系统悬浮窗，在任何应用上方都能出现。
 * 所有入口都自带兜底：出错只记日志，不会让桌面崩溃。
 */
public final class Send {
    static final String TAG = "Send";
    static Context app;
    static final Handler ui = new Handler(Looper.getMainLooper());
    /** 核心里的收发、取消都会碰网络，一律放到这条后台线程上做，不能在界面线程里调。 */
    static final ExecutorService bg = Executors.newSingleThreadExecutor();
    static final List<Uri> picked = new ArrayList<>();
    static LsCore.Device pendingTarget;

    // ================= 对外入口 =================
    public static void open(final Context c) {
        post(new Runnable() { public void run() { init(c); showPanel(); } });
    }
    public static void sendFiles(final Context c, final List<Uri> uris) {
        final List<Uri> copy = uris == null ? new ArrayList<Uri>() : new ArrayList<>(uris);
        post(new Runnable() { public void run() {
            init(c);
            picked.clear();
            picked.addAll(copy);
            LsCore.Device d = pendingTarget;
            pendingTarget = null;
            if (d != null && !picked.isEmpty()) sendTo(d); else showPanel();
        } });
    }

    static void post(final Runnable r) {
        ui.post(new Runnable() { public void run() { try { r.run(); } catch (Throwable e) { Log.w(TAG, "ui: " + e, e); } } });
    }
    static void bg(final Runnable r) {
        try { bg.execute(new Runnable() { public void run() { try { r.run(); } catch (Throwable e) { Log.w(TAG, "bg: " + e, e); } } }); }
        catch (Throwable e) { Log.w(TAG, "bg: " + e); }
    }
    static void init(Context c) {
        if (c != null) app = c.getApplicationContext() != null ? c.getApplicationContext() : c;
        if (T.panel == 0) { // 桌面还没画过（例如安全模式）：自己把主题定下来
            DisplayMetrics dm = app.getResources().getDisplayMetrics();
            T.u = Math.max(dm.widthPixels, dm.heightPixels) / 1280f;
            T.apply(Store.ctx(app).getSharedPreferences("desk", 0).getBoolean("dark", true));
        }
        if (SendService.core == null) SendService.start(app);
    }
    static SharedPreferences prefs() { return Store.ctx(app).getSharedPreferences("send", 0); }
    static void toast(String s) { try { Toast.makeText(app, s, Toast.LENGTH_SHORT).show(); } catch (Throwable ignored) {} }

    // ================= 核心 → 界面 =================
    static final LsCore.Listener LISTENER = new LsCore.Listener() {
        public void onDevices() { post(new Runnable() { public void run() { renderDevices(); } }); }
        public void onIncoming(final LsCore.Incoming s) { post(new Runnable() { public void run() { incoming(s); } }); }
        public void onIncomingUpdate(final LsCore.Incoming s) { post(new Runnable() { public void run() { incomingUpdate(s); } }); }
        public void onText(final LsCore.Device from, final String text) { post(new Runnable() { public void run() { textCard(from, text); } }); }
        public void onOutgoingUpdate(final LsCore.Outgoing o) { post(new Runnable() { public void run() { outgoingUpdate(o); } }); }
        public void log(String s) { Log.i(TAG, s); }
    };

    // ================= 小部件 =================
    static TextView tv(String s, float size, int color) {
        TextView t = new TextView(app);
        t.setText(s); t.setTextColor(color);
        t.setTextSize(TypedValue.COMPLEX_UNIT_PX, T.px(size));
        t.setIncludeFontPadding(false);
        return t;
    }
    static TextView button(String label, boolean primary, final Runnable act) {
        TextView b = tv(label, 12.5f, primary ? T.onAccent : T.text);
        b.setGravity(Gravity.CENTER);
        b.setPadding(T.px(14), 0, T.px(14), 0);
        b.setMinWidth(T.px(64));
        b.setBackground(new LayerDrawable(new Drawable[]{T.round(primary ? T.accent : T.chip, T.px(15)), T.state(T.px(15))}));
        b.setFocusable(true);
        b.setOnClickListener(new View.OnClickListener() { public void onClick(View v) { try { act.run(); } catch (Throwable e) { Log.w(TAG, "click: " + e, e); } } });
        return b;
    }
    static String size(long b) {
        if (b < 1024) return b + " B";
        if (b < 1024 * 1024) return String.format(Locale.US, "%.1f KB", b / 1024f);
        if (b < 1024L * 1024 * 1024) return String.format(Locale.US, "%.1f MB", b / 1048576f);
        return String.format(Locale.US, "%.2f GB", b / 1073741824f);
    }
    static String files(int n) { return L.t(n + " 个文件", "файлов: " + n, n == 1 ? "1 file" : n + " files"); }
    static int overlayType() { return Build.VERSION.SDK_INT >= 26 ? WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY : WindowManager.LayoutParams.TYPE_PHONE; }
    static WindowManager wm() { return (WindowManager) app.getSystemService(Context.WINDOW_SERVICE); }

    // ================= 右下角的提示卡片 =================
    static LinearLayout stack;
    static final Map<Object, Card> cards = new HashMap<>();

    /** 传输卡片当前占的高度，通知卡片据此叠在它上面而不是压住它。 */
    static int stackHeight() {
        LinearLayout s = stack;
        if (s == null || s.getHeight() == 0) return 0;
        int[] xy = new int[2];
        s.getLocationOnScreen(xy);
        android.graphics.Point sz = new android.graphics.Point();
        wm().getDefaultDisplay().getRealSize(sz);
        return Math.max(0, sz.y - xy[1] - Tray.shelfHeight()); // 从底栏上沿到最上面一张卡片顶部
    }
    static void ensureStack() {
        if (stack != null) return;
        LinearLayout s = new LinearLayout(app);
        s.setOrientation(LinearLayout.VERTICAL);
        WindowManager.LayoutParams lp = new WindowManager.LayoutParams(-2, -2, overlayType(),
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE | WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL | WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            PixelFormat.TRANSLUCENT);
        lp.gravity = Gravity.BOTTOM | Gravity.RIGHT;
        lp.x = T.px(10); lp.y = T.px(62);
        lp.setTitle("OpenDesktopSend");
        wm().addView(s, lp); // 没有悬浮窗权限时这里会抛出，由调用方处理
        stack = s;
        s.addOnLayoutChangeListener(new View.OnLayoutChangeListener() {
            public void onLayoutChange(View v, int l, int t, int r, int b, int ol, int ot, int or, int ob) { ui.post(new Runnable() { public void run() { Notif.place(); } }); }
        });
    }

    static final class Card {
        final LinearLayout v, btns;
        final TextView title, body;
        final ProgressBar bar;
        final CheckBox check;
        Runnable auto;
        boolean closed;

        Card() {
            ensureStack();
            v = new LinearLayout(app);
            v.setOrientation(LinearLayout.VERTICAL);
            v.setBackground(T.round(T.panel | 0xFF000000, T.px(16)));
            v.setPadding(T.px(16), T.px(14), T.px(16), T.px(12));
            v.setClickable(true);
            title = tv("", 14, T.text);
            title.setSingleLine(true);
            title.setEllipsize(TextUtils.TruncateAt.END);
            v.addView(title);
            body = tv("", 12, T.text2);
            body.setPadding(0, T.px(6), 0, 0);
            body.setMaxLines(8);
            body.setEllipsize(TextUtils.TruncateAt.END);
            body.setLineSpacing(T.px(3), 1f);
            v.addView(body);
            bar = new ProgressBar(app, null, android.R.attr.progressBarStyleHorizontal);
            bar.setMax(1000);
            bar.setProgressTintList(ColorStateList.valueOf(T.accent));
            bar.setIndeterminateTintList(ColorStateList.valueOf(T.accent));
            bar.setVisibility(View.GONE);
            LinearLayout.LayoutParams bl = new LinearLayout.LayoutParams(-1, T.px(6));
            bl.topMargin = T.px(10);
            v.addView(bar, bl);
            check = new CheckBox(app);
            check.setTextColor(T.text2);
            check.setTextSize(TypedValue.COMPLEX_UNIT_PX, T.px(11.5f));
            check.setButtonTintList(ColorStateList.valueOf(T.accent));
            check.setVisibility(View.GONE);
            v.addView(check);
            btns = new LinearLayout(app);
            btns.setGravity(Gravity.RIGHT);
            LinearLayout.LayoutParams al = new LinearLayout.LayoutParams(-1, -2);
            al.topMargin = T.px(10);
            v.addView(btns, al);
            LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(T.px(340), -2);
            lp.topMargin = T.px(8);
            stack.addView(v, lp);
        }
        Card set(String t, String b) {
            title.setText(t);
            body.setText(b == null ? "" : b);
            body.setVisibility(b == null || b.isEmpty() ? View.GONE : View.VISIBLE);
            return this;
        }
        /** 0..1000 显示进度；-1 表示还不知道（来回滚动）；-2 隐藏。 */
        Card progress(int permille) {
            bar.setVisibility(permille == -2 ? View.GONE : View.VISIBLE);
            bar.setIndeterminate(permille == -1);
            if (permille >= 0) bar.setProgress(permille);
            return this;
        }
        /** 参数成对：按钮文字、点了做什么。第一个是主按钮。 */
        Card actions(Object... pairs) {
            btns.removeAllViews();
            for (int i = 0; i + 1 < pairs.length; i += 2) {
                LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(-2, T.px(30));
                lp.leftMargin = T.px(8);
                btns.addView(button((String) pairs[i], i == 0, (Runnable) pairs[i + 1]), lp);
            }
            return this;
        }
        void closeIn(long ms) {
            if (auto != null) ui.removeCallbacks(auto);
            auto = new Runnable() { public void run() { close(); } };
            ui.postDelayed(auto, ms);
        }
        void close() {
            if (closed) return;
            closed = true;
            if (auto != null) ui.removeCallbacks(auto);
            try {
                stack.removeView(v);
                if (stack.getChildCount() == 0) { wm().removeViewImmediate(stack); stack = null; }
            } catch (Throwable e) { Log.w(TAG, "card close: " + e); stack = null; }
            Notif.place();
        }
        final Runnable closer = new Runnable() { public void run() { close(); } };
    }
    static Card card(Object key) {
        Card c = cards.get(key);
        if (c == null || c.closed) { c = new Card(); cards.put(key, c); }
        return c;
    }
    /** 已经报过最终结果的传输：协议层有时会为同一次传输再通知一遍（比如取消），不能再弹第二张卡片。 */
    static final Set<Object> finished = Collections.newSetFromMap(new WeakHashMap<Object, Boolean>());
    static void drop(Object key) { cards.remove(key); finished.add(key); }

    // ================= 接收 =================
    static Set<String> trusted() {
        Set<String> s = new HashSet<>();
        for (String x : prefs().getString("trusted", "").split("\n")) if (!x.isEmpty()) s.add(x);
        return s;
    }
    static void trust(String fp) {
        Set<String> s = trusted();
        if (fp == null || fp.isEmpty() || !s.add(fp)) return;
        prefs().edit().putString("trusted", TextUtils.join("\n", s)).apply();
    }
    static String fileList(LsCore.Incoming s) {
        StringBuilder b = new StringBuilder(files(s.files.size())).append(" · ").append(size(s.total));
        for (int i = 0; i < s.files.size() && i < 4; i++) {
            LsCore.FileMeta f = s.files.get(i);
            b.append('\n').append(f.name).append("  ").append(size(f.size));
        }
        if (s.files.size() > 4) b.append("\n…");
        return b.toString();
    }
    static void incoming(final LsCore.Incoming s) {
        // 放不下就直接拒收（留 300 MB 余量），不必再问
        try {
            long free = android.os.Environment.getExternalStorageDirectory().getUsableSpace();
            if (free > 0 && s.total > free - (300L << 20)) {
                bg(new Runnable() { public void run() { s.decline(); } });
                toast(s.sender.alias + L.t(" 发来的文件放不下，已拒收", ": недостаточно места, передача отклонена", ": not enough free space, transfer declined"));
                return;
            }
        } catch (Throwable ignored) {}
        // "总是接受"只对核实过身份的设备生效：回连对方、比对证书，核实不了就照常询问
        if (trusted().contains(s.sender.fingerprint)) {
            bg(new Runnable() { public void run() {
                if (s.core.verify(s.sender)) s.accept();
                else ui.post(new Runnable() { public void run() { ask(s); } });
            } });
            return;
        }
        ask(s);
    }
    static void ask(final LsCore.Incoming s) {
        final Card c;
        try { c = card(s); }
        catch (Throwable e) { // 卡片弹不出来就没法征求同意：稳妥起见一律拒收
            Log.w(TAG, "prompt: " + e);
            bg(new Runnable() { public void run() { s.decline(); } });
            toast(L.t("无法显示接收提示，已拒收", "Не удалось показать запрос, передача отклонена", "Could not show the prompt; transfer declined"));
            return;
        }
        c.set(s.sender.alias + L.t(" 想发送文件给你", " хочет отправить вам файлы", " wants to send you files"), fileList(s));
        c.check.setText(L.t("以后总是接受这台设备", "Всегда принимать от этого устройства", "Always accept from this device"));
        c.check.setChecked(false);
        c.check.setVisibility(View.VISIBLE);
        c.progress(-2);
        c.actions(L.t("接受", "Принять", "Accept"), new Runnable() { public void run() {
            if (c.check.isChecked()) trust(s.sender.fingerprint);
            c.actions();
            bg(new Runnable() { public void run() { s.accept(); } });
        } }, L.t("拒绝", "Отклонить", "Decline"), new Runnable() { public void run() {
            bg(new Runnable() { public void run() { s.decline(); } });
        } });
    }
    static void incomingUpdate(final LsCore.Incoming s) {
        if (s.state == LsCore.Incoming.DECLINED) { Card c = cards.remove(s); if (c != null) c.close(); return; }
        if (s.state == LsCore.Incoming.WAITING || finished.contains(s)) return;
        final Card c;
        try { c = card(s); } catch (Throwable e) { Log.w(TAG, "card: " + e); return; }
        c.check.setVisibility(View.GONE);
        String from = s.sender.alias;
        Runnable close = c.closer;
        switch (s.state) {
            case LsCore.Incoming.RECEIVING:
                c.set(L.t("正在接收 · ", "Получение · ", "Receiving · ") + from, size(s.done) + " / " + size(s.total));
                c.progress(s.total > 0 ? (int) (s.done * 1000 / s.total) : -1);
                if (c.btns.getChildCount() != 1) c.actions(L.t("取消", "Отмена", "Cancel"), new Runnable() { public void run() { bg(new Runnable() { public void run() { s.cancel(); } }); } });
                return;
            case LsCore.Incoming.DONE: {
                final List<String> saved = new ArrayList<>(s.saved);
                String where = L.t("来自 ", "От: ", "From ") + from + L.t(" · 保存在「下载/LocalSend」", " · сохранено в «Загрузки/LocalSend»", " · saved to Download/LocalSend");
                if (s.failed > 0 || saved.size() < s.files.size())
                    where += "\n" + L.t("有 " + (s.files.size() - saved.size()) + " 个没有收到", "Не получено: " + (s.files.size() - saved.size()), (s.files.size() - saved.size()) + " not received");
                c.set(L.t("已接收 ", "Получено: ", "Received ") + files(saved.size()), where);
                c.progress(-2);
                Runnable folder = new Runnable() { public void run() { showFolder(); c.close(); } };
                if (saved.size() == 1) c.actions(L.t("打开", "Открыть", "Open"), new Runnable() { public void run() { openRef(saved.get(0)); c.close(); } },
                    L.t("在文件夹中显示", "Показать в папке", "Show in folder"), folder, L.t("关闭", "Закрыть", "Close"), close);
                else c.actions(L.t("在文件夹中显示", "Показать в папке", "Show in folder"), folder, L.t("关闭", "Закрыть", "Close"), close);
                c.closeIn(120000);
                break;
            }
            case LsCore.Incoming.CANCELLED:
                c.set(L.t("传输已取消", "Передача отменена", "Transfer cancelled"), from);
                c.progress(-2); c.actions(L.t("关闭", "Закрыть", "Close"), close); c.closeIn(6000);
                break;
            default:
                c.set(L.t("接收失败", "Не удалось получить", "Receiving failed"), from + (s.error == null ? "" : "\n" + s.error));
                c.progress(-2); c.actions(L.t("关闭", "Закрыть", "Close"), close); c.closeIn(20000);
        }
        drop(s);
    }
    static void textCard(LsCore.Device from, final String text) {
        final Card c;
        try { c = new Card(); } catch (Throwable e) { Log.w(TAG, "card: " + e); toast(text); return; }
        c.set(from.alias + L.t(" 发来一段文字", " прислал(а) текст", " sent a text"), text.length() > 1200 ? text.substring(0, 1200) + "…" : text);
        c.body.setTextColor(T.text);
        final String t = text.trim();
        Runnable copy = new Runnable() { public void run() {
            ((ClipboardManager) app.getSystemService(Context.CLIPBOARD_SERVICE)).setPrimaryClip(ClipData.newPlainText("LocalSend", text));
            toast(L.t("已复制", "Скопировано", "Copied"));
            c.close();
        } };
        if ((t.startsWith("http://") || t.startsWith("https://")) && t.indexOf(' ') < 0 && t.indexOf('\n') < 0)
            c.actions(L.t("复制", "Копировать", "Copy"), copy, L.t("打开链接", "Открыть ссылку", "Open link"), new Runnable() { public void run() {
                try { app.startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(t)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)); } catch (Throwable e) { toast(L.t("打不开这个链接", "Не удалось открыть ссылку", "Could not open the link")); }
                c.close();
            } }, L.t("关闭", "Закрыть", "Close"), c.closer);
        else c.actions(L.t("复制", "Копировать", "Copy"), copy, L.t("关闭", "Закрыть", "Close"), c.closer);
    }

    static void openRef(String ref) {
        try {
            if (ref.startsWith("content:")) { view(Uri.parse(ref)); return; }
            // 直接写进文件夹的（老系统）：让媒体库登记一下，拿到可以交给别的应用的地址
            MediaScannerConnection.scanFile(app, new String[]{ref}, null, new MediaScannerConnection.OnScanCompletedListener() {
                public void onScanCompleted(String path, final Uri uri) { post(new Runnable() { public void run() { if (uri != null) view(uri); else showFolder(); } }); }
            });
        } catch (Throwable e) { Log.w(TAG, "open: " + e); showFolder(); }
    }
    static void view(Uri u) {
        try {
            app.startActivity(new Intent(Intent.ACTION_VIEW).setDataAndType(u, app.getContentResolver().getType(u))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_GRANT_READ_URI_PERMISSION));
        } catch (Throwable e) { toast(L.t("没有能打开这个文件的应用", "Нет приложения для этого файла", "No app can open this file")); }
    }
    /** 用系统的「文件」应用打开 下载/LocalSend。 */
    static void showFolder() {
        try {
            Uri dir = DocumentsContract.buildDocumentUri("com.android.externalstorage.documents", "primary:Download/" + SendService.FOLDER);
            app.startActivity(new Intent(Intent.ACTION_VIEW).setDataAndType(dir, DocumentsContract.Document.MIME_TYPE_DIR).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        } catch (Throwable e) {
            try { app.startActivity(new Intent(DownloadManager.ACTION_VIEW_DOWNLOADS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)); }
            catch (Throwable e2) { toast(L.t("文件在「下载/LocalSend」里", "Файлы лежат в «Загрузки/LocalSend»", "Files are in Download/LocalSend")); }
        }
    }

    // ================= 发送 =================
    /** 把一个 content/file 地址包装成核心能读的文件。名字和大小第一次用到时才去查（在后台线程）。 */
    static LsCore.Source source(final Uri u) {
        return new LsCore.Source() {
            String name, mime; long size = -1; boolean loaded;
            synchronized void load() {
                if (loaded) return;
                loaded = true;
                if ("file".equals(u.getScheme())) {
                    File f = new File(u.getPath());
                    name = f.getName(); size = f.length();
                } else {
                    ContentResolver cr = app.getContentResolver();
                    try { mime = cr.getType(u); } catch (Throwable ignored) {}
                    try (Cursor c = cr.query(u, null, null, null, null)) {
                        if (c != null && c.moveToFirst()) {
                            int ni = c.getColumnIndex(OpenableColumns.DISPLAY_NAME), si = c.getColumnIndex(OpenableColumns.SIZE);
                            if (ni >= 0) name = c.getString(ni);
                            if (si >= 0 && !c.isNull(si)) size = c.getLong(si);
                        }
                    } catch (Throwable ignored) {}
                    if (size < 0) { // 有些来源不报大小：读一遍数出来
                        try (InputStream in = cr.openInputStream(u)) {
                            long n = 0; int k; byte[] b = new byte[65536];
                            while ((k = in.read(b)) > 0) n += k;
                            size = n;
                        } catch (Throwable e) { size = 0; }
                    }
                }
                if (name == null || name.isEmpty()) name = u.getLastPathSegment() == null ? "file" : u.getLastPathSegment();
            }
            public String name() { load(); return name; }
            public long size() { load(); return size; }
            public String mime() { load(); return mime; }
            public InputStream open() throws IOException {
                if ("file".equals(u.getScheme())) return new FileInputStream(u.getPath());
                InputStream in = app.getContentResolver().openInputStream(u);
                if (in == null) throw new IOException("cannot open " + u);
                return in;
            }
        };
    }
    static void sendTo(final LsCore.Device d) {
        final List<Uri> uris = new ArrayList<>(picked);
        picked.clear();
        closePanel();
        bg(new Runnable() { public void run() {
            LsCore core = SendService.core;
            if (core == null) return;
            List<LsCore.Source> srcs = new ArrayList<>();
            for (Uri u : uris) srcs.add(source(u));
            final LsCore.Outgoing o = core.send(d, srcs, null);
            post(new Runnable() { public void run() { outgoingUpdate(o); } });
        } });
    }
    static void sendText(final LsCore.Device d, final String text) {
        closePanel();
        bg(new Runnable() { public void run() {
            LsCore core = SendService.core;
            if (core == null) return;
            final LsCore.Outgoing o = core.sendText(d, text, null);
            post(new Runnable() { public void run() { outgoingUpdate(o); } });
        } });
    }
    static void outgoingUpdate(final LsCore.Outgoing o) {
        if (finished.contains(o)) return;
        final Card c;
        try { c = card(o); } catch (Throwable e) { Log.w(TAG, "card: " + e); return; }
        String to = o.to.alias;
        String what = o.text != null ? L.t("一段文字", "текст", "a text") : files(o.files.size()) + (o.total > 0 ? " · " + size(o.total) : "");
        Runnable cancel = new Runnable() { public void run() { bg(new Runnable() { public void run() { o.cancel(); } }); } };
        String close = L.t("关闭", "Закрыть", "Close");
        switch (o.state) {
            case LsCore.Outgoing.WAITING:
                c.set(L.t("等待 " + to + " 接受…", "Ожидание: " + to + "…", "Waiting for " + to + " to accept…"), what);
                c.progress(-1);
                if (c.btns.getChildCount() != 1) c.actions(L.t("取消", "Отмена", "Cancel"), cancel);
                return;
            case LsCore.Outgoing.SENDING:
                c.set(L.t("正在发送给 ", "Отправка: ", "Sending to ") + to, size(o.done) + " / " + size(o.total) + (o.current == null ? "" : "\n" + o.current));
                c.progress(o.total > 0 ? (int) (o.done * 1000 / o.total) : -1);
                if (c.btns.getChildCount() != 1) c.actions(L.t("取消", "Отмена", "Cancel"), cancel);
                return;
            case LsCore.Outgoing.DONE:
                c.set(L.t("已发送给 ", "Отправлено: ", "Sent to ") + to, what + (o.skipped > 0 ? "\n" + L.t("对方跳过了 " + o.skipped + " 个", "Пропущено получателем: " + o.skipped, o.skipped + " skipped by the receiver") : ""));
                c.closeIn(8000);
                break;
            case LsCore.Outgoing.DECLINED:
                c.set(to + L.t(" 没有接受", " отклонил(а) передачу", " declined"), what); c.closeIn(10000); break;
            case LsCore.Outgoing.BUSY:
                c.set(to + L.t(" 正忙", " сейчас занят(а)", " is busy"), L.t("对方正在进行另一次传输，稍后再试", "У получателя идёт другая передача, попробуйте позже", "The receiver is in another transfer; try again later")); c.closeIn(10000); break;
            case LsCore.Outgoing.PIN:
                c.set(to + L.t(" 设置了口令", " защищён PIN-кодом", " requires a PIN"), L.t("请先在对方的 LocalSend 设置里关掉口令再发", "Отключите PIN в настройках LocalSend получателя и повторите", "Turn the PIN off in the receiver's LocalSend settings and send again")); c.closeIn(15000); break;
            case LsCore.Outgoing.CANCELLED:
                c.set(L.t("发送已取消", "Отправка отменена", "Sending cancelled"), to); c.closeIn(6000); break;
            default:
                c.set(L.t("发送失败", "Не удалось отправить", "Sending failed"), to + (o.error == null ? "" : "\n" + o.error)); c.closeIn(20000);
        }
        c.progress(-2);
        c.actions(close, c.closer);
        drop(o);
    }

    // ================= 「互传」面板 =================
    static FrameLayout panelRoot;
    static LinearLayout devList;
    static TextView selInfo, me, emptyHint;
    static EditText textBox, aliasBox;

    static final Runnable panelTick = new Runnable() { public void run() {
        if (panelRoot == null) return;
        try { renderDevices(); renderMe(); } catch (Throwable e) { Log.w(TAG, "tick: " + e); }
        ui.postDelayed(this, 1500);
    } };

    static void closePanel() {
        ui.removeCallbacks(panelTick);
        FrameLayout r = panelRoot;
        panelRoot = null; devList = null;
        if (r != null) try { wm().removeViewImmediate(r); } catch (Throwable e) { Log.w(TAG, "panel close: " + e); }
    }
    /** clear=true：清空列表重新找（「刷新」按钮）；false：保留已有的，只是再喊一遍并扫描一次（打开面板时）。 */
    static void refresh(final boolean clear) {
        bg(new Runnable() { public void run() {
            LsCore c = SendService.core;
            if (c == null) return;
            if (clear) c.refresh(true); else { c.announce(); c.scan(); }
        } });
    }

    static void showPanel() {
        if (panelRoot != null) { renderSelection(); renderDevices(); return; }
        DisplayMetrics dm = app.getResources().getDisplayMetrics();
        int W = Math.max(dm.widthPixels, dm.heightPixels), H = Math.min(dm.widthPixels, dm.heightPixels);

        FrameLayout root = new FrameLayout(app) {
            public boolean dispatchKeyEvent(KeyEvent e) {
                if (e.getKeyCode() == KeyEvent.KEYCODE_ESCAPE || e.getKeyCode() == KeyEvent.KEYCODE_BACK) {
                    if (e.getAction() == KeyEvent.ACTION_UP) post(new Runnable() { public void run() { closePanel(); } });
                    return true;
                }
                return super.dispatchKeyEvent(e);
            }
        };
        root.setBackgroundColor(0x66000000);
        root.setOnClickListener(new View.OnClickListener() { public void onClick(View v) { closePanel(); } });

        LinearLayout p = new LinearLayout(app);
        p.setOrientation(LinearLayout.VERTICAL);
        p.setBackground(T.round(T.panel | 0xFF000000, T.px(18)));
        p.setElevation(T.px(10));
        p.setPadding(T.px(20), T.px(18), T.px(20), T.px(16));
        p.setClickable(true);
        root.addView(p, new FrameLayout.LayoutParams(Math.round(W * 0.38f), -2, Gravity.CENTER));

        // 标题
        LinearLayout head = new LinearLayout(app);
        head.setGravity(Gravity.CENTER_VERTICAL);
        LinearLayout ht = new LinearLayout(app);
        ht.setOrientation(LinearLayout.VERTICAL);
        ht.addView(tv(L.t("互传", "Передача файлов", "File transfer"), 17, T.text));
        TextView sub = tv(L.t("与同一 Wi-Fi 下装有 LocalSend 的手机、电脑互传文件", "Обмен файлами с телефонами и ПК с LocalSend в той же сети Wi-Fi", "Share files with phones and computers running LocalSend on the same Wi-Fi"), 11.5f, T.text2);
        sub.setPadding(0, T.px(5), 0, 0);
        ht.addView(sub);
        head.addView(ht, new LinearLayout.LayoutParams(0, -2, 1));
        head.addView(button("✕", false, new Runnable() { public void run() { closePanel(); } }), new LinearLayout.LayoutParams(T.px(34), T.px(30)));
        p.addView(head);

        // 本机名字
        LinearLayout mr = new LinearLayout(app);
        mr.setGravity(Gravity.CENTER_VERTICAL);
        me = tv("", 12, T.text2);
        me.setSingleLine(true);
        me.setEllipsize(TextUtils.TruncateAt.END);
        mr.addView(me, new LinearLayout.LayoutParams(0, -2, 1));
        aliasBox = input(L.t("这台电脑的名字", "Имя этого компьютера", "This computer's name"));
        aliasBox.setVisibility(View.GONE);
        aliasBox.setImeOptions(EditorInfo.IME_ACTION_DONE | EditorInfo.IME_FLAG_NO_EXTRACT_UI);
        aliasBox.setOnEditorActionListener(new TextView.OnEditorActionListener() {
            public boolean onEditorAction(TextView v, int id, KeyEvent ev) { saveAlias(); return true; }
        });
        mr.addView(aliasBox, new LinearLayout.LayoutParams(0, T.px(34), 1));
        LinearLayout.LayoutParams rl = new LinearLayout.LayoutParams(-2, T.px(30));
        rl.leftMargin = T.px(8);
        mr.addView(button(L.t("改名", "Имя", "Rename"), false, new Runnable() { public void run() {
            if (aliasBox.getVisibility() == View.VISIBLE) { saveAlias(); return; }
            LsCore c = SendService.core;
            aliasBox.setText(c == null ? "" : c.cfg.alias);
            aliasBox.setVisibility(View.VISIBLE);
            me.setVisibility(View.GONE);
            aliasBox.requestFocus();
            aliasBox.selectAll();
        } }), rl);
        LinearLayout.LayoutParams ml = new LinearLayout.LayoutParams(-1, -2);
        ml.topMargin = T.px(14);
        p.addView(mr, ml);

        // 要发什么
        LinearLayout sr = new LinearLayout(app);
        sr.setGravity(Gravity.CENTER_VERTICAL);
        sr.setBackground(T.round(T.chip, T.px(12)));
        sr.setPadding(T.px(14), T.px(8), T.px(8), T.px(8));
        selInfo = tv("", 12.5f, T.text);
        selInfo.setMaxLines(2);
        selInfo.setEllipsize(TextUtils.TruncateAt.END);
        sr.addView(selInfo, new LinearLayout.LayoutParams(0, -2, 1));
        sr.addView(button(L.t("选择文件…", "Выбрать файлы…", "Choose files…"), true, new Runnable() { public void run() { pendingTarget = null; pick(); } }), new LinearLayout.LayoutParams(-2, T.px(30)));
        LinearLayout.LayoutParams sl = new LinearLayout.LayoutParams(-1, -2);
        sl.topMargin = T.px(12);
        p.addView(sr, sl);

        textBox = input(L.t("或者在这里输入、粘贴要发送的文字", "Или введите либо вставьте текст для отправки", "Or type or paste a text to send"));
        LinearLayout.LayoutParams tl = new LinearLayout.LayoutParams(-1, T.px(38));
        tl.topMargin = T.px(8);
        p.addView(textBox, tl);

        // 设备
        LinearLayout dh = new LinearLayout(app);
        dh.setGravity(Gravity.CENTER_VERTICAL);
        dh.addView(tv(L.t("附近的设备 · 点一下就发送", "Устройства рядом · нажмите, чтобы отправить", "Nearby devices · click one to send"), 11.5f, T.text2), new LinearLayout.LayoutParams(0, -2, 1));
        dh.addView(button(L.t("刷新", "Обновить", "Refresh"), false, new Runnable() { public void run() { refresh(true); } }), new LinearLayout.LayoutParams(-2, T.px(28)));
        LinearLayout.LayoutParams dl = new LinearLayout.LayoutParams(-1, -2);
        dl.topMargin = T.px(16); dl.bottomMargin = T.px(6);
        p.addView(dh, dl);

        ScrollView sc = new ScrollView(app);
        sc.setVerticalScrollBarEnabled(false);
        devList = new LinearLayout(app);
        devList.setOrientation(LinearLayout.VERTICAL);
        sc.addView(devList);
        p.addView(sc, new LinearLayout.LayoutParams(-1, Math.round(H * 0.30f)));

        // 底部
        LinearLayout foot = new LinearLayout(app);
        foot.setGravity(Gravity.CENTER_VERTICAL);
        foot.addView(tv(L.t("收到的文件保存在「下载/LocalSend」", "Полученные файлы — в «Загрузки/LocalSend»", "Received files are saved to Download/LocalSend"), 11, T.text2), new LinearLayout.LayoutParams(0, -2, 1));
        foot.addView(button(L.t("打开文件夹", "Открыть папку", "Open folder"), false, new Runnable() { public void run() { closePanel(); showFolder(); } }), new LinearLayout.LayoutParams(-2, T.px(28)));
        LinearLayout.LayoutParams fl = new LinearLayout.LayoutParams(-1, -2);
        fl.topMargin = T.px(10);
        p.addView(foot, fl);

        WindowManager.LayoutParams lp = new WindowManager.LayoutParams(-1, -1, overlayType(),
            WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN, PixelFormat.TRANSLUCENT);
        lp.softInputMode = WindowManager.LayoutParams.SOFT_INPUT_ADJUST_PAN | WindowManager.LayoutParams.SOFT_INPUT_STATE_HIDDEN;
        lp.setTitle("OpenDesktopSendPanel");
        try { wm().addView(root, lp); }
        catch (Throwable e) {
            Log.w(TAG, "panel: " + e);
            toast(L.t("需要「显示在其他应用上层」权限才能打开互传", "Для передачи файлов нужно разрешение «Поверх других приложений»", "File transfer needs the \"Display over other apps\" permission"));
            return;
        }
        panelRoot = root;
        root.setFocusableInTouchMode(true);
        root.requestFocus();
        renderSelection();
        renderMe();
        renderDevices();
        refresh(false);
        ui.postDelayed(panelTick, 1500);
    }
    static EditText input(String hint) {
        EditText e = new EditText(app);
        e.setHint(hint);
        e.setHintTextColor(T.text2);
        e.setTextColor(T.text);
        e.setTextSize(TypedValue.COMPLEX_UNIT_PX, T.px(12.5f));
        e.setSingleLine(true);
        e.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
        e.setImeOptions(EditorInfo.IME_FLAG_NO_EXTRACT_UI);
        e.setBackground(T.round(T.chip, T.px(12)));
        e.setPadding(T.px(14), 0, T.px(14), 0);
        e.setGravity(Gravity.CENTER_VERTICAL);
        return e;
    }
    static void saveAlias() {
        String a = aliasBox.getText().toString().trim();
        aliasBox.setVisibility(View.GONE);
        me.setVisibility(View.VISIBLE);
        final LsCore c = SendService.core;
        if (a.isEmpty() || c == null) return;
        if (a.length() > 40) a = a.substring(0, 40);
        prefs().edit().putString("alias", a).apply();
        c.cfg.alias = a;
        renderMe();
        bg(new Runnable() { public void run() { c.announce(); } });
    }
    static void renderMe() {
        if (panelRoot == null || me == null) return;
        final LsCore c = SendService.core;
        if (c == null) { me.setText(L.t("互传服务正在启动…", "Служба передачи запускается…", "Starting the transfer service…")); return; }
        bg(new Runnable() { public void run() {
            final List<String> ips = c.localIps();
            post(new Runnable() { public void run() {
                if (me == null) return;
                String s = L.t("这台电脑：", "Этот компьютер: ", "This computer: ") + c.cfg.alias;
                if (ips.isEmpty()) s += L.t(" · 没有连接 Wi-Fi", " · нет подключения к Wi-Fi", " · not connected to Wi-Fi");
                else s += " · " + ips.get(0);
                me.setText(s);
            } });
        } });
    }
    static void renderSelection() {
        if (panelRoot == null || selInfo == null) return;
        if (picked.isEmpty()) selInfo.setText(L.t("还没有选择文件", "Файлы не выбраны", "No files chosen"));
        else selInfo.setText(L.t("已选 ", "Выбрано: ", "Chosen: ") + files(picked.size()));
    }
    static String lastDevKey = "";
    static void renderDevices() {
        if (panelRoot == null || devList == null) return;
        LsCore c = SendService.core;
        List<LsCore.Device> ds = c == null ? new ArrayList<LsCore.Device>() : c.devices();
        StringBuilder k = new StringBuilder();
        for (LsCore.Device d : ds) k.append(d.fingerprint).append(d.alias).append(d.ip).append(d.port).append(';');
        if (devList.getChildCount() > 0 && k.toString().equals(lastDevKey)) return;
        lastDevKey = k.toString();
        devList.removeAllViews();
        if (ds.isEmpty()) {
            TextView t = tv(L.t("还没有发现设备。\n请在手机或电脑上打开 LocalSend，并确认连的是同一个 Wi-Fi。",
                "Устройства пока не найдены.\nОткройте LocalSend на телефоне или компьютере и проверьте, что он в той же сети Wi-Fi.",
                "No devices found yet.\nOpen LocalSend on your phone or computer and make sure it is on the same Wi-Fi."), 12, T.text2);
            t.setLineSpacing(T.px(4), 1f);
            t.setPadding(T.px(4), T.px(14), T.px(4), 0);
            devList.addView(t);
            return;
        }
        for (final LsCore.Device d : ds) {
            LinearLayout row = new LinearLayout(app);
            row.setGravity(Gravity.CENTER_VERTICAL);
            row.setPadding(T.px(8), T.px(8), T.px(10), T.px(8));
            row.setBackground(T.state(T.px(12)));
            row.setFocusable(true);
            TextView av = tv(d.alias.isEmpty() ? "?" : d.alias.substring(0, d.alias.offsetByCodePoints(0, 1)).toUpperCase(), 15, T.onAccent);
            av.setGravity(Gravity.CENTER);
            av.setBackground(T.round(T.accent, T.px(18)));
            LinearLayout.LayoutParams al = new LinearLayout.LayoutParams(T.px(36), T.px(36));
            al.rightMargin = T.px(12);
            row.addView(av, al);
            LinearLayout col = new LinearLayout(app);
            col.setOrientation(LinearLayout.VERTICAL);
            TextView n = tv(d.alias, 14, T.text);
            n.setSingleLine(true);
            n.setEllipsize(TextUtils.TruncateAt.END);
            col.addView(n);
            String kind = "mobile".equals(d.type) ? L.t("手机", "Телефон", "Phone") : "web".equals(d.type) ? L.t("网页", "Браузер", "Browser") : L.t("电脑", "Компьютер", "Computer");
            TextView s = tv(kind + (d.model == null || d.model.isEmpty() || "null".equals(d.model) ? "" : " · " + d.model) + " · " + d.ip, 11, T.text2);
            s.setPadding(0, T.px(4), 0, 0);
            s.setSingleLine(true);
            col.addView(s);
            row.addView(col, new LinearLayout.LayoutParams(0, -2, 1));
            row.setOnClickListener(new View.OnClickListener() { public void onClick(View v) { try { chosen(d); } catch (Throwable e) { Log.w(TAG, "device: " + e, e); } } });
            devList.addView(row, new LinearLayout.LayoutParams(-1, -2));
        }
    }
    /** 点了一台设备：有选好的文件就发文件；否则有文字就发文字；都没有就先去选文件，选完直接发给它。 */
    static void chosen(LsCore.Device d) {
        String text = textBox == null ? "" : textBox.getText().toString();
        if (!picked.isEmpty()) sendTo(d);
        else if (!text.trim().isEmpty()) sendText(d, text);
        else { pendingTarget = d; pick(); }
    }
    static void pick() {
        closePanel();
        try { app.startActivity(new Intent(app, SendPick.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)); }
        catch (Throwable e) { pendingTarget = null; toast(L.t("没有可用的文件选择器", "Нет приложения для выбора файлов", "No file picker available")); }
    }
}
