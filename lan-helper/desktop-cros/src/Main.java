package local.cloud.launcher;

import android.app.Activity;
import android.content.*;
import android.graphics.drawable.ColorDrawable;
import android.media.projection.MediaProjectionManager;
import android.net.Uri;
import android.os.*;
import android.provider.Settings;
import android.util.Log;
import android.view.*;
import android.widget.*;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.util.*;

/**
 * 桌面入口。两件事永远先做：启动控制通道服务、装好崩溃兜底。
 * 新桌面一旦崩溃，下次启动自动进安全模式（极简列表），并把崩溃信息报给 Mac。
 */
public class Main extends Activity {
    static final int REQ_PROJECTION = 42;
    static Apps apps;
    SharedPreferences sp;
    Desk desk;
    boolean safe;

    public void onCreate(Bundle state) {
        super.onCreate(state);
        try { startAgent(); } catch (Throwable ignored) {}
        sp = Store.ctx(this).getSharedPreferences("desk", 0);
        try { L.init(this); } catch (Throwable ignored) {}
        guard();
        if (sp.getBoolean("safe", false) && sp.getInt("safeVer", 0) != versionCode(this)) sp.edit().putBoolean("safe", false).apply();
        if (sp.getBoolean("safe", false)) { safeMode(); return; }
        try {
            getWindow().setBackgroundDrawable(new ColorDrawable(0xFF0C1424));
            if (apps == null) apps = new Apps(this);
            rebuild();
            // 仅测试构建（通道指向本机空地址）可用：验证崩溃后能进安全模式。
            if (AgentService.BASE.contains("127.0.0.1") && getIntent().getBooleanExtra("crashtest", false))
                new Handler(Looper.getMainLooper()).postDelayed(() -> { throw new IllegalStateException("crashtest"); }, 1500);
        } catch (Throwable t) {
            record(t);
            safeMode();
        }
    }
    void rebuild() {
        boolean was = desk != null && desk.resumed;
        if (desk != null) desk.pause();
        desk = new Desk(this, apps);
        setContentView(desk.build());
        // 常驻底栏（系统窗口）。成功后隐藏 Activity 内的那条，并让壁纸铺到屏幕边缘。
        try {
            if (Shelf.it != null && Shelf.active) Shelf.it.restyle(); else Shelf.ensure(getApplicationContext(), apps);
            if (Shelf.active) {
                desk.shelfView.setVisibility(View.GONE);
                if (Build.VERSION.SDK_INT >= 30) getWindow().setDecorFitsSystemWindows(false);
                getWindow().setStatusBarColor(0);
                getWindow().setNavigationBarColor(0);
            }
        } catch (Throwable t) { Log.w("Shelf", "init: " + t); }
        try { Wm.init(getApplicationContext()); } catch (Throwable t) { Log.w("Wm", "init: " + t); }
        if (was) desk.resume();
    }

    // ---------- 崩溃兜底 ----------
    void guard() {
        final Thread.UncaughtExceptionHandler prev = Thread.getDefaultUncaughtExceptionHandler();
        if (prev instanceof Guard) return;
        Thread.setDefaultUncaughtExceptionHandler(new Guard(getApplicationContext(), prev));
    }
    static final class Guard implements Thread.UncaughtExceptionHandler {
        final Context c; final Thread.UncaughtExceptionHandler prev;
        Guard(Context c, Thread.UncaughtExceptionHandler prev) { this.c = c; this.prev = prev; }
        public void uncaughtException(Thread t, Throwable e) {
            try {
                // 偶发一次崩溃只是重启桌面；三分钟内再崩一次才退到安全模式（安全模式没有底栏，不该轻易进）
                SharedPreferences p = Store.ctx(c).getSharedPreferences("desk", 0);
                long now = System.currentTimeMillis(), last = p.getLong("crashAt", 0);
                boolean again = now - last >= 0 && now - last < 180000;
                p.edit().putLong("crashAt", now).putBoolean("safe", again).putInt("safeVer", versionCode(c))
                    .putBoolean("traceSent", false).putString("trace", Log.getStackTraceString(e)).commit();
            } catch (Throwable ignored) {}
            if (prev != null) prev.uncaughtException(t, e);
            else { android.os.Process.killProcess(android.os.Process.myPid()); System.exit(10); }
        }
    }
    void record(Throwable t) {
        sp.edit().putBoolean("safe", true).putInt("safeVer", versionCode(this)).putString("trace", Log.getStackTraceString(t)).commit();
    }
    static int versionCode(Context c) {
        try { return c.getPackageManager().getPackageInfo(c.getPackageName(), 0).versionCode; } catch (Exception e) { return 0; }
    }
    static String versionName(Context c) {
        try { return c.getPackageManager().getPackageInfo(c.getPackageName(), 0).versionName; } catch (Exception e) { return "?"; }
    }

