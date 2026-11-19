package local.cloud.launcher;

import android.bluetooth.BluetoothAdapter;
import android.content.*;
import android.content.res.ColorStateList;
import android.graphics.PixelFormat;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.LayerDrawable;
import android.media.AudioManager;
import android.net.Uri;
import android.os.*;
import android.provider.Settings;
import android.text.InputType;
import android.text.TextUtils;
import android.util.Log;
import android.view.*;
import android.view.animation.DecelerateInterpolator;
import android.view.inputmethod.EditorInfo;
import android.widget.*;
import java.util.List;

/**
 * 快捷面板：独立的系统窗口（与底栏同一种窗口类型），在任何应用上面都能打开。
 * 右下角贴着底栏；点托盘开关，点面板外面或按 Esc 关闭。
 * 内容：电源按钮、Wi-Fi（开关 + 网络列表 + 直连）、蓝牙、主题、状态栏开关、传文件、音量、亮度、通知列表。
 * 外观和各项逻辑沿用 Desk 里原来的快捷设置；底栏系统窗口可用时 Desk 会把「打开快捷设置」转到这里。
 */
final class QuickPanel {
    static final String TAG = "QuickPanel";
    static QuickPanel it;
    static long closedAt;

    final Context c, v;
    final WindowManager wm;
    final Handler ui = new Handler(Looper.getMainLooper());
    Root root;
    LinearLayout panel, wifiBox, notifBox, confirm, wifiPill, btPill, barPill, sharePill;
    TextView date, foot, chan, wifiSub, btSub, barSub, confirmText, brightHint;
    SeekBar volume, bright;
    boolean volTracking, barHidden;
    String pwFor, connecting, wifiNote;
    Runnable pending;

    // ================= 对外 =================
    static boolean showing() { return it != null; }

    static void toggle(Context ctx) {
        try {
            if (it != null) hide();
            // 点托盘时「点到面板外面」会先把面板关掉：紧接着的这次点击不应再把它打开
            else if (SystemClock.uptimeMillis() - closedAt > 400) show(ctx);
        } catch (Throwable e) { Log.w(TAG, "toggle: " + e); forget(); }
    }
    static void show(Context ctx) {
        if (it != null) return;
        Context app = ctx.getApplicationContext();
        Tray.ensureTheme(app);
        QuickPanel q = new QuickPanel(app);
        try {
            q.build();
            q.attach();
            it = q;
        } catch (Throwable e) {
            Log.w(TAG, "show: " + e);
            try { q.wm.removeViewImmediate(q.root); } catch (Throwable ignored) {}
            return;
        }
        try { Notif.hidePopups(); } catch (Throwable ignored) {}
        try {
            Tray.readNet(app); Tray.readVol(app);
            q.barHidden = StatusBar.hidden(app);
            q.refresh(); q.renderWifi(); q.renderNotifs();
            if (Tray.wifiOn) Wifi.startScan(app);
            q.ui.postDelayed(q.tick, 5000);
        } catch (Throwable e) { Log.w(TAG, "fill: " + e); }
    }
    static void hide() {
        QuickPanel q = it;
        it = null;
        if (q == null) return;
        closedAt = SystemClock.uptimeMillis();
        q.ui.removeCallbacksAndMessages(null);
        try { q.wm.removeViewImmediate(q.root); } catch (Throwable e) { Log.w(TAG, "hide: " + e); }
    }
    static void forget() { try { hide(); } catch (Throwable ignored) {} it = null; }

    /** 托盘收到状态广播后转过来。 */
    static void onState(String action) {
        QuickPanel q = it;
        if (q == null) return;
        try {
            if (q.connecting != null && Tray.wifiConn && q.connecting.equals(Tray.ssid)) { q.connecting = null; q.wifiNote = null; }
            if ("android.intent.action.OVERLAY_CHANGED".equals(action)) q.barHidden = StatusBar.hidden(q.c);
            q.refresh();
            boolean net = action != null && action.startsWith("android.net.") && !action.endsWith("RSSI_CHANGED");
            if (net && q.pwFor == null) q.renderWifi(); // 正在输密码时不重建列表
        } catch (Throwable e) { Log.w(TAG, "state: " + e); }
    }
    static void onNotifs() {
        QuickPanel q = it;
        if (q != null) try { q.renderNotifs(); } catch (Throwable e) { Log.w(TAG, "notifs: " + e); }
    }

    QuickPanel(Context app) {
        c = app;
        v = new ContextThemeWrapper(app, T.dark ? android.R.style.Theme_DeviceDefault : android.R.style.Theme_DeviceDefault_Light);
        wm = (WindowManager) app.getSystemService(Context.WINDOW_SERVICE);
    }

