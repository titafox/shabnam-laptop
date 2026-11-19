package local.cloud.launcher;

import android.app.ActivityManager;
import android.app.ActivityOptions;
import android.content.*;
import android.content.pm.ResolveInfo;
import android.graphics.Point;
import android.graphics.Rect;
import android.os.*;
import android.provider.Settings;
import android.util.Log;
import android.util.SparseArray;
import android.view.WindowManager;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.*;

/**
 * 桌面式窗口管理：整块屏幕切到「自由窗口」模式，启动应用时给出合适的窗口大小并记住每个应用上次的位置，
 * 最小化 / 左右半屏 / 最大化 / 还原 / 显示桌面。全部走隐藏接口（反射），需要平台签名。
 * 任何一步失败都只记日志，绝不影响底栏和桌面本身。
 */
final class Wm {
    static final String TAG = "Wm";
    static final String ACTION = "local.cloud.launcher.WM";
    static final int LEFT = 1, RIGHT = 2, MAX = 3, RESTORE = 4, DOWN = 5;
    static final int MODE_FULLSCREEN = 1, MODE_FREEFORM = 5;

    static Context c;
    static ActivityManager am;
    static SharedPreferences sp;
    static final Handler ui = new Handler(Looper.getMainLooper());
    static boolean freeform;
    static final Map<String, Rect> saved = new HashMap<>();
    static final SparseArray<Rect> restore = new SparseArray<>();
    static final Set<Integer> seen = new HashSet<>();
    static List<Integer> hidden = new ArrayList<>();
    static boolean savedDirty;

    /** 一个任务的快照。 */
    static final class Tk {
        int id, mode, type;
        String pkg;
        Object token;
        Rect bounds = new Rect();
        boolean visible, focused;
    }

    // ---------- 启动 ----------
    static synchronized void init(Context app) {
        if (c != null) return;
        try {
            c = app.getApplicationContext();
            am = (ActivityManager) c.getSystemService(Context.ACTIVITY_SERVICE);
            sp = Store.ctx(c).getSharedPreferences("wm", 0);
            load();
            try { ensureFreeform(); } catch (Throwable e) { Log.w(TAG, "freeform: " + e); }
            try { ensureKeys(); } catch (Throwable e) { Log.w(TAG, "keys: " + e); }
            try {
                // 调试 / 脚本入口：adb shell am broadcast -a local.cloud.launcher.WM --es cmd left
                c.registerReceiver(new BroadcastReceiver() {
                    public void onReceive(Context x, Intent i) { try { command(i.getStringExtra("cmd")); } catch (Throwable e) { Log.w(TAG, "cmd: " + e); } }
                }, new IntentFilter(ACTION), "android.permission.INJECT_EVENTS", null);
            } catch (Throwable e) { Log.w(TAG, "receiver: " + e); }
            ui.postDelayed(poll, 1500);
        } catch (Throwable e) { Log.w(TAG, "init: " + e); }
    }

    /** 屏幕窗口模式设为自由窗口（系统会记在 display_settings.xml 里，重启后仍在）；每次启动再核对一遍。 */
    static void ensureFreeform() throws Exception {
        boolean want = sp.getBoolean("freeform", true);
        ContentResolver cr = c.getContentResolver();
        if (want) {
            if (Settings.Global.getInt(cr, "enable_freeform_support", 0) != 1) Settings.Global.putInt(cr, "enable_freeform_support", 1);
            if (Settings.Global.getInt(cr, "force_resizable_activities", 0) != 1) Settings.Global.putInt(cr, "force_resizable_activities", 1);
        }
        Object iwm = Class.forName("android.view.WindowManagerGlobal").getMethod("getWindowManagerService").invoke(null);
        int mode = (Integer) iwm.getClass().getMethod("getWindowingMode", int.class).invoke(iwm, 0);
        int target = want ? MODE_FREEFORM : MODE_FULLSCREEN;
        if (mode != target) {
            iwm.getClass().getMethod("setWindowingMode", int.class, int.class).invoke(iwm, 0, target);
            Log.i(TAG, "display windowing mode " + mode + " -> " + target);
        }
        freeform = want;
    }

