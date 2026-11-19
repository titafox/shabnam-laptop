package local.cloud.launcher;

import android.app.Instrumentation;
import android.content.ClipData;
import android.content.ClipDescription;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.SharedPreferences;
import android.graphics.PixelFormat;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.LayerDrawable;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.text.TextUtils;
import android.util.Log;
import android.util.TypedValue;
import android.view.*;
import android.widget.*;
import org.json.JSONArray;
import org.json.JSONObject;
import java.util.ArrayList;
import java.util.List;

/**
 * 剪贴板历史：记住最近 30 条文字，Clip.show(context) 弹出选择窗（再调一次则收起）。
 *
 * 安卓 10 起，应用不在前台就读不到剪贴板。这里靠平台签名拿到的两项权限解决：
 *   READ_CLIPBOARD_IN_BACKGROUND —— 在后台也能收到「剪贴板变了」的通知并读取内容；
 *   SUPPRESS_CLIPBOARD_ACCESS_NOTIFICATION —— 读的时候系统不弹「已从剪贴板粘贴」的提示。
 * 万一权限没拿到，还有一道保底：选择窗自己拿到焦点的那一刻再读一次（有焦点的窗口总是允许读），
 * 这样至少「当前这一条」会进历史。
 * 标记为敏感内容（密码管理器会打这个标记）的剪贴不记录。
 */
final class Clip {
    static final String TAG = "Clip";
    static final int MAX = 30, MAX_CHARS = 20000;
    /** 选中一条后是否顺手往当前窗口发一次 Ctrl+V。 */
    static boolean autoPaste = true;

    static final class Item { String text; long time; }

    static final List<Item> items = new ArrayList<>(); // 新的在前
    static final Handler ui = new Handler(Looper.getMainLooper());
    static Context app;
    static ClipboardManager cm;
    static boolean inited;
    static Panel win;
    static int sel;

    // ================= 记录 =================
    /** 可以重复调用。进程一启动就由 FilesProvider 调一次。 */
    static void init(Context c) {
        if (inited || c == null) return;
        inited = true;
        app = c.getApplicationContext() != null ? c.getApplicationContext() : c;
        ui.post(() -> {
            try {
                load();
                cm = (ClipboardManager) app.getSystemService(Context.CLIPBOARD_SERVICE);
                cm.addPrimaryClipChangedListener(() -> { try { grab(); } catch (Throwable e) { Log.w(TAG, "changed: " + e); } });
                grab();
            } catch (Throwable e) { Log.w(TAG, "init: " + e); }
        });
    }
    static SharedPreferences prefs() { return Store.ctx(app).getSharedPreferences("clip", 0); }
    static void load() {
        try {
            JSONArray a = new JSONArray(prefs().getString("items", "[]"));
            items.clear();
            for (int i = 0; i < a.length() && i < MAX; i++) {
                JSONObject o = a.getJSONObject(i);
                Item it = new Item();
                it.text = o.getString("t"); it.time = o.optLong("m", 0);
                if (!it.text.isEmpty()) items.add(it);
            }
        } catch (Throwable e) { Log.w(TAG, "load: " + e); }
    }
    static void save() {
        try {
            JSONArray a = new JSONArray();
            for (Item it : items) a.put(new JSONObject().put("t", it.text).put("m", it.time));
            prefs().edit().putString("items", a.toString()).apply();
        } catch (Throwable e) { Log.w(TAG, "save: " + e); }
    }
    /** 读当前剪贴板，是文字就记进历史。读不到（没有权限也没有焦点）就什么都不做。 */
    static void grab() {
        if (cm == null) return;
        ClipData d;
        try { d = cm.getPrimaryClip(); } catch (Throwable e) { Log.w(TAG, "read: " + e); return; }
        if (d == null || d.getItemCount() == 0) return;
        ClipDescription desc = d.getDescription();
        try {
            if (Build.VERSION.SDK_INT >= 24 && desc != null && desc.getExtras() != null
                && desc.getExtras().getBoolean("android.content.extra.IS_SENSITIVE", false)) return;
        } catch (Throwable ignored) {}
        CharSequence cs = d.getItemAt(0).getText();
        if (cs == null) return;
        String s = cs.toString();
        if (s.trim().isEmpty()) return;
        if (s.length() > MAX_CHARS) s = s.substring(0, MAX_CHARS);
        add(s);
    }
    static void add(String s) {
        for (int i = 0; i < items.size(); i++) if (items.get(i).text.equals(s)) {
            if (i == 0) return;
            items.remove(i);
            break;
        }
        Item it = new Item();
        it.text = s; it.time = System.currentTimeMillis();
        items.add(0, it);
        while (items.size() > MAX) items.remove(items.size() - 1);
        save();
        if (win != null) { sel = 0; win.adapter.notifyDataSetChanged(); win.refresh(); }
    }

