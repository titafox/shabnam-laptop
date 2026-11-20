package local.cloud.launcher;
import android.app.Activity;
import android.content.*;
import android.content.pm.*;
import android.graphics.drawable.Drawable;
import android.media.projection.MediaProjectionManager;
import android.net.Uri;
import android.os.*;
import android.provider.Settings;
import android.util.TypedValue;
import android.view.*;
import android.widget.*;
import java.util.*;

public class Main extends Activity {
    LinearLayout root;
    TextView clock, status;
    final Handler ui = new Handler(Looper.getMainLooper());
    int pad;
    static final int REQ_PROJECTION = 42;

    public void onCreate(Bundle state) {
        super.onCreate(state);
        pad = dp(10);
        ScrollView scroll = new ScrollView(this);
        scroll.setBackgroundColor(0xFF101418);
        scroll.setFillViewport(true);
        root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setPadding(pad, pad, pad, pad);
        scroll.addView(root);
        setContentView(scroll);
        startAgent();
    }
    protected void onResume() {
        super.onResume();
        startAgent();
        build();
        tick();
        if (AgentService.wantProjection && !AgentService.hasProjection) requestProjection();
    }
    protected void onPause() { super.onPause(); ui.removeCallbacksAndMessages(null); }

    void startAgent() {
        Intent svc = new Intent(this, AgentService.class);
        if (Build.VERSION.SDK_INT >= 26) startForegroundService(svc); else startService(svc);
    }
    void requestProjection() {
        try {
            MediaProjectionManager mpm = (MediaProjectionManager) getSystemService(MEDIA_PROJECTION_SERVICE);
            startActivityForResult(mpm.createScreenCaptureIntent(), REQ_PROJECTION);
        } catch (Exception e) { Toast.makeText(this, "无法请求投屏", Toast.LENGTH_SHORT).show(); }
    }
    protected void onActivityResult(int req, int res, Intent data) {
        super.onActivityResult(req, res, data);
        if (req == REQ_PROJECTION && res == RESULT_OK && data != null) {
            Intent svc = new Intent(this, AgentService.class).putExtra("code", res).putExtra("data", data);
            if (Build.VERSION.SDK_INT >= 26) startForegroundService(svc); else startService(svc);
        }
    }
    void tick() {
        clock.setText(android.text.format.DateFormat.format("yyyy-MM-dd  EEEE  HH:mm", new Date()));
        String s = "控制通道：" + AgentService.status;
        if (!Settings.canDrawOverlays(this)) s += "\n⚠ 请授予『显示在其他应用上层』权限（点这里）";
        status.setText(s);
        ui.postDelayed(this::tick, 2000);
    }
    int dp(int v) { return (int) TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v, getResources().getDisplayMetrics()); }

    void build() {
        root.removeAllViews();
        clock = new TextView(this);
        clock.setTextColor(0xFFE8EDF2); clock.setTextSize(18); clock.setPadding(pad, pad, pad, 0);
        root.addView(clock);
        status = new TextView(this);
        status.setTextColor(0xFF6FB98F); status.setTextSize(12); status.setPadding(pad, dp(2), pad, pad);
        status.setOnClickListener(v -> {
            if (!Settings.canDrawOverlays(this))
                open(new Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:" + getPackageName())));
        });
        root.addView(status);
        header("系统");
        GridLayout sys = grid(); root.addView(sys);
        tile(sys, "设置", new Intent().setComponent(new ComponentName(
                "com.android.settings", "com.android.settings.Settings")));
        tile(sys, "应用管理", new Intent(Settings.ACTION_MANAGE_APPLICATIONS_SETTINGS));
        tile(sys, "开发者选项", new Intent(Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS));
        tile(sys, "悬浮窗权限", new Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:" + getPackageName())));
        tile(sys, "存储", new Intent(Settings.ACTION_INTERNAL_STORAGE_SETTINGS));
        tile(sys, "Wi-Fi", new Intent(Settings.ACTION_WIFI_SETTINGS));
        tile(sys, "默认桌面", new Intent(Settings.ACTION_HOME_SETTINGS));
        tile(sys, "安全", new Intent(Settings.ACTION_SECURITY_SETTINGS));
        header("应用（点按打开，长按应用信息）");
        GridLayout apps = grid(); root.addView(apps);
        Intent query = new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER);
        PackageManager pm = getPackageManager();
        List<ResolveInfo> list = pm.queryIntentActivities(query, 0);
        Collections.sort(list, (a, b) -> a.loadLabel(pm).toString().compareToIgnoreCase(b.loadLabel(pm).toString()));
        for (ResolveInfo app : list) {
            String pkg = app.activityInfo.packageName;
            if (getPackageName().equals(pkg)) continue;
            Intent launch = new Intent(query).setClassName(pkg, app.activityInfo.name).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            Drawable icon = null; try { icon = app.loadIcon(pm); } catch (Exception ignored) {}
            appTile(apps, app.loadLabel(pm).toString(), icon, launch, pkg);
        }
    }
    void header(String text) {
        TextView t = new TextView(this);
        t.setText(text); t.setTextColor(0xFF8A97A5); t.setTextSize(13);
        t.setPadding(pad, dp(14), pad, dp(6)); root.addView(t);
    }
    GridLayout grid() {
        GridLayout g = new GridLayout(this);
        g.setColumnCount(Math.max(3, getResources().getDisplayMetrics().widthPixels / dp(104)));
        return g;
    }
    GridLayout.LayoutParams cell() {
        GridLayout.LayoutParams lp = new GridLayout.LayoutParams();
        lp.width = dp(92); lp.height = dp(92); lp.setMargins(dp(6), dp(6), dp(6), dp(6)); return lp;
    }
    void tile(GridLayout parent, String label, Intent intent) {
        TextView t = new TextView(this);
        t.setText(label); t.setGravity(Gravity.CENTER); t.setTextColor(0xFFFFFFFF); t.setTextSize(13);
        t.setBackgroundColor(0xFF3A6EA5); t.setLayoutParams(cell());
        t.setOnClickListener(v -> open(intent)); parent.addView(t);
    }
    void appTile(GridLayout parent, String label, Drawable icon, Intent launch, String pkg) {
        LinearLayout box = new LinearLayout(this);
        box.setOrientation(LinearLayout.VERTICAL); box.setGravity(Gravity.CENTER);
        box.setBackgroundColor(0xFF1B222A); box.setLayoutParams(cell());
        ImageView iv = new ImageView(this);
        iv.setLayoutParams(new LinearLayout.LayoutParams(dp(44), dp(44)));
        if (icon != null) iv.setImageDrawable(icon);
        TextView t = new TextView(this);
        t.setText(label); t.setMaxLines(2); t.setGravity(Gravity.CENTER); t.setTextColor(0xFFDDE4EA);
        t.setTextSize(11); t.setPadding(dp(2), dp(4), dp(2), 0);
        box.addView(iv); box.addView(t);
        box.setOnClickListener(v -> open(launch));
        box.setOnLongClickListener(v -> { open(new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:" + pkg))); return true; });
        parent.addView(box);
    }
    void open(Intent intent) {
        try { startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)); }
        catch (RuntimeException e) { Toast.makeText(this, "系统未允许打开", Toast.LENGTH_SHORT).show(); }
    }
}
