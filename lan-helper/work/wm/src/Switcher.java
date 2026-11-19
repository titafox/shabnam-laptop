package local.cloud.launcher;

import android.content.Context;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageManager;
import android.graphics.PixelFormat;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.LayerDrawable;
import android.text.TextUtils;
import android.util.Log;
import android.view.Gravity;
import android.view.View;
import android.view.WindowManager;
import android.widget.*;
import java.util.ArrayList;
import java.util.List;

/** Alt+Tab 切换面板：屏幕中央一排正在运行的窗口（图标 + 名称），松开 Alt 切到选中的那个。 */
final class Switcher {
    static final String TAG = "Switcher";
    static WindowManager wm;
    static View root;
    static List<Wm.Tk> list = new ArrayList<>();
    static List<View> cells = new ArrayList<>();
    static HorizontalScrollView scroll;
    static int sel;

    static boolean showing() { return root != null; }

    /** 第一次调用弹出面板，之后每次移动选中项。 */
    static void step(Context c, int dir) {
        try {
            if (root == null) {
                if (!show(c.getApplicationContext())) return;
                boolean topIsApp = !list.isEmpty() && list.get(0).focused && list.get(0).visible;
                if (dir > 0) sel = topIsApp && list.size() > 1 ? 1 : 0;
                else sel = list.size() - 1;
            } else sel = ((sel + dir) % list.size() + list.size()) % list.size();
            mark();
        } catch (Throwable e) { Log.w(TAG, "step: " + e); hide(); }
    }
    static void commit() {
        try {
            if (root == null) return;
            int id = sel >= 0 && sel < list.size() ? list.get(sel).id : -1;
            hide();
            if (id >= 0) Wm.front(id);
        } catch (Throwable e) { Log.w(TAG, "commit: " + e); hide(); }
    }
    static void cancel() { hide(); }

    static void hide() {
        try { if (root != null) wm.removeViewImmediate(root); } catch (Throwable ignored) {}
        root = null; scroll = null;
        cells = new ArrayList<>();
    }

    static boolean show(Context c) {
        list = Wm.appTasks();
        if (list.isEmpty()) {
            Toast.makeText(c, L.t("没有打开的窗口", "Нет открытых окон", "No open windows"), Toast.LENGTH_SHORT).show();
            return false;
        }
        if (T.panel == 0) T.apply(true);
        wm = (WindowManager) c.getSystemService(Context.WINDOW_SERVICE);
        LinearLayout row = new LinearLayout(c);
        row.setPadding(T.px(10), T.px(10), T.px(10), T.px(10));
        cells = new ArrayList<>();
        PackageManager pm = c.getPackageManager();
        for (int i = 0; i < list.size(); i++) {
            final int idx = i;
            Wm.Tk k = list.get(i);
            LinearLayout cell = new LinearLayout(c);
            cell.setOrientation(LinearLayout.VERTICAL);
            cell.setGravity(Gravity.CENTER_HORIZONTAL);
            cell.setPadding(T.px(8), T.px(10), T.px(8), T.px(8));
            ImageView iv = new ImageView(c);
            CharSequence label = k.pkg;
            Apps.App a = null;
            if (Main.apps != null) for (Apps.App y : Main.apps.all) if (y.pkg.equals(k.pkg)) { a = y; break; }
            try {
                if (a != null && a.icon != null) { iv.setImageBitmap(a.icon); label = a.label; }
                else {
                    ApplicationInfo ai = pm.getApplicationInfo(k.pkg, 0);
                    iv.setImageDrawable(pm.getApplicationIcon(ai));
                    label = pm.getApplicationLabel(ai);
                }
            } catch (Throwable ignored) {}
            cell.addView(iv, new LinearLayout.LayoutParams(T.px(48), T.px(48)));
            TextView tv = new TextView(c);
            tv.setText(label);
            tv.setTextColor(T.text);
            tv.setTextSize(android.util.TypedValue.COMPLEX_UNIT_PX, T.px(12));
            tv.setSingleLine(true);
            tv.setEllipsize(TextUtils.TruncateAt.END);
            tv.setGravity(Gravity.CENTER);
            LinearLayout.LayoutParams tl = new LinearLayout.LayoutParams(-1, -2);
            tl.topMargin = T.px(6);
            cell.addView(tv, tl);
            cell.setContentDescription(label);
            cell.setOnClickListener(v -> { sel = idx; commit(); });
            LinearLayout.LayoutParams cl = new LinearLayout.LayoutParams(T.px(104), -2);
            cl.leftMargin = cl.rightMargin = T.px(3);
            row.addView(cell, cl);
            cells.add(cell);
        }
        scroll = new HorizontalScrollView(c);
        scroll.setHorizontalScrollBarEnabled(false);
        scroll.addView(row);
        FrameLayout box = new FrameLayout(c);
        box.setBackground(T.round(T.panel | 0xFF000000, T.px(18)));
        box.addView(scroll, new FrameLayout.LayoutParams(-2, -2));

        WindowManager.LayoutParams lp = new WindowManager.LayoutParams(-2, -2, 2038 /* TYPE_APPLICATION_OVERLAY */,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE | WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL
                | WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN | WindowManager.LayoutParams.FLAG_DIM_BEHIND,
            PixelFormat.TRANSLUCENT);
        lp.dimAmount = 0.25f;
        lp.gravity = Gravity.CENTER;
        lp.setTitle("OpenDesktopSwitcher");
        lp.windowAnimations = 0;
        try { wm.addView(box, lp); }
        catch (Throwable e) {
            lp.type = 2024; // 没有悬浮窗许可时退回系统面板类型（与底栏同一类）
            wm.addView(box, lp);
        }
        root = box;
        return true;
    }

    static void mark() {
        for (int i = 0; i < cells.size(); i++) {
            View v = cells.get(i);
            v.setBackground(i == sel
                ? new LayerDrawable(new Drawable[]{T.round(T.hover, T.px(12)), T.round(T.chip, T.px(12))})
                : T.state(T.px(12)));
        }
        if (scroll != null && sel >= 0 && sel < cells.size()) {
            final View v = cells.get(sel);
            scroll.post(() -> { try { scroll.smoothScrollTo(Math.max(0, v.getLeft() - T.px(200)), 0); } catch (Throwable ignored) {} });
        }
    }
}
