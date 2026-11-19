package local.cloud.launcher;

import android.bluetooth.BluetoothAdapter;
import android.content.*;
import android.graphics.*;
import android.graphics.drawable.*;
import android.media.AudioManager;
import android.net.*;
import android.net.wifi.SupplicantState;
import android.net.wifi.WifiInfo;
import android.net.wifi.WifiManager;
import android.os.*;
import android.util.DisplayMetrics;
import android.util.Log;
import android.util.TypedValue;
import android.view.*;
import android.widget.*;
import java.text.SimpleDateFormat;
import java.util.Date;

/**
 * 底栏右侧的系统托盘：通知数、Wi-Fi、音量、电池、时钟。
 * 状态全部来自广播（不轮询），并以静态字段的形式供快捷面板读取。点击打开 QuickPanel。
 */
final class Tray {
    static final String TAG = "Tray";
    static Tray it;

    // ---- 当前状态（主线程读写） ----
    static int bat = -1;
    static boolean batPresent, charging;
    static boolean wifiOn, wifiConn, wired, online;
    static int bars;
    static String ssid = "";
    static int vol, volMax = 15;
    static boolean muted;

    final Context c;
    final Shelf shelf;
    LinearLayout box;
    TextView clock, batText, badge;
    View batBox;
    ImageView wifiView, volView, batView;
    Icon wifiIc, volIc, batIc;
    boolean listening;

    /** 由 Shelf.attach() 调用；主题切换重建底栏时会再次进来。 */
    static void attach(Shelf s, FrameLayout bar) {
        Tray old = it;
        if (old != null) old.detach();
        try { QuickPanel.hide(); } catch (Throwable ignored) {}
        ensureTheme(s.c);
        Tray t = new Tray(s);
        it = t;
        t.build(bar);
        t.listen();
        t.readAll();
        t.paint();
        try { Notif.init(s.c); } catch (Throwable e) { Log.w(TAG, "notif: " + e); }
        try { StatusBar.sync(s.c); } catch (Throwable e) { Log.w(TAG, "statusbar: " + e); }
    }

    Tray(Shelf s) { shelf = s; c = s.c; }

    /** 进程不是从桌面 Activity 起来的时候（比如系统只拉起了通知监听服务），主题还没初始化。 */
    static void ensureTheme(Context c) {
        if (T.text != 0) return;
        try { L.init(c); } catch (Throwable ignored) {}
        try {
            DisplayMetrics dm = new DisplayMetrics();
            ((WindowManager) c.getSystemService(Context.WINDOW_SERVICE)).getDefaultDisplay().getRealMetrics(dm);
            T.u = Math.max(dm.widthPixels, dm.heightPixels) / 1280f;
        } catch (Throwable ignored) {}
        boolean dark = true;
        try { dark = Store.ctx(c).getSharedPreferences("desk", 0).getBoolean("dark", true); } catch (Throwable ignored) {}
        T.apply(dark);
    }

    static int shelfHeight() { return Shelf.it != null ? Shelf.it.height() : T.px(52); }

    static TextView text(Context c, String s, float size, int color) {
        TextView t = new TextView(c);
        t.setText(s); t.setTextColor(color);
        t.setTextSize(TypedValue.COMPLEX_UNIT_PX, T.px(size));
        t.setIncludeFontPadding(false);
        return t;
    }