    // ================= 选择窗 =================
    /** 弹出 / 收起剪贴板历史。可以在任何线程调用。 */
    static void show(final Context c) {
        ui.post(() -> {
            try {
                if (win != null) { hide(); return; }
                init(c);
                if (app == null) return;
                if (T.text == 0) T.apply(Store.ctx(app).getSharedPreferences("desk", 0).getBoolean("dark", true));
                if (items.isEmpty()) load();
                try { cm = (ClipboardManager) app.getSystemService(Context.CLIPBOARD_SERVICE); } catch (Throwable ignored) {}
                Panel p = new Panel(app);
                WindowManager wm = (WindowManager) app.getSystemService(Context.WINDOW_SERVICE);
                WindowManager.LayoutParams lp = new WindowManager.LayoutParams(-1, -1,
                    Build.VERSION.SDK_INT >= 26 ? WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY : WindowManager.LayoutParams.TYPE_SYSTEM_ALERT,
                    WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN | WindowManager.LayoutParams.FLAG_DIM_BEHIND, PixelFormat.TRANSLUCENT);
                lp.dimAmount = 0.28f;
                lp.softInputMode = WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_HIDDEN;
                lp.setTitle("OpenDesktopClipboard");
                wm.addView(p, lp);
                win = p;
                sel = 0;
                p.requestFocus();
            } catch (Throwable e) {
                Log.w(TAG, "show: " + e);
                win = null;
                try { Toast.makeText(c, L.t("无法显示剪贴板历史（需要「显示在其他应用上层」权限）", "Не удалось показать историю буфера (нужно разрешение «Поверх других приложений»)", "Can't show clipboard history (needs the \"Display over other apps\" permission)"), Toast.LENGTH_LONG).show(); } catch (Throwable ignored) {}
            }
        });
    }
    static void hide() {
        Panel p = win;
        win = null;
        if (p == null) return;
        try { ((WindowManager) app.getSystemService(Context.WINDOW_SERVICE)).removeViewImmediate(p); } catch (Throwable e) { Log.w(TAG, "hide: " + e); }
    }
    static void choose(int i) {
        if (i < 0 || i >= items.size()) return;
        final String s = items.get(i).text;
        hide();
        try { cm.setPrimaryClip(ClipData.newPlainText("", s)); } catch (Throwable e) { Log.w(TAG, "set: " + e); return; }
        add(s);
        if (!autoPaste) return;
        // 等选择窗消失、焦点回到原来的窗口，再替用户按一次 Ctrl+V
        new Thread(() -> {
            try {
                Thread.sleep(220);
                Instrumentation in = new Instrumentation();
                long t = SystemClock.uptimeMillis();
                int meta = KeyEvent.META_CTRL_ON | KeyEvent.META_CTRL_LEFT_ON;
                in.sendKeySync(new KeyEvent(t, t, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_CTRL_LEFT, 0, meta));
                in.sendKeySync(new KeyEvent(t, t, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_V, 0, meta));
                in.sendKeySync(new KeyEvent(t, SystemClock.uptimeMillis(), KeyEvent.ACTION_UP, KeyEvent.KEYCODE_V, 0, meta));
                in.sendKeySync(new KeyEvent(t, SystemClock.uptimeMillis(), KeyEvent.ACTION_UP, KeyEvent.KEYCODE_CTRL_LEFT, 0, 0));
            } catch (Throwable e) { Log.w(TAG, "paste: " + e); }
        }, "clip-paste").start();
    }
    static void remove(int i) {
        if (i < 0 || i >= items.size()) return;
        items.remove(i);
        save();
        if (win != null) { sel = Math.max(0, Math.min(sel, items.size() - 1)); win.adapter.notifyDataSetChanged(); win.refresh(); }
    }
    static String ago(long t) {
        long d = (System.currentTimeMillis() - t) / 1000;
        if (t <= 0 || d < 0) return "";
        if (d < 60) return L.t("刚刚", "только что", "just now");
        if (d < 3600) return d / 60 + L.t(" 分钟前", " мин назад", " min ago");
        if (d < 86400) return d / 3600 + L.t(" 小时前", " ч назад", " h ago");
        return d / 86400 + L.t(" 天前", " дн. назад", " d ago");
    }

    static final class Panel extends FrameLayout {
        final ListView list;
        final TextView empty;
        final BaseAdapter adapter;