    // ================= 窗口 =================
    final class Root extends FrameLayout {
        Root(Context x) { super(x); }
        @Override public boolean dispatchKeyEvent(KeyEvent e) {
            int k = e.getKeyCode();
            if (k == KeyEvent.KEYCODE_ESCAPE || k == KeyEvent.KEYCODE_BACK) {
                if (e.getAction() == KeyEvent.ACTION_UP) ui.post(QuickPanel::hide);
                return true;
            }
            return super.dispatchKeyEvent(e);
        }
        @Override public boolean dispatchTouchEvent(MotionEvent e) {
            if (e.getActionMasked() == MotionEvent.ACTION_OUTSIDE) {
                // 点在托盘上的交给托盘自己的点击去关，其余一律关闭
                if (!Tray.hit(e.getRawX(), e.getRawY())) ui.post(QuickPanel::hide);
                return true;
            }
            return super.dispatchTouchEvent(e);
        }
        @Override public boolean onTouchEvent(MotionEvent e) { // 面板四周留给阴影的透明边
            if (e.getActionMasked() == MotionEvent.ACTION_DOWN) ui.post(QuickPanel::hide);
            return true;
        }
        @Override protected void onMeasure(int w, int h) {
            int max = screenH() - Tray.shelfHeight() - T.px(12);
            super.onMeasure(w, MeasureSpec.makeMeasureSpec(Math.max(T.px(200), max), MeasureSpec.AT_MOST));
        }
    }
    int screenH() {
        android.util.DisplayMetrics dm = new android.util.DisplayMetrics();
        wm.getDefaultDisplay().getRealMetrics(dm);
        return dm.heightPixels;
    }
    void attach() {
        WindowManager.LayoutParams lp = new WindowManager.LayoutParams(T.px(384), -2, 2024 /* TYPE_NAVIGATION_BAR_PANEL，与底栏同层 */,
            WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL | WindowManager.LayoutParams.FLAG_WATCH_OUTSIDE_TOUCH
                | WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN,
            PixelFormat.TRANSLUCENT);
        lp.gravity = Gravity.BOTTOM | Gravity.RIGHT;
        lp.y = Tray.shelfHeight();
        lp.softInputMode = WindowManager.LayoutParams.SOFT_INPUT_ADJUST_NOTHING;
        lp.setTitle("OpenDesktopQuickPanel");
        if (Build.VERSION.SDK_INT >= 30) lp.setFitInsetsTypes(0);
        wm.addView(root, lp);
        root.requestFocus();
        View p = root.getChildAt(0);
        p.setAlpha(0f);
        p.setTranslationY(T.px(14));
        p.animate().alpha(1f).translationY(0).setDuration(120).setInterpolator(new DecelerateInterpolator()).start();
    }

    // ================= 小部件（沿用 Desk 的样式） =================
    TextView text(String s, float size, int color) { return Tray.text(v, s, size, color); }
    void toast(String s) { try { Toast.makeText(c, s, Toast.LENGTH_LONG).show(); } catch (Throwable ignored) {} }

