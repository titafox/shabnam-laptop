package local.cloud.launcher;

import android.app.ActivityManager;
import android.app.Instrumentation;
import android.content.*;
import android.content.pm.PackageManager;
import android.graphics.*;
import android.graphics.drawable.*;
import android.os.*;
import android.util.Log;
import android.view.*;
import android.widget.*;
import java.lang.reflect.Array;
import java.lang.reflect.Constructor;
import java.util.*;

/**
 * 常驻底栏：一个独立的系统窗口，所有应用里都在，并向系统申报「导航栏」占位，
 * 应用窗口会自动让出这条高度。需要平台签名（INTERNAL_SYSTEM_WINDOW / STATUS_BAR_SERVICE）。
 * 加不上窗口时 active 保持 false，桌面退回自己在 Activity 里画的底栏。
 */
final class Shelf {
    static final String TAG = "Shelf";
    static Shelf it;
    static boolean active;

    final Context c;
    final Apps apps;
    final WindowManager wm;
    final ActivityManager am;
    final Handler ui = new Handler(Looper.getMainLooper());
    FrameLayout bar;
    LinearLayout row;
    String lastKey = "";
    int focusedTask = -1;

    static void ensure(Context app, Apps apps) {
        if (it != null) { it.render(true); return; }
        Shelf s = new Shelf(app, apps);
        it = s;
        try { s.attach(); active = true; }
        catch (Throwable e) { Log.w(TAG, "shelf window not available: " + e); active = false; }
        if (active) s.poll.run();
    }

    Shelf(Context app, Apps apps) {
        c = app.getApplicationContext(); this.apps = apps;
        wm = (WindowManager) c.getSystemService(Context.WINDOW_SERVICE);
        am = (ActivityManager) c.getSystemService(Context.ACTIVITY_SERVICE);
    }

    int height() { return T.px(52); }

    /** 主题切换后按新配色重建。 */
    void restyle() {
        try { if (bar != null) wm.removeViewImmediate(bar); } catch (Throwable ignored) {}
        bar = null; row = null; lastKey = "";
        try { attach(); active = true; } catch (Throwable e) { Log.w(TAG, "restyle: " + e); active = false; }
    }

    void attach() throws Exception {
        bar = new FrameLayout(c);
        bar.setBackgroundColor(T.shelf | 0xE0000000);

        ImageView home = new ImageView(c);
        home.setImageDrawable(new T.Glyph(T.Glyph.HOME, T.text));
        home.setBackground(new LayerDrawable(new Drawable[]{T.round(T.chip, T.px(18)), T.state(T.px(18))}));
        home.setContentDescription(L.t("启动器", "Меню приложений", "Launcher"));
        home.setOnClickListener(v -> goHome("launcher"));
        home.setOnLongClickListener(v -> { goHome("quick"); return true; });
        home.setOnContextClickListener(v -> { goHome("quick"); return true; });
        FrameLayout.LayoutParams hl = new FrameLayout.LayoutParams(T.px(36), T.px(36), Gravity.LEFT | Gravity.CENTER_VERTICAL);
        hl.leftMargin = T.px(10);
        bar.addView(home, hl);

        row = new LinearLayout(c);
        row.setGravity(Gravity.CENTER);
        bar.addView(row, new FrameLayout.LayoutParams(-2, -1, Gravity.CENTER));

        ImageView back = new ImageView(c);
        back.setImageDrawable(new Back(T.text));
        back.setBackground(new LayerDrawable(new Drawable[]{T.round(T.chip, T.px(18)), T.state(T.px(18))}));
        back.setContentDescription(L.t("返回", "Назад", "Back"));
        back.setOnClickListener(v -> key(KeyEvent.KEYCODE_BACK));
        FrameLayout.LayoutParams bl = new FrameLayout.LayoutParams(T.px(36), T.px(36), Gravity.RIGHT | Gravity.CENTER_VERTICAL);
        bl.rightMargin = T.px(10);
        bar.addView(back, bl);

        WindowManager.LayoutParams lp = new WindowManager.LayoutParams(-1, height(), 2024 /* TYPE_NAVIGATION_BAR_PANEL */,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE | WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN
                | WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL | WindowManager.LayoutParams.FLAG_SPLIT_TOUCH,
            PixelFormat.TRANSLUCENT);
        lp.gravity = Gravity.BOTTOM;
        lp.setTitle("OpenDesktopShelf");
        if (Build.VERSION.SDK_INT >= 30) lp.setFitInsetsTypes(0);
        provideInsets(lp);
        wm.addView(bar, lp);
        render(true);
    }

