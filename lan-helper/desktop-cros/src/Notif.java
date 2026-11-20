package local.cloud.launcher;

import android.app.ActivityOptions;
import android.app.Notification;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.graphics.PixelFormat;
import android.graphics.Typeface;
import android.os.*;
import android.service.notification.NotificationListenerService;
import android.service.notification.StatusBarNotification;
import android.text.TextUtils;
import android.util.Log;
import android.view.*;
import android.widget.*;
import java.util.*;

/**
 * 通知中心的数据与右下角弹出卡片。
 * 列表只收「可清除的普通通知」：常驻 / 前台服务 / 分组摘要 / 本应用自己的通知 / 最低重要级别的都跳过。
 * 全部方法都在主线程调用（监听服务的回调本来就在主线程）。
 */
final class Notif {
    static final String TAG = "Notif";
    static final int MAX_CARDS = 3, SHOW_MS = 6000, AFTER_HOVER_MS = 2500;

    static Context app;
    static NotificationListenerService svc;
    static boolean tried;
    static final List<StatusBarNotification> items = new ArrayList<>(); // 新的在前
    static final Handler ui = new Handler(Looper.getMainLooper());

    static boolean live() { return svc != null; }
    static int count() { return items.size(); }

    // ================= 接入 =================
    static void init(Context c) {
        app = c.getApplicationContext();
        if (tried || svc != null) return;
        tried = true;
        ui.postDelayed(Notif::ensure, 1500);
    }

    /** 让系统把通知交给我们：先给清单里的服务授权；不行就像系统界面那样直接注册成系统监听器。 */
    static void ensure() {
        if (svc != null || app == null) return;
        ComponentName cn = new ComponentName(app, NotifService.class);
        boolean ok = false;
        try {
            NotificationManager nm = (NotificationManager) app.getSystemService(Context.NOTIFICATION_SERVICE);
            ok = Build.VERSION.SDK_INT >= 27 && nm.isNotificationListenerAccessGranted(cn);
            if (!ok) {
                try {
                    NotificationManager.class.getMethod("setNotificationListenerAccessGranted", ComponentName.class, boolean.class, boolean.class).invoke(nm, cn, true, true);
                } catch (NoSuchMethodException e) {
                    NotificationManager.class.getMethod("setNotificationListenerAccessGranted", ComponentName.class, boolean.class).invoke(nm, cn, true);
                }
                ok = true;
                Log.i(TAG, "listener access granted by self");
            }
        } catch (Throwable e) { Log.w(TAG, "grant: " + e + (e.getCause() != null ? " / " + e.getCause() : "")); }
        if (ok) return;
        try {
            NotifService s = new NotifService();
            NotificationListenerService.class.getMethod("registerAsSystemService", Context.class, ComponentName.class, int.class)
                .invoke(s, app, cn, android.os.Process.myUserHandle().hashCode());
            Log.i(TAG, "registered as system listener");
        } catch (Throwable e) { Log.w(TAG, "system listener: " + e + (e.getCause() != null ? " / " + e.getCause() : "")); }
    }

    static void connected(NotificationListenerService s) {
        svc = s;
        if (app == null) app = s.getApplicationContext() != null ? s.getApplicationContext() : s;
        items.clear();
        try {
            NotificationListenerService.RankingMap rm = s.getCurrentRanking();
            StatusBarNotification[] all = s.getActiveNotifications();
            if (all != null) for (StatusBarNotification n : all) if (keep(n, rm)) items.add(n);
            Collections.sort(items, (a, b) -> Long.compare(b.getPostTime(), a.getPostTime()));
        } catch (Throwable e) { Log.w(TAG, "load: " + e); }
        changed();
        try { StatusBar.sync(app); } catch (Throwable ignored) {}
    }
    static void disconnected(NotificationListenerService s) {
        if (svc != s) return;
        svc = null;
        items.clear();
        changed();
    }