        Panel(final Context c) {
            super(c);
            setFocusable(true);
            setFocusableInTouchMode(true);
            setOnClickListener(v -> hide());

            LinearLayout p = new LinearLayout(c);
            p.setOrientation(LinearLayout.VERTICAL);
            p.setBackground(T.round(T.panel | 0xFF000000, T.px(18)));
            p.setElevation(T.px(12));
            p.setPadding(T.px(14), T.px(14), T.px(14), T.px(10));
            p.setClickable(true);
            FrameLayout.LayoutParams pl = new FrameLayout.LayoutParams(T.px(460), T.px(520), Gravity.CENTER);
            pl.leftMargin = pl.rightMargin = T.px(16);
            pl.topMargin = pl.bottomMargin = T.px(16);
            addView(p, pl);

            LinearLayout head = new LinearLayout(c);
            head.setGravity(Gravity.CENTER_VERTICAL);
            TextView title = tv(c, L.t("剪贴板", "Буфер обмена", "Clipboard"), 15, T.text);
            title.setPadding(T.px(6), 0, 0, 0);
            head.addView(title, new LinearLayout.LayoutParams(0, -2, 1));
            TextView clear = tv(c, L.t("清空", "Очистить", "Clear all"), 12, T.text);
            clear.setPadding(T.px(12), T.px(7), T.px(12), T.px(7));
            clear.setBackground(new LayerDrawable(new Drawable[]{T.round(T.chip, T.px(14)), T.state(T.px(14))}));
            clear.setOnClickListener(v -> { try { items.clear(); save(); sel = 0; refreshAll(); } catch (Throwable e) { Log.w(TAG, "clear: " + e); } });
            clear.setFocusable(false);
            head.addView(clear);
            LinearLayout.LayoutParams hl = new LinearLayout.LayoutParams(-1, -2);
            hl.bottomMargin = T.px(8);
            p.addView(head, hl);

            FrameLayout body = new FrameLayout(c);
            p.addView(body, new LinearLayout.LayoutParams(-1, 0, 1));
            list = new ListView(c);
            list.setDivider(null);
            list.setSelector(new android.graphics.drawable.ColorDrawable(0));
            list.setFocusable(false);
            list.setItemsCanFocus(false);
            list.setVerticalScrollBarEnabled(false);
            list.setOverScrollMode(View.OVER_SCROLL_NEVER);
            list.setCacheColorHint(0);
            body.addView(list, new FrameLayout.LayoutParams(-1, -1));
            empty = tv(c, L.t("还没有复制过文字", "Вы ещё ничего не копировали", "Nothing copied yet"), 13, T.text2);
            empty.setGravity(Gravity.CENTER);
            body.addView(empty, new FrameLayout.LayoutParams(-1, -2, Gravity.CENTER));

            TextView hint = tv(c, L.t("↑↓ 选择 · Enter 粘贴 · Del 删除 · Esc 关闭", "↑↓ выбор · Enter вставить · Del удалить · Esc закрыть", "↑↓ select · Enter paste · Del remove · Esc close"), 11, T.text2);
            hint.setPadding(T.px(6), T.px(8), 0, 0);
            p.addView(hint);

            adapter = new BaseAdapter() {
                public int getCount() { return items.size(); }
                public Object getItem(int i) { return items.get(i); }
                public long getItemId(int i) { return i; }
                public View getView(final int pos, View v, ViewGroup parent) {
                    LinearLayout row = (LinearLayout) v;
                    if (row == null) {
                        row = new LinearLayout(c);
                        row.setGravity(Gravity.CENTER_VERTICAL);
                        row.setPadding(T.px(10), T.px(8), T.px(4), T.px(8));
                        row.setBackground(T.state(T.px(12)));
                        row.setFocusable(false);
                        TextView num = tv(c, "", 11, T.text2);
                        row.addView(num, new LinearLayout.LayoutParams(T.px(20), -2));
                        LinearLayout mid = new LinearLayout(c);
                        mid.setOrientation(LinearLayout.VERTICAL);
                        TextView t = tv(c, "", 13.5f, T.text);
                        t.setMaxLines(2);
                        t.setEllipsize(TextUtils.TruncateAt.END);
                        mid.addView(t);
                        TextView when = tv(c, "", 10.5f, T.text2);
                        when.setPadding(0, T.px(3), 0, 0);
                        mid.addView(when);
                        row.addView(mid, new LinearLayout.LayoutParams(0, -2, 1));
                        ImageView x = new ImageView(c);
                        x.setImageDrawable(new FI.Glyph(FI.CLOSE, T.text2));
                        x.setPadding(T.px(8), T.px(8), T.px(8), T.px(8));
                        android.graphics.drawable.StateListDrawable xb = new android.graphics.drawable.StateListDrawable(); // 只响应悬停，不跟着整行高亮
                        xb.addState(new int[]{android.R.attr.state_hovered}, T.round(T.chip, T.px(15)));
                        xb.addState(new int[]{android.R.attr.state_pressed}, T.round(T.chip, T.px(15)));
                        xb.addState(new int[0], new android.graphics.drawable.ColorDrawable(0));
                        x.setBackground(xb);
                        x.setFocusable(false);
                        row.addView(x, new LinearLayout.LayoutParams(T.px(30), T.px(30)));
                    }
                    try {
                        Item it = items.get(pos);
                        LinearLayout mid = (LinearLayout) row.getChildAt(1);
                        ((TextView) row.getChildAt(0)).setText(pos < 9 ? String.valueOf(pos + 1) : "");
                        String s = it.text.length() > 300 ? it.text.substring(0, 300) : it.text;
                        ((TextView) mid.getChildAt(0)).setText(s.trim().replaceAll("\\s*\\n\\s*", " ⏎ "));
                        ((TextView) mid.getChildAt(1)).setText(ago(it.time) + (it.text.length() > 300 ? L.t(" · 共 ", " · всего ", " · ") + it.text.length() + L.t(" 字", " симв.", " chars") : ""));
                        row.setActivated(pos == sel);
                        row.setOnClickListener(v2 -> { try { choose(pos); } catch (Throwable e) { Log.w(TAG, "choose: " + e); } });
                        row.setOnContextClickListener(v2 -> { try { remove(pos); } catch (Throwable e) { Log.w(TAG, "remove: " + e); } return true; });
                        row.getChildAt(2).setOnClickListener(v2 -> { try { remove(pos); } catch (Throwable e) { Log.w(TAG, "remove: " + e); } });
                    } catch (Throwable e) { Log.w(TAG, "bind: " + e); }
                    return row;
                }
            };
            list.setAdapter(adapter);
            refresh();
        }
        static TextView tv(Context c, String s, float size, int color) {
            TextView t = new TextView(c);
            t.setText(s); t.setTextColor(color);
            t.setTextSize(TypedValue.COMPLEX_UNIT_PX, T.px(size));
            t.setIncludeFontPadding(false);
            return t;
        }
        void refresh() { empty.setVisibility(items.isEmpty() ? VISIBLE : GONE); }
        void refreshAll() { adapter.notifyDataSetChanged(); refresh(); }