    /** 全局快捷键靠无障碍服务的按键过滤，自己把自己加进已启用列表。 */
    static void ensureKeys() {
        if (!sp.getBoolean("keys", true)) return;
        ContentResolver cr = c.getContentResolver();
        String me = new ComponentName(c, Keys.class).flattenToString();
        String cur = Settings.Secure.getString(cr, "enabled_accessibility_services");
        if (cur == null) cur = "";
        boolean has = false;
        for (String s : cur.split(":")) {
            ComponentName n = ComponentName.unflattenFromString(s);
            if (n != null && n.equals(new ComponentName(c, Keys.class))) has = true;
        }
        if (!has) {
            Settings.Secure.putString(cr, "enabled_accessibility_services", cur.isEmpty() ? me : cur + ":" + me);
            Log.i(TAG, "enabled key service");
        }
        if (Settings.Secure.getInt(cr, "accessibility_enabled", 0) != 1) Settings.Secure.putInt(cr, "accessibility_enabled", 1);
    }

    // ---------- 任务列表 ----------
    static Field fToken, fConf, fVisible, fFocused, fWinConf;
    static Method mBounds, mMode, mType;
    static boolean refl;

    static void reflect() throws Exception {
        if (refl) return;
        Class<?> ti = Class.forName("android.app.TaskInfo");
        fToken = ti.getField("token");
        fConf = ti.getField("configuration");
        fVisible = ti.getField("isVisible");
        fFocused = ti.getField("isFocused");
        fWinConf = android.content.res.Configuration.class.getField("windowConfiguration");
        Class<?> wc = Class.forName("android.app.WindowConfiguration");
        mBounds = wc.getMethod("getBounds");
        mMode = wc.getMethod("getWindowingMode");
        mType = wc.getMethod("getActivityType");
        refl = true;
    }

    /** 所有普通应用任务（不含桌面自己），最近用的在前。 */
    static List<Tk> tasks() {
        List<Tk> out = new ArrayList<>();
        try {
            reflect();
            for (ActivityManager.RunningTaskInfo t : am.getRunningTasks(40)) {
                ComponentName base = t.baseActivity != null ? t.baseActivity : t.topActivity;
                if (base == null) continue;
                Tk k = new Tk();
                k.id = t.id; k.pkg = base.getPackageName();
                k.token = fToken.get(t);
                k.visible = fVisible.getBoolean(t);
                k.focused = fFocused.getBoolean(t);
                Object wc = fWinConf.get(fConf.get(t));
                k.bounds.set((Rect) mBounds.invoke(wc));
                k.mode = (Integer) mMode.invoke(wc);
                k.type = (Integer) mType.invoke(wc);
                out.add(k);
            }
        } catch (Throwable e) { Log.w(TAG, "tasks: " + e); }
        return out;
    }
    /** 桌面任务不在了（更新安装后系统会把它清掉，而前面还有别的窗口时系统不会自动重开）：把它开回来，底栏才会在。 */
    static void ensureHome() {
        try {
            if (c == null) return;
            List<Tk> ts = tasks();
            for (Tk k : ts) if (k.type == 2 && k.pkg.equals(c.getPackageName())) return;
            if (ts.isEmpty() && Shelf.it != null) return; // 查不到任务时不乱动
            Log.i(TAG, "home task missing, starting it");
            home("");
        } catch (Throwable e) { Log.w(TAG, "ensureHome: " + e); }
    }
    static boolean app(Tk k) { return k.type <= 1 && !k.pkg.equals(c.getPackageName()); }

    static List<Tk> appTasks() {
        List<Tk> out = new ArrayList<>();
        for (Tk k : tasks()) if (app(k)) out.add(k);
        return out;
    }

    /** 当前在最前面的应用任务；桌面在最前时返回 null。 */
    static Tk focused() {
        List<Tk> ts = tasks();
        if (ts.isEmpty()) return null;
        Tk f = null;
        for (Tk k : ts) if (k.focused) { f = k; break; }
        if (f == null) f = ts.get(0);
        return app(f) && f.visible ? f : null;
    }