    // ================= 过滤 =================
    static CharSequence title(StatusBarNotification s) {
        Bundle x = s.getNotification().extras;
        return x == null ? null : x.getCharSequence(Notification.EXTRA_TITLE);
    }
    static CharSequence body(StatusBarNotification s) {
        Bundle x = s.getNotification().extras;
        if (x == null) return null;
        CharSequence t = x.getCharSequence(Notification.EXTRA_TEXT);
        return TextUtils.isEmpty(t) ? x.getCharSequence(Notification.EXTRA_BIG_TEXT) : t;
    }
    static int importance(StatusBarNotification s, NotificationListenerService.RankingMap rm, NotificationListenerService.Ranking out) {
        try { if (rm != null && rm.getRanking(s.getKey(), out)) return out.getImportance(); } catch (Throwable ignored) {}
        return NotificationManager.IMPORTANCE_DEFAULT;
    }
    static boolean keep(StatusBarNotification s, NotificationListenerService.RankingMap rm) {
        Notification n = s.getNotification();
        if (n == null || s.isOngoing() || !s.isClearable()) return false;
        if ((n.flags & (Notification.FLAG_FOREGROUND_SERVICE | Notification.FLAG_GROUP_SUMMARY)) != 0) return false;
        if (app != null && app.getPackageName().equals(s.getPackageName())) return false;
        if (TextUtils.isEmpty(title(s)) && TextUtils.isEmpty(body(s))) return false;
        return importance(s, rm, new NotificationListenerService.Ranking()) >= NotificationManager.IMPORTANCE_LOW;
    }
    static int indexOf(String key) {
        for (int i = 0; i < items.size(); i++) if (items.get(i).getKey().equals(key)) return i;
        return -1;
    }

    // ================= 系统回调 =================
    static void posted(StatusBarNotification s, NotificationListenerService.RankingMap rm) {
        if (s == null) return;
        int i = indexOf(s.getKey());
        StatusBarNotification old = i >= 0 ? items.remove(i) : null;
        if (!keep(s, rm)) {
            dropCard(s.getKey());
            if (old != null) changed();
            return;
        }
        items.add(0, s);
        while (items.size() > 60) items.remove(items.size() - 1);
        changed();
        NotificationListenerService.Ranking r = new NotificationListenerService.Ranking();
        boolean loud = importance(s, rm, r) >= NotificationManager.IMPORTANCE_DEFAULT;
        try { if (rm != null && rm.getRanking(s.getKey(), r) && !r.matchesInterruptionFilter()) loud = false; } catch (Throwable ignored) {}
        boolean fresh = old == null || ((s.getNotification().flags & Notification.FLAG_ONLY_ALERT_ONCE) == 0
            && !(TextUtils.equals(title(old), title(s)) && TextUtils.equals(body(old), body(s))));
        if (loud && fresh && !QuickPanel.showing()) {
            try { popup(s); } catch (Throwable e) { Log.w(TAG, "popup: " + e); }
        }
    }
    static void removed(StatusBarNotification s) {
        if (s == null) return;
        dropCard(s.getKey());
        int i = indexOf(s.getKey());
        if (i >= 0) { items.remove(i); changed(); }
    }
    static void changed() {
        try { if (Tray.it != null) Tray.it.paint(); } catch (Throwable ignored) {}
        try { QuickPanel.onNotifs(); } catch (Throwable ignored) {}
    }