    // ================= 构建 =================
    void build(FrameLayout bar) {
        box = new LinearLayout(c);
        box.setGravity(Gravity.CENTER_VERTICAL);
        box.setBackground(new LayerDrawable(new Drawable[]{T.round(T.chip, T.px(17)), T.state(T.px(17))}));
        box.setPadding(T.px(12), 0, T.px(14), 0);
        box.setContentDescription(L.t("状态与快捷设置", "Состояние и быстрые настройки", "Status and quick settings"));
        box.setOnClickListener(v -> QuickPanel.toggle(c));
        box.setOnContextClickListener(v -> { QuickPanel.toggle(c); return true; });
        // 右边留出返回键的位置：10 边距 + 36 按钮 + 8 间隔
        FrameLayout.LayoutParams tl = new FrameLayout.LayoutParams(-2, T.px(34), Gravity.RIGHT | Gravity.CENTER_VERTICAL);
        tl.rightMargin = T.px(54);
        bar.addView(box, tl);

        badge = text(c, "", 11, T.onAccent);
        badge.setGravity(Gravity.CENTER);
        badge.setBackground(T.round(T.accent, T.px(9)));
        badge.setPadding(T.px(5), 0, T.px(5), 0);
        badge.setMinWidth(T.px(18));
        badge.setVisibility(View.GONE);
        LinearLayout.LayoutParams bl = new LinearLayout.LayoutParams(-2, T.px(18));
        bl.rightMargin = T.px(9);
        box.addView(badge, bl);

        wifiIc = new Icon(Icon.WIFI, T.text);
        wifiView = icon(wifiIc, 16, 16, 9);
        volIc = new Icon(Icon.VOLUME, T.text);
        volView = icon(volIc, 17, 16, 9);

        LinearLayout bb = new LinearLayout(c);
        bb.setGravity(Gravity.CENTER_VERTICAL);
        batIc = new Icon(Icon.BATTERY, T.text);
        batView = new ImageView(c);
        batView.setImageDrawable(batIc);
        bb.addView(batView, new LinearLayout.LayoutParams(T.px(21), T.px(15)));
        batText = text(c, "", 12, T.text);
        batText.setPadding(T.px(4), 0, T.px(10), 0);
        bb.addView(batText);
        bb.setVisibility(View.GONE);
        batBox = bb;
        box.addView(bb);

        clock = text(c, "", 13, T.text);
        box.addView(clock);
    }
    ImageView icon(Icon ic, int w, int h, int gap) {
        ImageView v = new ImageView(c);
        v.setImageDrawable(ic);
        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(T.px(w), T.px(h));
        lp.rightMargin = T.px(gap);
        box.addView(v, lp);
        return v;
    }

    /** 屏幕坐标是否落在托盘上（快捷面板据此区分「点托盘」和「点外面」）。 */
    static boolean hit(float x, float y) {
        Tray t = it;
        if (t == null || t.box == null || !t.box.isAttachedToWindow()) return false;
        int[] p = new int[2];
        t.box.getLocationOnScreen(p);
        return x >= p[0] && x <= p[0] + t.box.getWidth() && y >= p[1] && y <= p[1] + t.box.getHeight();
    }