        public void onWindowFocusChanged(boolean has) {
            super.onWindowFocusChanged(has);
            if (has) try { grab(); } catch (Throwable e) { Log.w(TAG, "focus grab: " + e); }
        }
        public boolean dispatchKeyEvent(KeyEvent e) {
            try {
                if (e.getAction() != KeyEvent.ACTION_DOWN) return true;
                int c = e.getKeyCode(), n = items.size();
                if (c == KeyEvent.KEYCODE_ESCAPE || c == KeyEvent.KEYCODE_BACK || c == KeyEvent.KEYCODE_V && e.isMetaPressed()) hide();
                else if (c == KeyEvent.KEYCODE_DPAD_DOWN || c == KeyEvent.KEYCODE_TAB) move(Math.min(n - 1, sel + 1));
                else if (c == KeyEvent.KEYCODE_DPAD_UP) move(Math.max(0, sel - 1));
                else if (c == KeyEvent.KEYCODE_MOVE_HOME) move(0);
                else if (c == KeyEvent.KEYCODE_MOVE_END) move(n - 1);
                else if (c == KeyEvent.KEYCODE_ENTER || c == KeyEvent.KEYCODE_NUMPAD_ENTER) { if (e.getRepeatCount() == 0) choose(sel); }
                else if (c == KeyEvent.KEYCODE_FORWARD_DEL || c == KeyEvent.KEYCODE_DEL) remove(sel);
                else if (c >= KeyEvent.KEYCODE_1 && c <= KeyEvent.KEYCODE_9 && !e.isMetaPressed() && !e.isCtrlPressed()) choose(c - KeyEvent.KEYCODE_1);
            } catch (Throwable t) { Log.w(TAG, "key: " + t); }
            return true;
        }
        void move(int to) {
            if (to < 0 || to >= items.size()) return;
            sel = to;
            adapter.notifyDataSetChanged();
            if (to <= list.getFirstVisiblePosition() || to >= list.getLastVisiblePosition()) list.setSelection(Math.max(0, to - 2));
        }
    }
}