    /** lp.providedInsets = { navigationBars, tappableElement, mandatorySystemGestures }（隐藏接口，反射）。 */
    void provideInsets(WindowManager.LayoutParams lp) throws Exception {
        Class<?> k = Class.forName("android.view.InsetsFrameProvider");
        Constructor<?> ctor = k.getConstructor(Object.class, int.class, int.class);
        Binder owner = new Binder();
        int[] types = {WindowInsets.Type.navigationBars(), WindowInsets.Type.tappableElement(), WindowInsets.Type.mandatorySystemGestures()};
        Object arr = Array.newInstance(k, types.length);
        for (int i = 0; i < types.length; i++) Array.set(arr, i, ctor.newInstance(owner, 0, types[i]));
        WindowManager.LayoutParams.class.getField("providedInsets").set(lp, arr);
    }

    // ---------- 动作 ----------
    void goHome(String panel) {
        Intent i = new Intent(c, Main.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK).putExtra("panel", panel);
        try { c.startActivity(i); } catch (RuntimeException e) { Log.w(TAG, "home: " + e); }
    }
    void key(final int code) {
        new Thread(() -> { try { new Instrumentation().sendKeyDownUpSync(code); } catch (Throwable e) { Log.w(TAG, "key: " + e); } }).start();
    }
    void open(Apps.App app, int taskId) {
        try {
            if (taskId >= 0) {
                if (Wm.shelfClick(taskId)) return; // 已在最前：最小化
                am.moveTaskToFront(taskId, 0); return;
            }
        } catch (Throwable e) { Log.w(TAG, "front: " + e); }
        apps.markUsed(app);
        try {
            Intent li = new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER).setClassName(app.pkg, app.cls)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_RESET_TASK_IF_NEEDED);
            c.startActivity(li, Wm.opts(c, li));
        } catch (RuntimeException e) { Toast.makeText(c, L.t("系统未允许打开", "Система не разрешила открыть", "The system did not allow this"), Toast.LENGTH_SHORT).show(); }
        ui.postDelayed(() -> render(false), 600);
    }
    void closeTask(int taskId) {
        try { if (!Wm.removeTask(taskId)) ActivityManager.class.getMethod("removeTask", int.class).invoke(am, taskId); }
        catch (Throwable e) { Log.w(TAG, "close: " + e); }
        ui.postDelayed(() -> render(false), 300);
    }

    // ---------- 内容 ----------
    static final class Item { Apps.App app; int task = -1; boolean pinned, focused; }

    List<Item> items() {
        List<Item> out = new ArrayList<>();
        Map<String, Item> byPkg = new HashMap<>();
        for (String id : apps.pinIds()) {
            Apps.App a = apps.find(id);
            if (a == null) continue;
            Item x = new Item(); x.app = a; x.pinned = true;
            out.add(x);
            if (!byPkg.containsKey(a.pkg)) byPkg.put(a.pkg, x);
        }
        focusedTask = -1;
        try {
            List<ActivityManager.RunningTaskInfo> ts = am.getRunningTasks(24);
            boolean first = true;
            for (ActivityManager.RunningTaskInfo t : ts) {
                ComponentName base = t.baseActivity != null ? t.baseActivity : t.topActivity;
                if (base == null) continue;
                String pkg = base.getPackageName();
                boolean top = first; first = false;
                if (pkg.equals(c.getPackageName())) continue;
                Item x = byPkg.get(pkg);
                if (x == null) {
                    Apps.App a = null;
                    for (Apps.App y : apps.all) if (y.pkg.equals(pkg)) { a = y; break; }
                    if (a == null) continue; // 没有桌面入口的任务（系统界面等）不显示
                    x = new Item(); x.app = a;
                    out.add(x); byPkg.put(pkg, x);
                }
                if (x.task < 0) x.task = t.id;
                if (top) { x.focused = true; focusedTask = t.id; }
            }
        } catch (Throwable e) { Log.w(TAG, "tasks: " + e); }
        return out;
    }

    void render(boolean force) {
        if (row == null) return;
        List<Item> its = items();
        StringBuilder k = new StringBuilder();
        for (Item x : its) k.append(x.app.id()).append(x.task >= 0 ? 'r' : '-').append(x.focused ? 'f' : '-').append(x.pinned ? 'p' : '-').append(';');
        String key = k.toString() + T.dark;
        if (!force && key.equals(lastKey)) return;
        lastKey = key;
        row.removeAllViews();
        boolean sep = false;
        for (final Item x : its) {
            if (!x.pinned && !sep) {
                sep = true;
                if (row.getChildCount() > 0) {
                    View line = new View(c);
                    line.setBackgroundColor(T.chip);
                    LinearLayout.LayoutParams ll = new LinearLayout.LayoutParams(Math.max(1, T.px(1)), T.px(24));
                    ll.leftMargin = ll.rightMargin = T.px(8);
                    row.addView(line, ll);
                }
            }
            FrameLayout cell = new FrameLayout(c);
            cell.setBackground(x.focused ? new LayerDrawable(new Drawable[]{T.round(T.chip, T.px(12)), T.state(T.px(12))}) : T.state(T.px(12)));
            ImageView iv = new ImageView(c);
            iv.setImageBitmap(x.app.icon);
            FrameLayout.LayoutParams il = new FrameLayout.LayoutParams(T.px(34), T.px(34), Gravity.CENTER);
            cell.addView(iv, il);
            if (x.task >= 0) {
                View dot = new View(c);
                dot.setBackground(T.round(x.focused ? T.accent : T.text2, T.px(2)));
                FrameLayout.LayoutParams dl = new FrameLayout.LayoutParams(x.focused ? T.px(14) : T.px(5), T.px(3), Gravity.BOTTOM | Gravity.CENTER_HORIZONTAL);
                dl.bottomMargin = T.px(1);
                cell.addView(dot, dl);
            }
            cell.setContentDescription(x.app.label);
            if (Build.VERSION.SDK_INT >= 26) cell.setTooltipText(x.app.label);
            cell.setOnClickListener(v -> open(x.app, x.task));
            cell.setOnLongClickListener(v -> { menu(v, x); return true; });
            cell.setOnContextClickListener(v -> { menu(v, x); return true; });
            LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(T.px(44), T.px(44));
            lp.leftMargin = lp.rightMargin = T.px(3);
            row.addView(cell, lp);
        }
    }

    void menu(View anchor, final Item x) {
        try {
            Context th = new ContextThemeWrapper(c, T.dark ? android.R.style.Theme_DeviceDefault : android.R.style.Theme_DeviceDefault_Light);
            PopupMenu m = new PopupMenu(th, anchor);
            m.getMenu().add(0, 1, 0, x.pinned ? L.t("取消固定", "Открепить", "Unpin") : L.t("固定到底栏", "Закрепить на панели", "Pin to shelf"));
            if (x.task >= 0) m.getMenu().add(0, 2, 1, L.t("关闭", "Закрыть", "Close"));
            m.setOnMenuItemClickListener(mi -> {
                if (mi.getItemId() == 1) { apps.togglePin(x.app); render(true); if (apps.listener != null) apps.listener.onApps(); }
                else closeTask(x.task);
                return true;
            });
            m.show();
        } catch (Throwable e) { Log.w(TAG, "menu: " + e); }
    }

    final Runnable poll = new Runnable() {
        public void run() {
            try { render(false); } catch (Throwable e) { Log.w(TAG, "render: " + e); }
            ui.postDelayed(this, 1200);
        }
    };

    /** 返回箭头。 */
    static final class Back extends Drawable {
        final Paint p = new Paint(Paint.ANTI_ALIAS_FLAG);
        Back(int color) { p.setColor(color); p.setStyle(Paint.Style.STROKE); p.setStrokeCap(Paint.Cap.ROUND); p.setStrokeJoin(Paint.Join.ROUND); }
        public void draw(Canvas cv) {
            Rect b = getBounds();
            float w = b.width(), cx = b.exactCenterX(), cy = b.exactCenterY(), r = w * 0.17f;
            p.setStrokeWidth(w * 0.07f);
            Path path = new Path();
            path.moveTo(cx + r * 0.5f, cy - r); path.lineTo(cx - r * 0.6f, cy); path.lineTo(cx + r * 0.5f, cy + r);
            cv.drawPath(path, p);
        }
        public void setAlpha(int a) {}
        public void setColorFilter(ColorFilter f) {}
        public int getOpacity() { return PixelFormat.TRANSLUCENT; }
    }
}