    // ---------- 安全模式：不用任何新界面代码 ----------
    void safeMode() {
        safe = true;
        desk = null;
        final String trace = sp.getString("trace", "");
        ScrollView scroll = new ScrollView(this);
        scroll.setBackgroundColor(0xFF101418);
        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setPadding(32, 32, 32, 32);
        scroll.addView(root);
        row(root, L.t("开放桌面 · 安全模式（", "Открытый стол · безопасный режим (", "Open Desktop · safe mode (") + versionName(this) + L.t("）", ")", ")"), 0xFFE8EDF2, 20, null);
        row(root, L.t("新桌面上次运行出错，已自动退回这个简易列表。控制通道不受影响。", "Новый рабочий стол завершился с ошибкой, поэтому показан этот простой список. Канал управления работает.", "The new desktop crashed last time, so this simple list is shown instead. The control channel is unaffected."), 0xFF8A97A5, 14, null);
        row(root, L.t("▶ 重新启用新桌面", "▶ Включить новый рабочий стол снова", "▶ Re-enable the new desktop"), 0xFF8AB4F8, 16, v -> { sp.edit().putBoolean("safe", false).apply(); recreate(); });
        row(root, L.t("▶ 系统设置", "▶ Настройки системы", "▶ System settings"), 0xFF8AB4F8, 16, v -> go(systemSettings(this)));
        row(root, L.t("▶ 默认桌面", "▶ Рабочий стол по умолчанию", "▶ Default home app"), 0xFF8AB4F8, 16, v -> go(new Intent(Settings.ACTION_HOME_SETTINGS)));
        row(root, L.t("▶ 悬浮窗权限", "▶ Разрешение «Поверх других приложений»", "▶ \"Display over other apps\" permission"), 0xFF8AB4F8, 16, v -> go(new Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:" + getPackageName()))));
        row(root, L.t("应用", "Приложения", "Apps"), 0xFF8A97A5, 13, null);
        try {
            final Intent q = new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER);
            final android.content.pm.PackageManager pm = getPackageManager();
            List<android.content.pm.ResolveInfo> l = pm.queryIntentActivities(q, 0);
            for (final android.content.pm.ResolveInfo r : l) {
                if (getPackageName().equals(r.activityInfo.packageName)) continue;
                row(root, String.valueOf(r.loadLabel(pm)), 0xFFDDE4EA, 16,
                    v -> go(new Intent(q).setClassName(r.activityInfo.packageName, r.activityInfo.name)));
            }
        } catch (Throwable ignored) {}
        if (!trace.isEmpty()) row(root, trace.length() > 1500 ? trace.substring(0, 1500) : trace, 0xFF6C7884, 11, null);
        setContentView(scroll);
        if (!trace.isEmpty() && !sp.getBoolean("traceSent", false)) new Thread(() -> {
            try {
                org.json.JSONObject d = new org.json.JSONObject().put("type", "desktop_crash").put("version", versionName(this)).put("trace", trace);
                HttpURLConnection post = (HttpURLConnection) new URL(AgentService.BASE + "log").openConnection();
                try {
                    post.setConnectTimeout(2500); post.setReadTimeout(6000);
                    post.setRequestMethod("POST"); post.setDoOutput(true);
                    post.setRequestProperty("Content-Type", "application/json");
                    try (OutputStream o = post.getOutputStream()) { o.write(d.toString().getBytes("UTF-8")); }
                    if (post.getResponseCode() < 400) sp.edit().putBoolean("traceSent", true).apply();
                } finally { post.disconnect(); }
            } catch (Throwable ignored) {}
        }).start();
    }
    void row(LinearLayout root, String s, int color, int size, View.OnClickListener l) {
        TextView t = new TextView(this);
        t.setText(s); t.setTextColor(color); t.setTextSize(size); t.setPadding(8, 14, 8, 14);
        if (l != null) { t.setOnClickListener(l); t.setFocusable(true); }
        root.addView(t);
    }
    void go(Intent i) {
        try { startActivity(i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)); }
        catch (RuntimeException e) { Toast.makeText(this, L.t("系统未允许打开", "Система не разрешила открыть", "The system did not allow this"), Toast.LENGTH_SHORT).show(); }
    }