    // ================= 动作 =================
    /** 点通知：触发它自带的意图；没有就打开那个应用。 */
    static void open(StatusBarNotification s) {
        Notification n = s.getNotification();
        try {
            PendingIntent pi = n.contentIntent != null ? n.contentIntent : n.fullScreenIntent;
            if (pi != null) {
                Bundle o = null;
                if (Build.VERSION.SDK_INT >= 34) {
                    ActivityOptions ao = ActivityOptions.makeBasic();
                    ao.setPendingIntentBackgroundActivityStartMode(ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED);
                    o = ao.toBundle();
                }
                pi.send(app, 0, null, null, null, null, o);
            } else {
                Intent li = app.getPackageManager().getLaunchIntentForPackage(s.getPackageName());
                if (li != null) app.startActivity(li.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
            }
        } catch (Throwable e) { Log.w(TAG, "open: " + e); }
        if ((n.flags & Notification.FLAG_AUTO_CANCEL) != 0) cancel(s);
        else dropCard(s.getKey());
    }
    static void cancel(StatusBarNotification s) {
        try { if (svc != null) svc.cancelNotification(s.getKey()); } catch (Throwable e) { Log.w(TAG, "cancel: " + e); }
        dropCard(s.getKey());
        int i = indexOf(s.getKey());
        if (i >= 0) { items.remove(i); changed(); }
    }
    static void clearAll() {
        try { if (svc != null) svc.cancelAllNotifications(); } catch (Throwable e) { Log.w(TAG, "clear: " + e); }
        items.clear();
        hidePopups();
        changed();
    }
    static CharSequence appLabel(String pkg) {
        try { return app.getPackageManager().getApplicationLabel(app.getPackageManager().getApplicationInfo(pkg, 0)); }
        catch (Throwable e) { return pkg; }
    }

    // ================= 卡片视图（弹出卡片与面板列表共用） =================
    static View card(Context c, final StatusBarNotification s, boolean inPanel) {
        LinearLayout row = new LinearLayout(c);
        row.setGravity(Gravity.CENTER_VERTICAL);
        row.setPadding(T.px(12), T.px(10), T.px(6), T.px(10));
        if (inPanel) row.setBackground(new android.graphics.drawable.LayerDrawable(new android.graphics.drawable.Drawable[]{T.round(T.chip, T.px(14)), T.state(T.px(14))}));
        else {
            row.setBackground(new android.graphics.drawable.LayerDrawable(new android.graphics.drawable.Drawable[]{T.round(T.panel | 0xFF000000, T.px(14)), T.state(T.px(14))}));
            row.setElevation(T.px(8));
        }
        ImageView iv = new ImageView(c);
        try { iv.setImageDrawable(app.getPackageManager().getApplicationIcon(s.getPackageName())); } catch (Throwable ignored) {}
        LinearLayout.LayoutParams il = new LinearLayout.LayoutParams(T.px(30), T.px(30));
        il.rightMargin = T.px(10);
        row.addView(iv, il);

        LinearLayout col = new LinearLayout(c);
        col.setOrientation(LinearLayout.VERTICAL);
        CharSequence when = android.text.format.DateFormat.format("HH:mm", new Date(s.getPostTime()));
        TextView from = Tray.text(c, appLabel(s.getPackageName()) + " · " + when, 10.5f, T.text2);
        from.setSingleLine(true); from.setEllipsize(TextUtils.TruncateAt.END);
        col.addView(from);
        CharSequence tt = title(s), bd = body(s);
        if (!TextUtils.isEmpty(tt)) {
            TextView t = Tray.text(c, tt.toString(), 13, T.text);
            t.setTypeface(Typeface.DEFAULT_BOLD);
            t.setSingleLine(true); t.setEllipsize(TextUtils.TruncateAt.END);
            t.setPadding(0, T.px(3), 0, 0);
            col.addView(t);
        }
        if (!TextUtils.isEmpty(bd)) {
            TextView t = Tray.text(c, bd.toString(), 12, T.text);
            t.setMaxLines(inPanel ? 2 : 3); t.setEllipsize(TextUtils.TruncateAt.END);
            t.setPadding(0, T.px(3), 0, 0);
            col.addView(t);
        }
        row.addView(col, new LinearLayout.LayoutParams(0, -2, 1));

        ImageView x = new ImageView(c);
        x.setImageDrawable(new Tray.Icon(Tray.Icon.CLOSE, T.text2));
        x.setBackground(T.state(T.px(14)));
        x.setContentDescription(L.t("清除这条通知", "Убрать уведомление", "Dismiss notification"));
        x.setOnClickListener(v -> { try { cancel(s); } catch (Throwable e) { Log.w(TAG, "x: " + e); } });
        row.addView(x, new LinearLayout.LayoutParams(T.px(28), T.px(28)));

        row.setOnClickListener(v -> { try { QuickPanel.hide(); open(s); } catch (Throwable e) { Log.w(TAG, "click: " + e); } });
        return row;
    }

    // ================= 弹出卡片 =================
    static FrameLayout popRoot;
    static LinearLayout stack;
    static boolean hovering;
    static final Map<String, View> cards = new LinkedHashMap<>();
    static final Map<String, Runnable> timers = new HashMap<>();

    /** 整个卡片窗口的悬停状态：鼠标停在上面时不自动消失。 */
    static final class PopRoot extends FrameLayout {
        PopRoot(Context c) { super(c); }
        @Override protected boolean dispatchHoverEvent(MotionEvent e) {
            try {
                int a = e.getActionMasked();
                if (a == MotionEvent.ACTION_HOVER_ENTER || a == MotionEvent.ACTION_HOVER_MOVE) {
                    if (!hovering) { hovering = true; for (Runnable r : timers.values()) ui.removeCallbacks(r); }
                } else if (a == MotionEvent.ACTION_HOVER_EXIT) {
                    hovering = false;
                    for (Runnable r : timers.values()) { ui.removeCallbacks(r); ui.postDelayed(r, AFTER_HOVER_MS); }
                }
            } catch (Throwable ignored) {}
            return super.dispatchHoverEvent(e);
        }
    }

    static void popup(final StatusBarNotification s) {
        if (app == null) return;
        Tray.ensureTheme(app);
        WindowManager wm = (WindowManager) app.getSystemService(Context.WINDOW_SERVICE);
        if (popRoot == null) {
            PopRoot r = new PopRoot(app);
            r.setPadding(T.px(10), T.px(10), T.px(10), T.px(8));
            r.setClipToPadding(false);
            LinearLayout st = new LinearLayout(app);
            st.setOrientation(LinearLayout.VERTICAL);
            st.setClipToPadding(false); st.setClipChildren(false);
            r.addView(st, new FrameLayout.LayoutParams(-1, -2));
            WindowManager.LayoutParams lp = new WindowManager.LayoutParams(T.px(360), -2, 2024 /* TYPE_NAVIGATION_BAR_PANEL，与底栏同层 */,
                WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE | WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL | WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN,
                PixelFormat.TRANSLUCENT);
            lp.gravity = Gravity.BOTTOM | Gravity.RIGHT;
            lp.y = Tray.shelfHeight() + Send.stackHeight();
            lp.setTitle("OpenDesktopNotifications");
            if (Build.VERSION.SDK_INT >= 30) lp.setFitInsetsTypes(0);
            wm.addView(r, lp);
            popRoot = r; stack = st; hovering = false;
        }
        final String key = s.getKey();
        View old = cards.remove(key);
        if (old != null) stack.removeView(old);
        while (cards.size() >= MAX_CARDS) dropCard(cards.keySet().iterator().next()); // 最旧的让位（至少还剩两张，窗口不会被关掉）
        View card = card(app, s, false);
        LinearLayout.LayoutParams cl = new LinearLayout.LayoutParams(-1, -2);
        cl.topMargin = T.px(8);
        stack.addView(card, cl);
        cards.put(key, card);
        card.setAlpha(0f); card.setTranslationX(T.px(24));
        card.animate().alpha(1f).translationX(0).setDuration(160).start();
        Runnable prev = timers.remove(key);
        if (prev != null) ui.removeCallbacks(prev);
        Runnable t = () -> dropCard(key);
        timers.put(key, t);
        if (!hovering) ui.postDelayed(t, SHOW_MS);
    }
    /** 传输卡片的高度变了：把通知卡片挪到它上方。 */
    static void place() {
        FrameLayout r = popRoot;
        if (r == null || app == null) return;
        try {
            WindowManager.LayoutParams lp = (WindowManager.LayoutParams) r.getLayoutParams();
            int y = Tray.shelfHeight() + Send.stackHeight();
            if (lp.y == y) return;
            lp.y = y;
            ((WindowManager) app.getSystemService(Context.WINDOW_SERVICE)).updateViewLayout(r, lp);
        } catch (Throwable e) { Log.w(TAG, "place: " + e); }
    }
    static void dropCard(String key) {
        Runnable t = timers.remove(key);
        if (t != null) ui.removeCallbacks(t);
        View v = cards.remove(key);
        if (v != null && stack != null) stack.removeView(v);
        if (cards.isEmpty()) closePop();
    }
    static void hidePopups() {
        for (Runnable r : timers.values()) ui.removeCallbacks(r);
        timers.clear(); cards.clear();
        closePop();
    }
    static void closePop() {
        FrameLayout r = popRoot;
        popRoot = null; stack = null; hovering = false;
        if (r == null || app == null) return;
        try { ((WindowManager) app.getSystemService(Context.WINDOW_SERVICE)).removeViewImmediate(r); } catch (Throwable e) { Log.w(TAG, "close: " + e); }
    }
}