    // ---------- 尺寸 ----------
    /** 状态栏以下、底栏以上的可用区域。 */
    static Rect work() {
        Point p = new Point();
        ((WindowManager) c.getSystemService(Context.WINDOW_SERVICE)).getDefaultDisplay().getRealSize(p);
        int top = 0, bottom = 0;
        if (Build.VERSION.SDK_INT >= 30) try {
            // 这台机器的 status_bar_height 资源是 0，要按实际占位算
            android.view.WindowInsets in = ((WindowManager) c.getSystemService(Context.WINDOW_SERVICE)).getCurrentWindowMetrics().getWindowInsets();
            top = in.getInsetsIgnoringVisibility(android.view.WindowInsets.Type.statusBars()).top;
            bottom = in.getInsetsIgnoringVisibility(android.view.WindowInsets.Type.navigationBars()).bottom;
            if (bottom > 0) return new Rect(0, top, p.x, p.y - bottom);
        } catch (Throwable ignored) {}
        try {
            int id = c.getResources().getIdentifier("status_bar_height", "dimen", "android");
            if (id != 0 && top == 0) top = c.getResources().getDimensionPixelSize(id);
        } catch (Throwable ignored) {}
        if (Shelf.it == null || Shelf.active) bottom = T.px(52); // 底栏高度（底栏还没建好时也按它算）
        else try {
            int id = c.getResources().getIdentifier("navigation_bar_height", "dimen", "android");
            if (id != 0) bottom = c.getResources().getDimensionPixelSize(id);
        } catch (Throwable ignored) {}
        return new Rect(0, top, p.x, p.y - bottom);
    }
    static Rect rect(int how) {
        Rect w = work();
        if (how == LEFT) return new Rect(w.left, w.top, w.left + w.width() / 2, w.bottom);
        if (how == RIGHT) return new Rect(w.left + w.width() / 2, w.top, w.right, w.bottom);
        if (how == MAX) return w;
        int dw = w.width() / 10, dh = w.height() / 10; // 默认：居中，占可用区域约 80%
        return new Rect(w.left + dw, w.top + dh, w.right - dw, w.bottom - dh);
    }
    static boolean snapped(Tk k) {
        return k.mode == MODE_FULLSCREEN || k.bounds.equals(rect(LEFT)) || k.bounds.equals(rect(RIGHT)) || k.bounds.equals(rect(MAX));
    }
    /**
     * 系统自己给新窗口定的默认大小：竖屏应用是手机那么大的竖条，横屏应用是「高 = 可用高度的平方 / 可用宽度」的小横条
     * （TaskLaunchParamsModifier 里写死的算法，没有可调的系统开关）。
     */
    static boolean phoneSized(Rect b) {
        Rect w = work();
        if (b.width() < b.height() && b.width() <= w.width() * 3 / 10) return true;
        int ph = Math.min(w.width(), w.height()), pw = ph * ph / Math.max(w.width(), w.height());
        return Math.abs(b.width() - ph) <= 2 && Math.abs(b.height() - pw) <= 2;
    }
    /** 把记下的位置收进可用区域（换了分辨率、底栏高度变了之后仍然看得见）。 */
    static Rect fit(Rect r) {
        Rect w = work();
        Rect o = new Rect(r);
        if (o.width() > w.width()) { o.left = w.left; o.right = w.right; }
        if (o.height() > w.height()) { o.top = w.top; o.bottom = w.bottom; }
        if (o.left < w.left) o.offset(w.left - o.left, 0);
        if (o.right > w.right) o.offset(w.right - o.right, 0);
        if (o.top < w.top) o.offset(0, w.top - o.top);
        if (o.bottom > w.bottom) o.offset(0, w.bottom - o.bottom);
        return o;
    }

    /** 某个应用下次打开用的窗口位置：上次的，没有就用默认（和已有窗口错开一点）。 */
    static Rect boundsFor(String pkg, List<Tk> running) {
        Rect s = pkg == null ? null : saved.get(pkg);
        if (s != null && s.width() > 100 && s.height() > 100) return fit(s);
        Rect d = rect(0);
        int step = T.px(24);
        for (int n = 0; n < 4 && running != null; n++) {
            boolean clash = false;
            for (Tk k : running) if (k.visible && k.bounds.left == d.left && k.bounds.top == d.top) { clash = true; break; }
            if (!clash) break;
            d.offset(step, step);
        }
        return fit(d);
    }

    /** 启动应用时带上的窗口位置；不在自由窗口模式或出错时返回 null（等于不带）。 */
    static Bundle opts(Context x, Intent i) {
        try {
            if (c == null || !freeform || Build.VERSION.SDK_INT < 24) return null;
            String pkg = i.getComponent() != null ? i.getComponent().getPackageName() : i.getPackage();
            if (pkg == null) {
                ResolveInfo r = x.getPackageManager().resolveActivity(i, 0);
                if (r != null && r.activityInfo != null) pkg = r.activityInfo.packageName;
            }
            if (c.getPackageName().equals(pkg)) return null;
            ActivityOptions o = ActivityOptions.makeBasic();
            o.setLaunchBounds(boundsFor(pkg, appTasks()));
            return o.toBundle();
        } catch (Throwable e) { Log.w(TAG, "opts: " + e); return null; }
    }