    // ================= 读取状态 =================
    void readAll() {
        try { readBattery(c.registerReceiver(null, new IntentFilter(Intent.ACTION_BATTERY_CHANGED))); } catch (Throwable e) { Log.w(TAG, "battery: " + e); }
        readNet(c);
        readVol(c);
    }
    static void readBattery(Intent i) {
        if (i == null) return;
        batPresent = i.getBooleanExtra("present", false);
        int lv = i.getIntExtra("level", -1), sc = i.getIntExtra("scale", 100);
        bat = lv >= 0 && sc > 0 ? lv * 100 / sc : -1;
        charging = i.getIntExtra("plugged", 0) != 0;
    }
    static void readNet(Context c) {
        try {
            ConnectivityManager cm = (ConnectivityManager) c.getSystemService(Context.CONNECTIVITY_SERVICE);
            Network n = cm.getActiveNetwork();
            NetworkCapabilities nc = n == null ? null : cm.getNetworkCapabilities(n);
            online = nc != null;
            wired = nc != null && nc.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET);
        } catch (Throwable e) { online = false; wired = false; }
        boolean on = false, conn = false; int b = 0; String name = "";
        try {
            WifiManager w = (WifiManager) c.getSystemService(Context.WIFI_SERVICE);
            on = w != null && w.isWifiEnabled();
            WifiInfo wi = on ? w.getConnectionInfo() : null;
            if (wi != null && wi.getSupplicantState() == SupplicantState.COMPLETED && wi.getNetworkId() != -1) {
                conn = true;
                b = WifiManager.calculateSignalLevel(wi.getRssi(), 5);
                name = unquote(wi.getSSID());
            }
        } catch (Throwable e) { Log.w(TAG, "wifi: " + e); }
        wifiOn = on; wifiConn = conn; bars = b; ssid = name;
    }
    static String unquote(String s) {
        if (s == null || s.contains("unknown ssid")) return "";
        return s.length() >= 2 && s.startsWith("\"") && s.endsWith("\"") ? s.substring(1, s.length() - 1) : s;
    }
    static void readVol(Context c) {
        try {
            AudioManager am = (AudioManager) c.getSystemService(Context.AUDIO_SERVICE);
            vol = am.getStreamVolume(AudioManager.STREAM_MUSIC);
            volMax = Math.max(1, am.getStreamMaxVolume(AudioManager.STREAM_MUSIC));
            muted = am.isStreamMute(AudioManager.STREAM_MUSIC);
        } catch (Throwable e) { Log.w(TAG, "volume: " + e); }
    }

    static String netLine() {
        if (wired) return L.t("有线网络已连接", "Проводная сеть подключена", "Wired network connected");
        if (wifiConn) return ssid.isEmpty() ? L.t("Wi-Fi 已连接", "Wi-Fi подключён", "Wi-Fi connected") : "Wi-Fi · " + ssid;
        if (!wifiOn) return L.t("Wi-Fi 已关闭", "Wi-Fi выключен", "Wi-Fi is off");
        return L.t("未连接网络", "Нет сети", "Not connected");
    }
    static String batLine() {
        if (!batPresent || bat < 0) return "";
        return L.t("电量 ", "Батарея ", "Battery ") + bat + "%" + (charging ? L.t(" · 充电中", " · заряжается", " · charging") : "");
    }
    static String dateLine() {
        return new SimpleDateFormat(L.t("M月d日 EEEE", "d MMMM, EEEE", "EEE, MMM d"), L.loc).format(new Date());
    }

    // ================= 绘制 =================
    void paint() {
        if (box == null) return;
        try {
            clock.setText(android.text.format.DateFormat.format("HH:mm", new Date()));
            wifiIc.off = !wifiOn && !wired;
            wifiIc.dim = !wifiConn && !wired;
            wifiIc.level = wired ? 1f : wifiConn ? (bars + 1) / 5f : 0f;
            wifiView.invalidate();
            volIc.off = muted || vol == 0;
            volIc.level = vol / (float) volMax;
            volView.invalidate();
            boolean showBat = batPresent && bat >= 0;
            batBox.setVisibility(showBat ? View.VISIBLE : View.GONE);
            if (showBat) {
                batIc.level = bat / 100f;
                batIc.alt = charging;
                batView.invalidate();
                batText.setText(bat + "%");
            }
            int n = Notif.count();
            badge.setVisibility(n > 0 ? View.VISIBLE : View.GONE);
            if (n > 0) badge.setText(n > 99 ? "99+" : String.valueOf(n));
            if (Build.VERSION.SDK_INT >= 26) {
                String tip = dateLine() + "\n" + netLine();
                String b = batLine();
                if (!b.isEmpty()) tip += "\n" + b;
                if (n > 0) tip += "\n" + L.t("通知 ", "Уведомления: ", "Notifications: ") + n;
                box.setTooltipText(tip);
            }
        } catch (Throwable e) { Log.w(TAG, "paint: " + e); }
    }

    // ================= 广播 =================
    final BroadcastReceiver rx = new BroadcastReceiver() {
        public void onReceive(Context ctx, Intent i) {
            try {
                String a = String.valueOf(i.getAction());
                if (Intent.ACTION_BATTERY_CHANGED.equals(a)) {
                    int ob = bat; boolean oc = charging, op = batPresent;
                    readBattery(i);
                    if (ob == bat && oc == charging && op == batPresent) return; // 电压、温度之类的变化不用重画
                } else if (a.startsWith("android.media.")) readVol(c);
                else if (a.startsWith("android.net.")) readNet(c);
                else if (a.equals("android.intent.action.OVERLAY_CHANGED")) StatusBar.sync(c);
                paint();
                QuickPanel.onState(a);
            } catch (Throwable e) { Log.w(TAG, "rx: " + e); }
        }
    };
    void listen() {
        try {
            IntentFilter f = new IntentFilter();
            f.addAction(Intent.ACTION_TIME_TICK); f.addAction(Intent.ACTION_TIME_CHANGED); f.addAction(Intent.ACTION_TIMEZONE_CHANGED);
            f.addAction(Intent.ACTION_BATTERY_CHANGED);
            f.addAction(ConnectivityManager.CONNECTIVITY_ACTION);
            f.addAction(WifiManager.WIFI_STATE_CHANGED_ACTION);
            f.addAction(WifiManager.NETWORK_STATE_CHANGED_ACTION);
            f.addAction(WifiManager.RSSI_CHANGED_ACTION);
            f.addAction(WifiManager.SCAN_RESULTS_AVAILABLE_ACTION);
            f.addAction("android.media.VOLUME_CHANGED_ACTION");
            f.addAction("android.media.STREAM_MUTE_CHANGED_ACTION");
            f.addAction(BluetoothAdapter.ACTION_STATE_CHANGED);
            c.registerReceiver(rx, f);
            IntentFilter o = new IntentFilter("android.intent.action.OVERLAY_CHANGED");
            o.addDataScheme("package");
            c.registerReceiver(rx, o);
            listening = true;
        } catch (Throwable e) { Log.w(TAG, "listen: " + e); }
    }
    void detach() {
        if (listening) try { c.unregisterReceiver(rx); } catch (Throwable ignored) {}
        listening = false;
        box = null;
        if (it == this) it = null;
    }

    // ================= 图形 =================
    /** 托盘和面板用到的小图形，全部矢量手绘。 */
    static final class Icon extends Drawable {
        static final int WIFI = 0, VOLUME = 1, BATTERY = 2, CLOSE = 3, LOCK = 4, POWER = 5, RESTART = 6, SLEEP = 7, GEAR = 8;
        final int type;
        final Paint p = new Paint(Paint.ANTI_ALIAS_FLAG);
        int color;
        float level = 1f;
        boolean off, dim, alt;

        Icon(int type, int color) { this.type = type; this.color = color; p.setStrokeCap(Paint.Cap.ROUND); p.setStrokeJoin(Paint.Join.ROUND); }

        void stroke(float w) { p.setStyle(Paint.Style.STROKE); p.setStrokeWidth(w); }
        void fill() { p.setStyle(Paint.Style.FILL); }

        public void draw(Canvas cv) {
            Rect b = getBounds();
            float w = b.width(), h = b.height(), x = b.left, y = b.top, cx = b.exactCenterX(), cy = b.exactCenterY();
            float s = Math.min(w, h), sw = Math.max(1.5f, s * 0.10f);
            p.setColor(color);
            p.setAlpha(255);
            switch (type) {
                case WIFI: {
                    float r = w * 0.72f, bx = x + w / 2, by = y + h * 0.88f;
                    fill();
                    p.setAlpha(70);
                    cv.drawArc(new RectF(bx - r, by - r, bx + r, by + r), -135, 90, true, p);
                    if (!off && !dim) {
                        float k = r * Math.max(0.3f, Math.min(1f, level));
                        p.setAlpha(255);
                        cv.drawArc(new RectF(bx - k, by - k, bx + k, by + k), -135, 90, true, p);
                    }
                    if (off) { p.setAlpha(255); stroke(sw); cv.drawLine(x + w * 0.18f, y + h * 0.12f, x + w * 0.82f, y + h * 0.90f, p); }
                    break;
                }
                case VOLUME: {
                    fill();
                    Path sp = new Path();
                    sp.moveTo(x + w * 0.06f, cy - h * 0.14f); sp.lineTo(x + w * 0.22f, cy - h * 0.14f); sp.lineTo(x + w * 0.44f, cy - h * 0.36f);
                    sp.lineTo(x + w * 0.44f, cy + h * 0.36f); sp.lineTo(x + w * 0.22f, cy + h * 0.14f); sp.lineTo(x + w * 0.06f, cy + h * 0.14f);
                    sp.close();
                    cv.drawPath(sp, p);
                    stroke(sw);
                    if (off) {
                        cv.drawLine(x + w * 0.62f, cy - h * 0.16f, x + w * 0.92f, cy + h * 0.16f, p);
                        cv.drawLine(x + w * 0.92f, cy - h * 0.16f, x + w * 0.62f, cy + h * 0.16f, p);
                    } else {
                        float ax = x + w * 0.44f, r1 = w * 0.20f, r2 = w * 0.40f;
                        cv.drawArc(new RectF(ax - r1, cy - r1, ax + r1, cy + r1), -45, 90, false, p);
                        if (level < 0.5f) p.setAlpha(70);
                        cv.drawArc(new RectF(ax - r2, cy - r2, ax + r2, cy + r2), -45, 90, false, p);
                    }
                    break;
                }
                case BATTERY: {
                    float bw = Math.max(1f, h * 0.09f);
                    stroke(bw);
                    RectF o = new RectF(x + bw, y + h * 0.20f, x + w * 0.88f, y + h * 0.80f);
                    cv.drawRoundRect(o, bw * 1.5f, bw * 1.5f, p);
                    fill();
                    cv.drawRect(x + w * 0.90f, y + h * 0.38f, x + w, y + h * 0.62f, p);
                    if (alt) p.setColor(0xFF5BD08D); else if (level <= 0.15f) p.setColor(0xFFF0605A);
                    float in = bw * 1.7f;
                    cv.drawRect(o.left + in, o.top + in, o.left + in + (o.width() - 2 * in) * Math.max(0.06f, Math.min(1f, level)), o.bottom - in, p);
                    break;
                }
                case CLOSE: {
                    stroke(sw);
                    float r = s * 0.20f;
                    cv.drawLine(cx - r, cy - r, cx + r, cy + r, p);
                    cv.drawLine(cx + r, cy - r, cx - r, cy + r, p);
                    break;
                }
                case LOCK: {
                    stroke(sw * 0.9f);
                    cv.drawArc(new RectF(cx - s * 0.17f, cy - s * 0.38f, cx + s * 0.17f, cy + s * 0.10f), 180, 180, false, p);
                    fill();
                    cv.drawRoundRect(new RectF(cx - s * 0.27f, cy - s * 0.08f, cx + s * 0.27f, cy + s * 0.36f), s * 0.06f, s * 0.06f, p);
                    break;
                }
                case POWER: {
                    stroke(sw);
                    float r = s * 0.27f;
                    cv.drawArc(new RectF(cx - r, cy - r + s * 0.03f, cx + r, cy + r + s * 0.03f), -55, 290, false, p);
                    cv.drawLine(cx, cy - s * 0.33f, cx, cy - s * 0.02f, p);
                    break;
                }
                case RESTART: {
                    // 顺时针的环形箭头：缺口在右上，弧的末端是一个实心箭头
                    stroke(sw);
                    float r = s * 0.25f, end = -35f;
                    cv.drawArc(new RectF(cx - r, cy - r, cx + r, cy + r), end + 75f, 285f - 12f, false, p);
                    double a = Math.toRadians(end);
                    float nx = (float) Math.cos(a), ny = (float) Math.sin(a), tx = -ny, ty = nx; // 径向、切向（顺时针）
                    float ex = cx + r * nx, ey = cy + r * ny, ah = s * 0.19f, aw = s * 0.15f;
                    Path ar = new Path();
                    ar.moveTo(ex + tx * ah * 0.55f, ey + ty * ah * 0.55f);
                    ar.lineTo(ex - tx * ah * 0.45f + nx * aw, ey - ty * ah * 0.45f + ny * aw);
                    ar.lineTo(ex - tx * ah * 0.45f - nx * aw, ey - ty * ah * 0.45f - ny * aw);
                    ar.close();
                    p.setStyle(Paint.Style.FILL_AND_STROKE); p.setStrokeWidth(sw * 0.35f);
                    cv.drawPath(ar, p);
                    break;
                }
                case SLEEP: {
                    fill();
                    Path m = new Path(), cut = new Path();
                    m.addCircle(cx - s * 0.03f, cy + s * 0.03f, s * 0.30f, Path.Direction.CW);
                    cut.addCircle(cx + s * 0.12f, cy - s * 0.10f, s * 0.26f, Path.Direction.CW);
                    m.op(cut, Path.Op.DIFFERENCE);
                    cv.drawPath(m, p);
                    break;
                }
                default: { // GEAR
                    stroke(sw);
                    cv.drawCircle(cx, cy, s * 0.11f, p);
                    cv.drawCircle(cx, cy, s * 0.25f, p);
                    stroke(sw * 1.5f);
                    for (int i = 0; i < 8; i++) {
                        double a = Math.PI * i / 4;
                        float ca = (float) Math.cos(a), sa = (float) Math.sin(a);
                        cv.drawLine(cx + ca * s * 0.30f, cy + sa * s * 0.30f, cx + ca * s * 0.36f, cy + sa * s * 0.36f, p);
                    }
                }
            }
        }
        public void setAlpha(int a) {}
        public void setColorFilter(ColorFilter f) {}
        public int getOpacity() { return PixelFormat.TRANSLUCENT; }
    }
}