    /** 这台机器上通用的"设置"意图会被移动定制的「设置应用」接走，所以优先点名安卓原生设置。 */
    static Intent systemSettings(Context c) {
        Intent i = new Intent(Intent.ACTION_MAIN).setClassName("com.android.settings", "com.android.settings.Settings");
        return c.getPackageManager().resolveActivity(i, 0) != null ? i : new Intent(Settings.ACTION_SETTINGS);
    }

    // ---------- 生命周期 ----------
    protected void onResume() {
        super.onResume();
        try { startAgent(); } catch (Throwable ignored) {}
        if (desk != null) desk.resume();
        if (AgentService.wantProjection && !AgentService.hasProjection) requestProjection();
    }
    protected void onPause() {
        super.onPause();
        if (desk != null) desk.pause();
    }
    protected void onNewIntent(Intent i) {
        super.onNewIntent(i);
        if (desk == null) return;
        String panel = i.getStringExtra("panel");
        if ("launcher".equals(panel)) desk.toggle(desk.launcher);
        else if ("quick".equals(panel)) desk.toggle(desk.quick);
        else if ("theme".equals(panel)) { sp.edit().putBoolean("dark", !T.dark).apply(); rebuild(); }   // 来自 QuickPanel
        else if ("desksettings".equals(panel)) desk.deskSettings();                                      // 来自 QuickPanel
        else desk.hidePanels();
    }
    public void onBackPressed() { if (desk != null) desk.hidePanels(); }
    public boolean onKeyDown(int code, KeyEvent ev) {
        if (desk != null && ev.getRepeatCount() == 0 && desk.deskKey(code, ev)) return true;
        if (code == KeyEvent.KEYCODE_ESCAPE) return true;
        return super.onKeyDown(code, ev);
    }
    public boolean onKeyUp(int code, KeyEvent ev) {
        if (desk != null && (code == KeyEvent.KEYCODE_META_LEFT || code == KeyEvent.KEYCODE_META_RIGHT)) { desk.toggle(desk.launcher); return true; }
        return super.onKeyUp(code, ev);
    }

    // ---------- 控制通道配合（与旧版行为一致） ----------
    void startAgent() {
        SendService.start(this); // 互传（LocalSend 兼容）服务；内部自带兜底，不会抛出
        Intent svc = new Intent(this, AgentService.class);
        if (Build.VERSION.SDK_INT >= 26) startForegroundService(svc); else startService(svc);
    }
    void requestProjection() {
        try {
            MediaProjectionManager mpm = (MediaProjectionManager) getSystemService(MEDIA_PROJECTION_SERVICE);
            startActivityForResult(mpm.createScreenCaptureIntent(), REQ_PROJECTION);
        } catch (Exception e) { Toast.makeText(this, L.t("无法请求投屏", "Не удалось запросить запись экрана", "Could not request screen capture"), Toast.LENGTH_SHORT).show(); }
    }
    protected void onActivityResult(int req, int res, Intent data) {
        super.onActivityResult(req, res, data);
        if (req == REQ_PROJECTION && res == RESULT_OK && data != null) {
            Intent svc = new Intent(this, AgentService.class).putExtra("code", res).putExtra("data", data);
            if (Build.VERSION.SDK_INT >= 26) startForegroundService(svc); else startService(svc);
        } else if (req == Desk.REQ_WALL && res == RESULT_OK && data != null && data.getData() != null && desk != null) {
            desk.pickedWallpaper(data.getData());
        }
    }
}