    // ---------- 动作 ----------
    static Object wct() throws Exception { return Class.forName("android.window.WindowContainerTransaction").newInstance(); }
    static void commit(Object t) throws Exception {
        Class<?> org = Class.forName("android.window.WindowOrganizer");
        org.getMethod("applyTransaction", t.getClass()).invoke(org.newInstance(), t);
    }
    static Class<?> tokenClass() throws Exception { return Class.forName("android.window.WindowContainerToken"); }

    static boolean setBounds(Tk k, Rect r) {
        try {
            Object t = wct();
            if (k.mode != MODE_FREEFORM) t.getClass().getMethod("setWindowingMode", tokenClass(), int.class).invoke(t, k.token, MODE_FREEFORM);
            t.getClass().getMethod("setBounds", tokenClass(), Rect.class).invoke(t, k.token, r);
            commit(t);
            return true;
        } catch (Throwable e) { Log.w(TAG, "setBounds: " + e); return false; }
    }
    static boolean minimize(Tk k) {
        try {
            Object t = wct();
            t.getClass().getMethod("reorder", tokenClass(), boolean.class).invoke(t, k.token, false);
            commit(t);
            refreshShelf();
            return true;
        } catch (Throwable e) { Log.w(TAG, "minimize: " + e); return false; }
    }
    static void front(int taskId) {
        try { am.moveTaskToFront(taskId, 0); } catch (Throwable e) { Log.w(TAG, "front: " + e); }
        refreshShelf();
    }
    static void refreshShelf() {
        ui.postDelayed(() -> { try { if (Shelf.it != null) Shelf.it.render(false); } catch (Throwable ignored) {} }, 250);
    }

    /** 底栏点了一个正在运行的任务：它就在最前面时最小化并返回 true；否则返回 false，由底栏把它调到前面。 */
    static boolean shelfClick(int taskId) {
        try {
            if (c == null) return false;
            Tk f = focused();
            return f != null && f.id == taskId && minimize(f);
        } catch (Throwable e) { Log.w(TAG, "shelfClick: " + e); return false; }
    }

    static void snap(int how) {
        try {
            Tk f = focused();
            if (f == null) return;
            boolean sn = snapped(f);
            if (how == RESTORE || how == DOWN) {
                if (!sn) { if (how == DOWN) minimize(f); return; }
                Rect r = restore.get(f.id);
                setBounds(f, fit(r != null ? r : rect(0)));
                return;
            }
            if (!sn && f.mode == MODE_FREEFORM) restore.put(f.id, new Rect(f.bounds));
            setBounds(f, rect(how));
        } catch (Throwable e) { Log.w(TAG, "snap: " + e); }
    }
    static void minimizeFocused() { try { Tk f = focused(); if (f != null) minimize(f); } catch (Throwable e) { Log.w(TAG, "min: " + e); } }

    static void close() {
        try {
            Tk f = focused();
            if (f == null) return;
            removeTask(f.id);
        } catch (Throwable e) { Log.w(TAG, "close: " + e); }
    }
    /** 按任务号关闭。安卓 15 上 ActivityManager.removeTask 已不存在，要走 ActivityTaskManager。 */
    static boolean removeTask(int taskId) {
        try {
            try {
                Object atm = Class.forName("android.app.ActivityTaskManager").getMethod("getService").invoke(null);
                atm.getClass().getMethod("removeTask", int.class).invoke(atm, taskId);
            } catch (NoSuchMethodException | ClassNotFoundException old) {
                ActivityManager a = am != null ? am : null;
                if (a == null) return false;
                ActivityManager.class.getMethod("removeTask", int.class).invoke(a, taskId);
            }
            refreshShelf();
            return true;
        } catch (Throwable e) { Log.w(TAG, "removeTask: " + e); return false; }
    }