    LinearLayout pill(String title, String sub) {
        LinearLayout p = new LinearLayout(v);
        p.setOrientation(LinearLayout.VERTICAL);
        p.setGravity(Gravity.CENTER_VERTICAL);
        p.setPadding(T.px(18), 0, T.px(12), 0);
        TextView t = text(title, 13, T.text);
        t.setSingleLine(true); t.setEllipsize(TextUtils.TruncateAt.END);
        p.addView(t);
        TextView s = text(sub, 10.5f, T.text2);
        s.setSingleLine(true); s.setEllipsize(TextUtils.TruncateAt.END);
        s.setPadding(0, T.px(3), 0, 0);
        if (sub == null) s.setVisibility(View.GONE);
        p.addView(s);
        setPill(p, false);
        return p;
    }
    void setPill(LinearLayout p, boolean on) {
        p.setBackground(new LayerDrawable(new Drawable[]{T.round(on ? T.accent : T.chip, T.px(24)), T.state(T.px(24))}));
        ((TextView) p.getChildAt(0)).setTextColor(on ? T.onAccent : T.text);
        ((TextView) p.getChildAt(1)).setTextColor(on ? T.onAccent : T.text2);
    }
    void pillRow(View l, View r) {
        LinearLayout row = new LinearLayout(v);
        LinearLayout.LayoutParams x = new LinearLayout.LayoutParams(0, T.px(48), 1), y = new LinearLayout.LayoutParams(0, T.px(48), 1);
        x.rightMargin = T.px(4); y.leftMargin = T.px(4);
        row.addView(l, x);
        row.addView(r != null ? r : new View(v), y);
        LinearLayout.LayoutParams rl = new LinearLayout.LayoutParams(-1, -2);
        rl.bottomMargin = T.px(8);
        panel.addView(row, rl);
    }
    SeekBar slider(String name, TextView hint) {
        LinearLayout row = new LinearLayout(v);
        row.setGravity(Gravity.CENTER_VERTICAL);
        row.addView(text(name, 12, T.text2), new LinearLayout.LayoutParams(T.px(62), -2));
        SeekBar s = new SeekBar(v);
        ColorStateList acc = ColorStateList.valueOf(T.accent);
        s.setProgressTintList(acc);
        s.setThumbTintList(acc);
        s.setProgressBackgroundTintList(ColorStateList.valueOf(T.text2));
        row.addView(s, new LinearLayout.LayoutParams(0, -2, 1));
        if (hint != null) row.addView(hint);
        panel.addView(row, new LinearLayout.LayoutParams(-1, T.px(40)));
        return s;
    }
    ImageView iconButton(int type, String label, View.OnClickListener l) {
        ImageView b = new ImageView(v);
        b.setImageDrawable(new Tray.Icon(type, T.text));
        b.setBackground(new LayerDrawable(new Drawable[]{T.round(T.chip, T.px(17)), T.state(T.px(17))}));
        b.setContentDescription(label);
        if (Build.VERSION.SDK_INT >= 26) b.setTooltipText(label);
        b.setOnClickListener(l);
        return b;
    }
    TextView link(String s, View.OnClickListener l) {
        TextView t = text(s, 12, T.accent);
        t.setPadding(T.px(10), T.px(7), T.px(10), T.px(7));
        t.setBackground(T.state(T.px(13)));
        t.setOnClickListener(l);
        return t;
    }
    void line() {
        View line = new View(v);
        line.setBackgroundColor(T.chip);
        LinearLayout.LayoutParams ll = new LinearLayout.LayoutParams(-1, Math.max(1, T.px(1)));
        ll.topMargin = T.px(8); ll.bottomMargin = T.px(6);
        panel.addView(line, ll);
    }