    /** 显示桌面；桌面已在最前且刚才是这样收起的，再按一次把窗口按原来的层次放回来。 */
    static void desktop() {
        try {
            List<Integer> vis = new ArrayList<>();
            for (Tk k : appTasks()) if (k.visible) vis.add(k.id);
            if (vis.isEmpty()) {
                List<Integer> back = hidden;
                hidden = new ArrayList<>();
                for (int i = back.size() - 1; i >= 0; i--) am.moveTaskToFront(back.get(i), 0);
                if (back.isEmpty()) home("");
            } else {
                hidden = vis;
                home("");
            }
            refreshShelf();
        } catch (Throwable e) { Log.w(TAG, "desktop: " + e); }
    }
    static void home(String panel) {
        // 必须是「不点名组件的 HOME 意图」：否则桌面任务不在时（刚更新完）会被当成普通应用开成一个小窗口
        try { c.startActivity(new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME).setPackage(c.getPackageName()).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK).putExtra("panel", panel)); }
        catch (Throwable e) { Log.w(TAG, "home: " + e); }
    }

    static void command(String cmd) {
        if (cmd == null) return;
        switch (cmd) {
            case "left": snap(LEFT); break;
            case "right": snap(RIGHT); break;
            case "max": snap(MAX); break;
            case "restore": snap(RESTORE); break;
            case "down": snap(DOWN); break;
            case "min": minimizeFocused(); break;
            case "close": close(); break;
            case "desktop": desktop(); break;
            case "launcher": home("launcher"); break;
            case "next": Switcher.step(c, 1); break;
            case "prev": Switcher.step(c, -1); break;
            case "commit": Switcher.commit(); break;
            case "cancel": Switcher.cancel(); break;
            case "freeform-on": case "freeform-off":
                sp.edit().putBoolean("freeform", cmd.endsWith("on")).apply();
                try { ensureFreeform(); } catch (Throwable e) { Log.w(TAG, "freeform: " + e); }
                break;
            case "keys-on": case "keys-off":
                sp.edit().putBoolean("keys", cmd.endsWith("on")).apply();
                if (cmd.endsWith("on")) ensureKeys();
                break;
            case "forget":
                saved.clear();
                SharedPreferences.Editor ed = sp.edit();
                for (String k : sp.getAll().keySet()) if (k.startsWith("b:")) ed.remove(k);
                ed.apply();
                break;
            case "dump":
                Tk f = focused();
                Log.i(TAG, "work=" + work() + " freeform=" + freeform + " keys=" + (Keys.it != null) + " focused=" + (f == null ? "-" : f.id + "/" + f.pkg));
                for (Tk k : tasks()) Log.i(TAG, "task " + k.id + " " + k.pkg + " mode=" + k.mode + " type=" + k.type + " vis=" + k.visible + " foc=" + k.focused + " " + k.bounds + " token=" + (k.token != null));
                Log.i(TAG, "saved=" + saved);
                break;
        }
    }

    // ---------- 记住位置、纠正手机大小的新窗口 ----------
    static final Runnable poll = new Runnable() {
        public void run() {
            scan();
            ui.postDelayed(this, 1500);
        }
    };
    static final Runnable kicked = new Runnable() { public void run() { scan(); } };
    /** 有窗口变化时尽快看一眼（由无障碍事件触发）。 */
    static void kick() { ui.removeCallbacks(kicked); ui.postDelayed(kicked, 120); }

    static void scan() {
        try {
            if (c == null || !freeform) return;
            List<Tk> ts = appTasks();
            Set<Integer> alive = new HashSet<>();
            for (Tk k : ts) {
                alive.add(k.id);
                boolean first = seen.add(k.id);
                if (k.mode != MODE_FREEFORM || k.bounds.isEmpty()) continue;
                if (first && phoneSized(k.bounds)) {
                    // 别的应用（或系统）拉起来的新窗口：系统默认给手机大小，这里换成记住的 / 默认的大小
                    Rect want = boundsFor(k.pkg, ts);
                    if (!want.equals(k.bounds) && !phoneSized(want)) { setBounds(k, want); continue; }
                }
                if (!snapped(k)) restore.put(k.id, new Rect(k.bounds));
                if (k.visible) remember(k.pkg, k.bounds);
            }
            for (Iterator<Integer> it = seen.iterator(); it.hasNext(); ) {
                int id = it.next();
                if (!alive.contains(id)) { it.remove(); restore.remove(id); }
            }
            if (savedDirty) store();
        } catch (Throwable e) { Log.w(TAG, "scan: " + e); }
    }
    static void remember(String pkg, Rect b) {
        Rect old = saved.get(pkg);
        if (old != null && old.equals(b)) return;
        saved.put(pkg, new Rect(b));
        savedDirty = true;
    }
    static void load() {
        try {
            for (Map.Entry<String, ?> e : sp.getAll().entrySet()) {
                if (!e.getKey().startsWith("b:") || !(e.getValue() instanceof String)) continue;
                Rect r = Rect.unflattenFromString((String) e.getValue());
                if (r != null) saved.put(e.getKey().substring(2), r);
            }
        } catch (Throwable e) { Log.w(TAG, "load: " + e); }
    }
    static void store() {
        savedDirty = false;
        SharedPreferences.Editor ed = sp.edit();
        for (Map.Entry<String, Rect> e : saved.entrySet()) ed.putString("b:" + e.getKey(), e.getValue().flattenToString());
        ed.apply();
    }
}