    // ================= 构建 =================
    void build() {
        root = new Root(v);
        root.setPadding(T.px(12), T.px(12), T.px(12), T.px(8));
        root.setClipToPadding(false);
        root.setFocusableInTouchMode(true);

        ScrollView sc = new ScrollView(v);
        sc.setBackground(T.round(T.panel, T.px(18)));
        sc.setClipToOutline(true);
        sc.setElevation(T.px(10));
        sc.setVerticalScrollBarEnabled(false);
        sc.setOverScrollMode(View.OVER_SCROLL_NEVER);
        root.addView(sc, new FrameLayout.LayoutParams(-1, -2));

        panel = new LinearLayout(v);
        panel.setOrientation(LinearLayout.VERTICAL);
        panel.setPadding(T.px(14), T.px(12), T.px(14), T.px(12));
        panel.setClickable(true);
        sc.addView(panel, new FrameLayout.LayoutParams(-1, -2));

        // ---- 顶行：日期 + 设置 / 睡眠 / 重启 / 关机 ----
        LinearLayout head = new LinearLayout(v);
        head.setGravity(Gravity.CENTER_VERTICAL);
        date = text("", 13, T.text);
        date.setSingleLine(true);
        head.addView(date, new LinearLayout.LayoutParams(0, -2, 1));
        View[] bs = {
            iconButton(Tray.Icon.GEAR, L.t("系统设置", "Настройки", "System settings"), x -> { hide(); start(Main.systemSettings(c)); }),
            iconButton(Tray.Icon.SLEEP, L.t("睡眠", "Сон", "Sleep"), x -> { hide(); power(0); }),
            iconButton(Tray.Icon.RESTART, L.t("重启", "Перезагрузка", "Restart"), x -> ask(L.t("确定要重启吗？", "Перезагрузить устройство?", "Restart the device?"), () -> power(1))),
            iconButton(Tray.Icon.POWER, L.t("关机", "Выключение", "Power off"), x -> ask(L.t("确定要关机吗？", "Выключить устройство?", "Power off the device?"), () -> power(2))),
        };
        for (View b : bs) {
            LinearLayout.LayoutParams bl = new LinearLayout.LayoutParams(T.px(34), T.px(34));
            bl.leftMargin = T.px(6);
            head.addView(b, bl);
        }
        LinearLayout.LayoutParams hl = new LinearLayout.LayoutParams(-1, -2);
        hl.bottomMargin = T.px(10);
        panel.addView(head, hl);

        // ---- 重启 / 关机确认 ----
        confirm = new LinearLayout(v);
        confirm.setGravity(Gravity.CENTER_VERTICAL);
        confirm.setBackground(T.round(T.chip, T.px(14)));
        confirm.setPadding(T.px(14), T.px(4), T.px(4), T.px(4));
        confirmText = text("", 13, T.text);
        confirm.addView(confirmText, new LinearLayout.LayoutParams(0, -2, 1));
        confirm.addView(link(L.t("取消", "Отмена", "Cancel"), x -> { pending = null; confirm.setVisibility(View.GONE); }));
        confirm.addView(link(L.t("确定", "Да", "Confirm"), x -> { Runnable r = pending; pending = null; hide(); if (r != null) r.run(); }));
        confirm.setVisibility(View.GONE);
        LinearLayout.LayoutParams cl = new LinearLayout.LayoutParams(-1, -2);
        cl.bottomMargin = T.px(10);
        panel.addView(confirm, cl);

        // ---- 开关 ----
        wifiPill = pill("Wi-Fi", "…");
        wifiSub = (TextView) wifiPill.getChildAt(1);
        wifiPill.setOnClickListener(x -> toggleWifi());
        wifiPill.setOnLongClickListener(x -> { openWifiSettings(); return true; });
        wifiPill.setOnContextClickListener(x -> { openWifiSettings(); return true; });

        final BluetoothAdapter ba = Desk.btAdapter();
        if (ba != null) {
            btPill = pill(L.t("蓝牙", "Bluetooth", "Bluetooth"), "…");
            btSub = (TextView) btPill.getChildAt(1);
            btPill.setOnClickListener(x -> {
                try {
                    boolean ok = ba.isEnabled() ? ba.disable() : ba.enable();
                    if (!ok) throw new SecurityException();
                    btSub.setText(L.t("切换中…", "Переключение…", "Switching…")); // 状态广播到了会刷新
                } catch (Throwable e) { hide(); start(new Intent(Settings.ACTION_BLUETOOTH_SETTINGS)); }
            });
            btPill.setOnLongClickListener(x -> { hide(); start(new Intent(Settings.ACTION_BLUETOOTH_SETTINGS)); return true; });
            btPill.setOnContextClickListener(x -> { hide(); start(new Intent(Settings.ACTION_BLUETOOTH_SETTINGS)); return true; });
        }
        LinearLayout theme = pill(T.dark ? L.t("深色模式", "Тёмная тема", "Dark theme") : L.t("浅色模式", "Светлая тема", "Light theme"),
            L.t("点按切换", "Нажмите для смены", "Click to switch"));
        theme.setOnClickListener(x -> home("theme"));
        barPill = pill(L.t("系统状态栏", "Строка состояния", "System status bar"), "…");
        barSub = (TextView) barPill.getChildAt(1);
        barPill.setOnClickListener(x -> toggleStatusBar());
        sharePill = pill(L.t("传文件", "Передача файлов", "File transfer"), "LocalSend");
        sharePill.setOnClickListener(x -> { hide(); try { Send.open(c); } catch (Throwable e) { android.util.Log.w("QuickPanel", "send: " + e); } });
        LinearLayout desk = pill(L.t("桌面设置", "Рабочий стол", "Desktop settings"), L.t("壁纸、应用、权限", "Обои, приложения", "Wallpaper, apps"));
        desk.setOnClickListener(x -> home("desksettings"));

        if (btPill != null) { pillRow(wifiPill, btPill); pillRow(theme, barPill); pillRow(sharePill, desk); }
        else { pillRow(wifiPill, theme); pillRow(barPill, sharePill); pillRow(desk, null); }

        // ---- 音量 / 亮度 ----
        final AudioManager am = (AudioManager) c.getSystemService(Context.AUDIO_SERVICE);
        volume = slider(L.t("音量", "Звук", "Volume"), null);
        volume.setMax(am.getStreamMaxVolume(AudioManager.STREAM_MUSIC));
        volume.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
            public void onProgressChanged(SeekBar s, int val, boolean user) {
                if (user) try { am.setStreamVolume(AudioManager.STREAM_MUSIC, val, 0); } catch (RuntimeException ignored) {}
            }
            public void onStartTrackingTouch(SeekBar s) { volTracking = true; }
            public void onStopTrackingTouch(SeekBar s) { volTracking = false; }
        });
        brightHint = text(L.t("点按授权", "Разрешить", "Allow"), 10.5f, T.accent);
        bright = slider(L.t("亮度", "Яркость", "Brightness"), brightHint);
        bright.setMax(255);
        ((View) bright.getParent()).setOnClickListener(x -> {
            if (!Settings.System.canWrite(c)) { hide(); start(new Intent(Settings.ACTION_MANAGE_WRITE_SETTINGS, Uri.parse("package:" + c.getPackageName()))); }
        });
        bright.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
            public void onProgressChanged(SeekBar s, int val, boolean user) {
                if (!user || !Settings.System.canWrite(c)) return;
                try {
                    Settings.System.putInt(c.getContentResolver(), Settings.System.SCREEN_BRIGHTNESS_MODE, Settings.System.SCREEN_BRIGHTNESS_MODE_MANUAL);
                    Settings.System.putInt(c.getContentResolver(), Settings.System.SCREEN_BRIGHTNESS, Math.max(8, val));
                } catch (RuntimeException ignored) {}
            }
            public void onStartTrackingTouch(SeekBar s) {}
            public void onStopTrackingTouch(SeekBar s) {}
        });

        // ---- Wi-Fi 网络 ----
        wifiBox = new LinearLayout(v);
        wifiBox.setOrientation(LinearLayout.VERTICAL);
        panel.addView(wifiBox);

        // ---- 通知 ----
        line();
        notifBox = new LinearLayout(v);
        notifBox.setOrientation(LinearLayout.VERTICAL);
        panel.addView(notifBox);

        // ---- 底部：电量、控制通道 ----
        line();
        foot = text("", 12, T.text2);
        foot.setSingleLine(true);
        foot.setEllipsize(TextUtils.TruncateAt.END);
        panel.addView(foot);
        chan = text("", 10.5f, T.text2);
        chan.setPadding(0, T.px(4), 0, 0);
        panel.addView(chan);
    }

    Apps.App localSend() {
        Apps apps = Shelf.it != null ? Shelf.it.apps : Main.apps;
        if (apps != null) for (Apps.App x : apps.all) if (Desk.LOCALSEND.equals(x.pkg)) return x;
        return null;
    }

    // ================= 刷新 =================
    void refresh() {
        date.setText(Tray.dateLine());
        setPill(wifiPill, Tray.wifiOn);
        wifiSub.setText(Tray.wifiConn ? (Tray.ssid.isEmpty() ? L.t("已连接", "Подключён", "Connected") : Tray.ssid)
            : Tray.wifiOn ? L.t("已开启，未连接", "Включён, нет сети", "On, not connected")
            : Tray.wired ? L.t("已关闭 · 有线已连接", "Выключен · проводная сеть", "Off · wired connected")
            : L.t("已关闭", "Выключен", "Off"));
        if (btPill != null) {
            boolean on = false;
            try { on = Desk.btAdapter().isEnabled(); } catch (Throwable ignored) {}
            setPill(btPill, on);
            btSub.setText(on ? L.t("已开启", "Включён", "On") : L.t("已关闭", "Выключен", "Off"));
        }
        setPill(barPill, !barHidden);
        barSub.setText(barHidden ? L.t("已隐藏 · 点按恢复", "Скрыта · нажмите, чтобы вернуть", "Hidden · click to restore")
                                 : L.t("显示中 · 点按隐藏", "Показана · нажмите, чтобы скрыть", "Shown · click to hide"));
        ((TextView) sharePill.getChildAt(1)).setText(L.t("与手机、电脑互传 · 兼容 LocalSend", "Обмен с телефоном и ПК · совместимо с LocalSend", "Share with phones and PCs · works with LocalSend"));
        if (!volTracking) volume.setProgress(Tray.vol);
        boolean can = Settings.System.canWrite(c);
        bright.setEnabled(can);
        bright.setAlpha(can ? 1f : 0.4f);
        brightHint.setVisibility(can ? View.GONE : View.VISIBLE);
        try { bright.setProgress(Settings.System.getInt(c.getContentResolver(), Settings.System.SCREEN_BRIGHTNESS)); } catch (Exception ignored) {}
        String b = Tray.batLine();
        foot.setText(b.isEmpty() ? Tray.netLine() : b + " · " + Tray.netLine());
        chan.setText(channelLine());
    }
    static String channelLine() {
        String s = String.valueOf(AgentService.status);
        String st = s.contains("已连接") ? L.t("已连接 Mac ✓", "Mac подключён ✓", "Mac connected ✓")
            : s.contains("失败") ? L.t("连接 Mac 失败，重试中…", "нет связи с Mac, повтор…", "Mac unreachable, retrying…")
            : s.contains("拒绝") ? L.t("拒绝未签名指令", "отклонена неподписанная команда", "rejected an unsigned command")
            : s.contains("待命") ? L.t("待命", "ожидание", "idle") : s;
        return L.t("控制通道：", "Канал управления: ", "Control channel: ") + st;
    }
    /** 面板开着的时候每 5 秒看一眼控制通道状态（它没有广播）。 */
    final Runnable tick = new Runnable() {
        public void run() {
            if (it != QuickPanel.this) return;
            try { chan.setText(channelLine()); } catch (Throwable ignored) {}
            ui.postDelayed(this, 5000);
        }
    };

    // ================= 动作 =================
    void start(Intent i) {
        try { c.startActivity(i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)); }
        catch (RuntimeException e) { toast(L.t("系统未允许打开", "Система не разрешила открыть", "The system did not allow this")); }
    }
    /** 需要 Activity 才能做的事（换主题要重建桌面、桌面设置是个对话框）交给桌面。 */
    void home(String what) {
        hide();
        start(new Intent(c, Main.class).putExtra("panel", what));
    }
    void ask(String q, Runnable act) {
        pending = act;
        confirmText.setText(q);
        confirm.setVisibility(View.VISIBLE);
    }
    /** 0 睡眠 / 1 重启 / 2 关机。需要平台签名带来的 DEVICE_POWER、REBOOT。 */
    void power(final int what) {
        final Context app = c;
        new Thread(() -> {
            try {
                PowerManager pm = (PowerManager) app.getSystemService(Context.POWER_SERVICE);
                if (what == 0) PowerManager.class.getMethod("goToSleep", long.class).invoke(pm, SystemClock.uptimeMillis());
                else if (what == 1) pm.reboot(null);
                else PowerManager.class.getMethod("shutdown", boolean.class, String.class, boolean.class).invoke(pm, false, "userrequested", false);
            } catch (Throwable e) {
                Log.w(TAG, "power " + what + ": " + e + (e.getCause() != null ? " / " + e.getCause() : ""));
                ui.post(() -> toast(L.t("系统未允许这个操作", "Система не разрешила это действие", "The system did not allow this")));
            }
        }).start();
    }
    void toggleStatusBar() {
        final boolean hideIt = !barHidden;
        String err = StatusBar.set(c, hideIt);
        if (err != null) { toast(L.t("没能切换状态栏：", "Не удалось переключить строку состояния: ", "Could not switch the status bar: ") + err); return; }
        barHidden = hideIt;
        refresh();
        final Context app = c;
        // 系统应用新资源需要一点时间；之后以实际状态为准
        new Handler(Looper.getMainLooper()).postDelayed(() -> {
            try { StatusBar.sync(app); if (it != null) { it.barHidden = StatusBar.hidden(app); it.refresh(); } } catch (Throwable ignored) {}
        }, 1500);
    }

    // ================= Wi-Fi =================
    void toggleWifi() {
        boolean ok = false;
        try { ok = Wifi.wm(c).setWifiEnabled(!Tray.wifiOn); } catch (Throwable e) { Log.w(TAG, "wifi toggle: " + e); }
        if (ok) { wifiSub.setText(L.t("切换中…", "Переключение…", "Switching…")); return; }
        toast(L.t("这个系统不允许桌面直接开关 Wi-Fi，已打开系统 Wi-Fi 面板", "Система не позволяет переключать Wi-Fi отсюда — открыта системная панель Wi-Fi",
            "This system does not let the desktop switch Wi-Fi directly — opened the system Wi-Fi panel"));
        openWifiSettings();
    }
    void openWifiSettings() {
        hide();
        String[][] tries = {{"android.settings.panel.action.WIFI", "com.android.settings"}, {Settings.ACTION_WIFI_SETTINGS, "com.android.settings"}, {Settings.ACTION_WIFI_SETTINGS, null}};
        for (String[] t : tries) {
            try {
                Intent i = new Intent(t[0]).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                if (t[1] != null) i.setPackage(t[1]);
                c.startActivity(i);
                return;
            } catch (RuntimeException ignored) {}
        }
        toast(L.t("系统未允许打开", "Система не разрешила открыть", "The system did not allow this"));
    }

    void renderWifi() {
        wifiBox.removeAllViews();
        if (!Tray.wifiOn) { wifiBox.setVisibility(View.GONE); return; }
        wifiBox.setVisibility(View.VISIBLE);
        View sep = new View(v);
        sep.setBackgroundColor(T.chip);
        LinearLayout.LayoutParams sl = new LinearLayout.LayoutParams(-1, Math.max(1, T.px(1)));
        sl.topMargin = T.px(8); sl.bottomMargin = T.px(2);
        wifiBox.addView(sep, sl);

        LinearLayout head = new LinearLayout(v);
        head.setGravity(Gravity.CENTER_VERTICAL);
        TextView title = text(wifiNote != null ? wifiNote : L.t("Wi-Fi 网络", "Сети Wi-Fi", "Wi-Fi networks"), 11, wifiNote != null ? T.accent : T.text2);
        title.setSingleLine(true); title.setEllipsize(TextUtils.TruncateAt.END);
        head.addView(title, new LinearLayout.LayoutParams(0, -2, 1));
        head.addView(link(L.t("重新扫描", "Обновить", "Rescan"), x -> { Wifi.startScan(c); ui.postDelayed(() -> { if (pwFor == null) renderWifi(); }, 3500); }));
        head.addView(link(L.t("更多…", "Ещё…", "More…"), x -> openWifiSettings()));
        wifiBox.addView(head);

        List<Wifi.Net> nets = Wifi.list(c);
        if (nets.isEmpty()) {
            TextView t = text(L.t("正在扫描…", "Поиск сетей…", "Scanning…"), 12, T.text2);
            t.setPadding(T.px(8), T.px(8), 0, T.px(8));
            wifiBox.addView(t);
            return;
        }
        int shown = 0;
        for (final Wifi.Net n : nets) {
            if (shown++ >= 6) break;
            LinearLayout row = new LinearLayout(v);
            row.setGravity(Gravity.CENTER_VERTICAL);
            row.setPadding(T.px(8), 0, T.px(8), 0);
            row.setBackground(T.state(T.px(12)));
            Tray.Icon ic = new Tray.Icon(Tray.Icon.WIFI, n.connected ? T.accent : T.text);
            ic.level = (n.bars() + 1) / 5f;
            ImageView iv = new ImageView(v);
            iv.setImageDrawable(ic);
            LinearLayout.LayoutParams il = new LinearLayout.LayoutParams(T.px(16), T.px(16));
            il.rightMargin = T.px(10);
            row.addView(iv, il);
            TextView name = text(n.ssid, 13, n.connected ? T.accent : T.text);
            name.setSingleLine(true); name.setEllipsize(TextUtils.TruncateAt.END);
            row.addView(name, new LinearLayout.LayoutParams(0, -2, 1));
            String st = n.connected ? L.t("已连接", "Подключено", "Connected")
                : n.ssid.equals(connecting) ? L.t("连接中…", "Подключение…", "Connecting…")
                : n.netId >= 0 ? L.t("已保存", "Сохранена", "Saved") : "";
            if (!st.isEmpty()) row.addView(text(st, 10.5f, T.text2));
            if (n.secured()) {
                ImageView lk = new ImageView(v);
                lk.setImageDrawable(new Tray.Icon(Tray.Icon.LOCK, T.text2));
                LinearLayout.LayoutParams ll = new LinearLayout.LayoutParams(T.px(14), T.px(14));
                ll.leftMargin = T.px(6);
                row.addView(lk, ll);
            }
            row.setOnClickListener(x -> { try { pick(n); } catch (Throwable e) { Log.w(TAG, "pick: " + e); } });
            wifiBox.addView(row, new LinearLayout.LayoutParams(-1, T.px(36)));
            if (n.ssid.equals(pwFor)) wifiBox.addView(passwordRow(n), new LinearLayout.LayoutParams(-1, T.px(40)));
        }
    }
    View passwordRow(final Wifi.Net n) {
        LinearLayout row = new LinearLayout(v);
        row.setGravity(Gravity.CENTER_VERTICAL);
        row.setPadding(T.px(34), 0, 0, T.px(4));
        final EditText pw = new EditText(v);
        pw.setHint(L.t("输入密码，回车连接", "Пароль, затем Enter", "Password, then Enter"));
        pw.setHintTextColor(T.text2);
        pw.setTextColor(T.text);
        pw.setTextSize(android.util.TypedValue.COMPLEX_UNIT_PX, T.px(13));
        pw.setSingleLine(true);
        pw.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD);
        pw.setImeOptions(EditorInfo.IME_ACTION_GO | EditorInfo.IME_FLAG_NO_EXTRACT_UI);
        pw.setBackground(T.round(T.chip, T.px(16)));
        pw.setPadding(T.px(14), 0, T.px(14), 0);
        pw.setGravity(Gravity.CENTER_VERTICAL);
        row.addView(pw, new LinearLayout.LayoutParams(0, T.px(34), 1));
        final Runnable go = () -> {
            String s = pw.getText().toString();
            if (s.length() < 8) { pw.setError(L.t("密码至少 8 位", "Не меньше 8 символов", "At least 8 characters")); return; }
            connect(n, s);
        };
        pw.setOnEditorActionListener((t, id, ev) -> { go.run(); return true; });
        pw.setOnKeyListener((t, code, ev) -> {
            if (code != KeyEvent.KEYCODE_ENTER && code != KeyEvent.KEYCODE_NUMPAD_ENTER) return false;
            if (ev.getAction() == KeyEvent.ACTION_UP) go.run();
            return true;
        });
        row.addView(link(L.t("连接", "Подключить", "Connect"), x -> go.run()));
        ui.post(pw::requestFocus);
        return row;
    }
    void pick(Wifi.Net n) {
        if (n.connected) { openWifiSettings(); return; }
        if (n.sec == Wifi.EAP || n.sec == Wifi.WEP) {
            toast(L.t("企业认证 / WEP 网络请在系统 Wi-Fi 设置里连接", "Сети с корпоративной защитой / WEP подключаются в системных настройках Wi-Fi",
                "Enterprise / WEP networks must be joined from the system Wi-Fi settings"));
            openWifiSettings();
            return;
        }
        if (n.netId >= 0 || !n.secured()) { connect(n, null); return; }
        pwFor = n.ssid.equals(pwFor) ? null : n.ssid;
        renderWifi();
    }
    void connect(final Wifi.Net n, String pw) {
        pwFor = null;
        connecting = n.ssid;
        wifiNote = L.t("正在连接 ", "Подключение к ", "Connecting to ") + n.ssid + "…";
        renderWifi();
        final QuickPanel self = this;
        Wifi.connect(c, n, pw, (ok, unsupported) -> {
            if (unsupported) {
                toast(L.t("这个系统不允许桌面直接连接 Wi-Fi，已打开系统 Wi-Fi 面板，请在那里连接", "Система не позволяет подключаться к Wi-Fi отсюда — открыта системная панель Wi-Fi",
                    "This system does not let the desktop join Wi-Fi directly — opened the system Wi-Fi panel instead"));
                if (it == self) openWifiSettings();
            } else if (!ok && it == self) {
                connecting = null;
                wifiNote = L.t("连接失败：", "Не удалось подключиться: ", "Could not connect: ") + n.ssid;
                renderWifi();
            }
        });
        ui.postDelayed(() -> {
            if (it != self || !n.ssid.equals(connecting)) return;
            connecting = null;
            wifiNote = L.t("没能连上 ", "Нет подключения к ", "Could not join ") + n.ssid + L.t("，请检查密码", " — проверьте пароль", " — check the password");
            renderWifi();
        }, 20000);
    }

    // ================= 通知 =================
    void renderNotifs() {
        notifBox.removeAllViews();
        List<android.service.notification.StatusBarNotification> all = new java.util.ArrayList<>(Notif.items);
        LinearLayout head = new LinearLayout(v);
        head.setGravity(Gravity.CENTER_VERTICAL);
        String t = L.t("通知", "Уведомления", "Notifications") + (all.isEmpty() ? "" : " · " + all.size());
        head.addView(text(t, 11, T.text2), new LinearLayout.LayoutParams(0, -2, 1));
        if (!all.isEmpty()) head.addView(link(L.t("全部清除", "Очистить все", "Clear all"), x -> { try { Notif.clearAll(); } catch (Throwable e) { Log.w(TAG, "clear: " + e); } }));
        notifBox.addView(head, new LinearLayout.LayoutParams(-1, T.px(30)));
        if (all.isEmpty()) {
            TextView e = text(Notif.live() ? L.t("没有新通知", "Новых уведомлений нет", "No new notifications")
                : L.t("通知监听还没有接通", "Доступ к уведомлениям ещё не включён", "Notification access is not connected yet"), 12, T.text2);
            e.setPadding(T.px(8), T.px(4), 0, T.px(6));
            notifBox.addView(e);
            return;
        }
        int shown = 0;
        for (android.service.notification.StatusBarNotification s : all) {
            if (shown++ >= 6) break;
            LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(-1, -2);
            lp.bottomMargin = T.px(6);
            notifBox.addView(Notif.card(v, s, true), lp);
        }
        if (all.size() > 6) {
            TextView more = text("+" + (all.size() - 6), 11, T.text2);
            more.setPadding(T.px(8), 0, 0, T.px(4));
            notifBox.addView(more);
        }
    }
}
