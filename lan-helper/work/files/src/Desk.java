package local.cloud.launcher;

import android.app.AlertDialog;
import android.bluetooth.BluetoothAdapter;
import android.content.*;
import android.content.res.ColorStateList;
import android.content.res.Configuration;
import android.graphics.*;
import android.graphics.drawable.*;
import android.media.AudioManager;
import android.net.*;
import android.os.*;
import android.provider.Settings;
import android.text.*;
import android.util.TypedValue;
import android.view.*;
import android.view.animation.DecelerateInterpolator;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputMethodManager;
import android.widget.*;
import java.io.*;
import java.util.*;

/** ChromeOS 风格桌面：壁纸、底栏、启动器面板、快捷设置面板。 */
final class Desk implements Apps.Listener {
    static final int COLS = 5, REQ_WALL = 43;
    final Main a;
    final Apps apps;
    final SharedPreferences sp;
    final Handler ui = new Handler(Looper.getMainLooper());
    int W, H;

    FrameLayout root;
    View scrim, launcher, quick, shelfView;
    LinearLayout pinRow;
    EditText search;
    ListView list;
    final RowAdapter adapter = new RowAdapter();
    final List<Row> rows = new ArrayList<>();
    final List<Object> items = new ArrayList<>(); // 可选中项：App 或网址字符串
    int sel = -1, recentCount = 0;
    boolean searching;

    TextView clock, batText, foot, chan, btSub, wifiSub, themeTitle, brightHint;
    View dot, batBox, btPill, wifiPill;
    T.Glyph netGlyph, batGlyph;
    ImageView netIcon, batIcon;
    SeekBar volume, bright;
    boolean resumed, hasBattery;
    int batLevel = -1;
    String lastChan = "";

    Desk(Main a, Apps apps) {
        this.a = a; this.apps = apps;
        sp = apps.sp;
        apps.listener = this;
    }

    // ================= 构建 =================
    View build() {
        android.util.DisplayMetrics dm = new android.util.DisplayMetrics();
        a.getWindowManager().getDefaultDisplay().getRealMetrics(dm);
        W = Math.max(dm.widthPixels, dm.heightPixels); H = Math.min(dm.widthPixels, dm.heightPixels);
        T.u = W / 1280f;
        T.apply(sp.getBoolean("dark", true));
        apps.iconPx = T.px(48);

        root = new FrameLayout(a);
        root.setBackgroundColor(T.base);
        root.setFocusable(true);
        root.setFocusableInTouchMode(true);

        scrim = new View(a);
        scrim.setVisibility(View.GONE);
        scrim.setOnClickListener(v -> hidePanels());
        root.addView(scrim, new FrameLayout.LayoutParams(-1, -1));

        buildShelf();
        buildLauncher();
        buildQuick();
        loadWallpaper();
        renderPins();
        refreshRows();
        return root;
    }

    TextView text(String s, float size, int color) {
        TextView t = new TextView(a);
        t.setText(s); t.setTextColor(color);
        t.setTextSize(TypedValue.COMPLEX_UNIT_PX, T.px(size));
        t.setIncludeFontPadding(false);
        return t;
    }

    void buildShelf() {
        FrameLayout shelf = new FrameLayout(a);
        shelf.setBackgroundColor(T.shelf);
        shelf.setClickable(true);
        shelfView = shelf;
        root.addView(shelf, new FrameLayout.LayoutParams(-1, T.px(52), Gravity.BOTTOM));

        ImageView home = new ImageView(a);
        home.setImageDrawable(new T.Glyph(T.Glyph.HOME, T.text));
        LayerDrawable hb = new LayerDrawable(new Drawable[]{T.round(T.chip, T.px(18)), T.state(T.px(18))});
        home.setBackground(hb);
        home.setContentDescription(L.t("启动器", "Меню приложений", "Launcher"));
        home.setOnClickListener(v -> toggle(launcher));
        FrameLayout.LayoutParams hl = new FrameLayout.LayoutParams(T.px(36), T.px(36), Gravity.LEFT | Gravity.CENTER_VERTICAL);
        hl.leftMargin = T.px(10);
        shelf.addView(home, hl);

        pinRow = new LinearLayout(a);
        pinRow.setGravity(Gravity.CENTER);
        shelf.addView(pinRow, new FrameLayout.LayoutParams(-2, -1, Gravity.CENTER));

        LinearLayout tray = new LinearLayout(a);
        tray.setGravity(Gravity.CENTER_VERTICAL);
        tray.setBackground(new LayerDrawable(new Drawable[]{T.round(T.chip, T.px(17)), T.state(T.px(17))}));
        tray.setPadding(T.px(12), 0, T.px(14), 0);
        tray.setOnClickListener(v -> toggle(quick));
        FrameLayout.LayoutParams tl = new FrameLayout.LayoutParams(-2, T.px(34), Gravity.RIGHT | Gravity.CENTER_VERTICAL);
        tl.rightMargin = T.px(10);
        shelf.addView(tray, tl);

        dot = new View(a);
        dot.setBackground(T.round(0xFF9AA0A6, T.px(4)));
        LinearLayout.LayoutParams dl = new LinearLayout.LayoutParams(T.px(7), T.px(7));
        dl.rightMargin = T.px(9);
        tray.addView(dot, dl);

        netIcon = new ImageView(a);
        netGlyph = new T.Glyph(T.Glyph.NET, T.text);
        netIcon.setImageDrawable(netGlyph);
        LinearLayout.LayoutParams nl = new LinearLayout.LayoutParams(T.px(15), T.px(15));
        nl.rightMargin = T.px(9);
        tray.addView(netIcon, nl);

        LinearLayout bb = new LinearLayout(a);
        bb.setGravity(Gravity.CENTER_VERTICAL);
        batIcon = new ImageView(a);
        batGlyph = new T.Glyph(T.Glyph.BATTERY, T.text);
        batIcon.setImageDrawable(batGlyph);
        bb.addView(batIcon, new LinearLayout.LayoutParams(T.px(20), T.px(15)));
        batText = text("", 12, T.text);
        batText.setPadding(T.px(4), 0, T.px(9), 0);
        bb.addView(batText);
        bb.setVisibility(View.GONE);
        batBox = bb;
        tray.addView(bb);

        clock = text("", 13, T.text);
        tray.addView(clock);
    }

    void buildLauncher() {
        LinearLayout p = new LinearLayout(a);
        p.setOrientation(LinearLayout.VERTICAL);
        p.setBackground(T.round(T.panel, T.px(18)));
        p.setElevation(T.px(10));
        p.setPadding(T.px(14), T.px(14), T.px(14), T.px(8));
        p.setClickable(true);
        p.setVisibility(View.GONE);
        FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(Math.round(W * 0.36f), Math.round(H * 0.75f), Gravity.BOTTOM | Gravity.LEFT);
        lp.leftMargin = T.px(10); lp.bottomMargin = T.px(60);
        root.addView(p, lp);
        launcher = p;

        search = new EditText(a);
        search.setHint(L.t("搜索应用、文件、设置、网址…", "Поиск приложений, файлов, настроек, сайтов…", "Search apps, files, settings, web…"));
        search.setHintTextColor(T.text2);
        search.setTextColor(T.text);
        search.setTextSize(TypedValue.COMPLEX_UNIT_PX, T.px(14));
        search.setSingleLine(true);
        search.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
        search.setImeOptions(EditorInfo.IME_ACTION_GO | EditorInfo.IME_FLAG_NO_EXTRACT_UI);
        search.setBackground(T.round(T.chip, T.px(20)));
        search.setPadding(T.px(16), 0, T.px(16), 0);
        search.setGravity(Gravity.CENTER_VERTICAL);
        LinearLayout.LayoutParams sl = new LinearLayout.LayoutParams(-1, T.px(40));
        sl.bottomMargin = T.px(8);
        p.addView(search, sl);
        search.addTextChangedListener(new TextWatcher() {
            public void beforeTextChanged(CharSequence s, int x, int y, int z) {}
            public void onTextChanged(CharSequence s, int x, int y, int z) {}
            public void afterTextChanged(Editable e) { refreshRows(); }
        });
        search.setOnKeyListener((v, code, ev) -> ev.getAction() == KeyEvent.ACTION_DOWN && panelKey(code));
        search.setOnEditorActionListener((v, id, ev) -> { activate(); return true; });

        list = new ListView(a);
        list.setDivider(null);
        list.setSelector(new ColorDrawable(0));
        list.setFocusable(false);
        list.setItemsCanFocus(false);
        list.setVerticalScrollBarEnabled(false);
        list.setOverScrollMode(View.OVER_SCROLL_NEVER);
        list.setCacheColorHint(0);
        list.setAdapter(adapter);
        p.addView(list, new LinearLayout.LayoutParams(-1, 0, 1));
    }

    // ================= 启动器内容 =================
    static final class Row {
        int type; // 0 标题 1 网格行 2 结果行
        String text;
        int base, count;
    }
    static boolean looksLikeUrl(String q) {
        if (q.contains(" ")) return false;
        return q.startsWith("http://") || q.startsWith("https://") || (q.indexOf('.') > 0 && q.indexOf('.') < q.length() - 1);
    }
    void addGrid(int from, int n) {
        for (int i = 0; i < n; i += COLS) {
            Row r = new Row(); r.type = 1; r.base = from + i; r.count = Math.min(COLS, n - i);
            rows.add(r);
        }
    }
    void label(String s) { Row r = new Row(); r.type = 0; r.text = s; rows.add(r); }

    void refreshRows() {
        if (list == null) return;
        rows.clear(); items.clear();
        String q = search.getText().toString().trim();
        searching = !q.isEmpty();
        if (!searching) {
            List<Apps.App> rec = apps.recents(COLS), all = apps.visible();
            recentCount = rec.size();
            items.addAll(rec); items.addAll(all);
            if (!rec.isEmpty()) { label(L.t("最近使用", "Недавние", "Recent")); addGrid(0, rec.size()); }
            label(apps.all.isEmpty() ? L.t("正在读取应用…", "Загрузка приложений…", "Loading apps…") : L.t("全部应用", "Все приложения", "All apps"));
            addGrid(rec.size(), all.size());
            sel = -1;
        } else {
            if (looksLikeUrl(q)) items.add(q);
            items.addAll(apps.search(q));
            try { Find.add(this, q); } catch (Throwable ignored) {} // 统一搜索：设置入口立即加入，文件稍后追加
            if (items.isEmpty()) label(L.t("没有匹配的应用", "Ничего не найдено", "No matching apps"));
            for (int i = 0; i < items.size(); i++) { Row r = new Row(); r.type = 2; r.base = i; r.count = 1; rows.add(r); }
            sel = items.isEmpty() ? -1 : 0;
        }
        adapter.notifyDataSetChanged();
        if (searching) list.setSelection(0);
    }

    final class RowAdapter extends BaseAdapter {
        public int getCount() { return rows.size(); }
        public Object getItem(int i) { return rows.get(i); }
        public long getItemId(int i) { return i; }
        public int getViewTypeCount() { return 3; }
        public int getItemViewType(int i) { return rows.get(i).type; }
        public boolean areAllItemsEnabled() { return false; }
        public boolean isEnabled(int i) { return false; }

        public View getView(int pos, View v, ViewGroup parent) {
            Row r = rows.get(pos);
            if (r.type == 0) {
                TextView t = v != null ? (TextView) v : text("", 11, T.text2);
                t.setPadding(T.px(8), T.px(8), 0, T.px(6));
                t.setText(r.text);
                return t;
            }
            if (r.type == 1) {
                LinearLayout row = (LinearLayout) v;
                if (row == null) {
                    row = new LinearLayout(a);
                    for (int i = 0; i < COLS; i++) {
                        LinearLayout cell = new LinearLayout(a);
                        cell.setOrientation(LinearLayout.VERTICAL);
                        cell.setGravity(Gravity.CENTER_HORIZONTAL);
                        cell.setPadding(T.px(2), T.px(9), T.px(2), T.px(8));
                        cell.setBackground(T.state(T.px(12)));
                        ImageView iv = new ImageView(a);
                        cell.addView(iv, new LinearLayout.LayoutParams(T.px(44), T.px(44)));
                        TextView t = text("", 11, T.text);
                        t.setSingleLine(true);
                        t.setEllipsize(TextUtils.TruncateAt.END);
                        t.setGravity(Gravity.CENTER);
                        t.setPadding(0, T.px(6), 0, 0);
                        cell.addView(t, new LinearLayout.LayoutParams(-1, -2));
                        row.addView(cell, new LinearLayout.LayoutParams(0, -2, 1));
                    }
                }
                for (int i = 0; i < COLS; i++) {
                    LinearLayout cell = (LinearLayout) row.getChildAt(i);
                    if (i >= r.count) { cell.setVisibility(View.INVISIBLE); cell.setOnClickListener(null); continue; }
                    Apps.App app = (Apps.App) items.get(r.base + i);
                    cell.setVisibility(View.VISIBLE);
                    ((ImageView) cell.getChildAt(0)).setImageBitmap(app.icon);
                    ((TextView) cell.getChildAt(1)).setText(app.label);
                    cell.setActivated(r.base + i == sel);
                    bind(cell, app);
                }
                return row;
            }
            LinearLayout row = (LinearLayout) v;
            if (row == null) {
                row = new LinearLayout(a);
                row.setGravity(Gravity.CENTER_VERTICAL);
                row.setPadding(T.px(10), T.px(7), T.px(12), T.px(7));
                row.setBackground(T.state(T.px(12)));
                ImageView iv = new ImageView(a);
                LinearLayout.LayoutParams il = new LinearLayout.LayoutParams(T.px(34), T.px(34));
                il.rightMargin = T.px(12);
                row.addView(iv, il);
                TextView t = text("", 14, T.text);
                t.setSingleLine(true);
                t.setEllipsize(TextUtils.TruncateAt.END);
                row.addView(t, new LinearLayout.LayoutParams(0, -2, 1));
                row.addView(text("", 11, T.text2));
            }
            Object it = items.get(r.base);
            ImageView iv = (ImageView) row.getChildAt(0);
            TextView t = (TextView) row.getChildAt(1), hint = (TextView) row.getChildAt(2);
            row.setActivated(r.base == sel);
            hint.setText(r.base == sel ? L.t("回车打开", "Enter — открыть", "Enter to open") : "");
            if (it instanceof Apps.App) {
                Apps.App app = (Apps.App) it;
                iv.setImageBitmap(app.icon);
                t.setText(app.label);
                bind(row, app);
            } else if (it instanceof Find.Hit) {
                Find.bind(Desk.this, row, iv, t, (Find.Hit) it);
            } else {
                iv.setImageDrawable(new T.Glyph(T.Glyph.NET, T.accent));
                t.setText(L.t("打开网址  ", "Открыть адрес  ", "Open address  ") + it);
                row.setOnClickListener(x -> openUrl((String) it));
                row.setOnLongClickListener(null);
                row.setOnContextClickListener(null);
            }
            return row;
        }
    }
    void bind(View v, Apps.App app) {
        v.setOnClickListener(x -> launch(app));
        v.setOnLongClickListener(x -> { menu(x, app); return true; });
        v.setOnContextClickListener(x -> { menu(x, app); return true; });
    }

    // ================= 键盘 =================
    /** 面板打开时的导航键。返回 true 表示已处理。 */
    boolean panelKey(int code) {
        if (code == KeyEvent.KEYCODE_ESCAPE) { hidePanels(); return true; }
        if (launcher.getVisibility() != View.VISIBLE) return false;
        if (code == KeyEvent.KEYCODE_ENTER || code == KeyEvent.KEYCODE_NUMPAD_ENTER) { activate(); return true; }
        int n = items.size();
        if (n == 0) return false;
        int next = sel;
        if (searching) {
            if (code == KeyEvent.KEYCODE_DPAD_DOWN) next = Math.min(n - 1, sel + 1);
            else if (code == KeyEvent.KEYCODE_DPAD_UP) next = Math.max(0, sel - 1);
            else return false;
        } else {
            if (code != KeyEvent.KEYCODE_DPAD_DOWN && code != KeyEvent.KEYCODE_DPAD_UP
                && code != KeyEvent.KEYCODE_DPAD_LEFT && code != KeyEvent.KEYCODE_DPAD_RIGHT) return false;
            if (sel < 0) next = 0;
            else {
                int rc = recentCount;
                boolean inRec = sel < rc;
                int col = inRec ? sel : (sel - rc) % COLS;
                if (code == KeyEvent.KEYCODE_DPAD_LEFT) next = Math.max(0, sel - 1);
                else if (code == KeyEvent.KEYCODE_DPAD_RIGHT) next = Math.min(n - 1, sel + 1);
                else if (code == KeyEvent.KEYCODE_DPAD_DOWN) next = inRec ? Math.min(n - 1, rc + col) : (sel + COLS < n ? sel + COLS : sel);
                else next = inRec ? sel : (sel - COLS >= rc ? sel - COLS : (rc > 0 ? Math.min(col, rc - 1) : sel));
            }
        }
        if (next != sel) {
            sel = next;
            adapter.notifyDataSetChanged();
            for (int i = 0; i < rows.size(); i++) {
                Row r = rows.get(i);
                if (r.type != 0 && sel >= r.base && sel < r.base + r.count) {
                    if (i < list.getFirstVisiblePosition() + 1 || i >= list.getLastVisiblePosition()) list.setSelection(Math.max(0, i - 1));
                    break;
                }
            }
        }
        return true;
    }
    void activate() {
        if (sel < 0 || sel >= items.size()) return;
        Object it = items.get(sel);
        if (it instanceof Apps.App) launch((Apps.App) it);
        else if (it instanceof Find.Hit) Find.open(this, (Find.Hit) it);
        else openUrl((String) it);
    }
    /** 桌面上（没有面板时）的按键：打字即搜索。 */
    boolean deskKey(int code, KeyEvent ev) {
        if (code == KeyEvent.KEYCODE_SEARCH) { toggle(launcher); return true; }
        if (launcher.getVisibility() == View.VISIBLE || quick.getVisibility() == View.VISIBLE) return panelKey(code);
        if (ev.isCtrlPressed() || ev.isAltPressed() || ev.isMetaPressed()) return false;
        int ch = ev.getUnicodeChar();
        if (ch > 32 && ch != 127 && !Character.isISOControl(ch)) {
            show(launcher);
            String s = new String(Character.toChars(ch));
            search.setText(s);
            search.setSelection(s.length());
            return true;
        }
        return false;
    }

    // ================= 面板开关 =================
    void toggle(View p) { if (p.getVisibility() == View.VISIBLE) hidePanels(); else show(p); }
    void show(View p) {
        View other = p == launcher ? quick : launcher;
        other.setVisibility(View.GONE);
        scrim.setVisibility(View.VISIBLE);
        if (p == launcher) {
            search.setText("");
            list.setSelection(0);
        } else refreshQuick();
        p.setVisibility(View.VISIBLE);
        p.setAlpha(0f);
        p.setTranslationY(T.px(14));
        p.animate().alpha(1f).translationY(0).setDuration(120).setInterpolator(new DecelerateInterpolator()).start();
        if (p == launcher) {
            search.requestFocus();
            if (a.getResources().getConfiguration().keyboard == Configuration.KEYBOARD_NOKEYS)
                ((InputMethodManager) a.getSystemService(Context.INPUT_METHOD_SERVICE)).showSoftInput(search, 0);
        }
    }
    boolean hidePanels() {
        boolean was = launcher.getVisibility() == View.VISIBLE || quick.getVisibility() == View.VISIBLE;
        launcher.animate().cancel(); quick.animate().cancel();
        launcher.setVisibility(View.GONE);
        quick.setVisibility(View.GONE);
        scrim.setVisibility(View.GONE);
        ((InputMethodManager) a.getSystemService(Context.INPUT_METHOD_SERVICE)).hideSoftInputFromWindow(root.getWindowToken(), 0);
        root.requestFocus();
        return was;
    }

    // ================= 动作 =================
    void open(Intent i) {
        try { a.startActivity(i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)); }
        catch (RuntimeException e) { Toast.makeText(a, L.t("系统未允许打开", "Система не разрешила открыть", "The system did not allow this"), Toast.LENGTH_SHORT).show(); }
    }
    void launch(Apps.App app) {
        hidePanels();
        apps.markUsed(app);
        open(new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER).setClassName(app.pkg, app.cls)
            .addFlags(Intent.FLAG_ACTIVITY_RESET_TASK_IF_NEEDED));
    }
    void openUrl(String q) {
        hidePanels();
        open(new Intent(Intent.ACTION_VIEW, Uri.parse(q.contains("://") ? q : "http://" + q)));
    }
    Context themed() {
        return new ContextThemeWrapper(a, T.dark ? android.R.style.Theme_DeviceDefault : android.R.style.Theme_DeviceDefault_Light);
    }
    void menu(View anchor, Apps.App app) {
        PopupMenu m = new PopupMenu(themed(), anchor);
        m.getMenu().add(0, 1, 0, apps.pinned(app) ? L.t("取消固定", "Открепить", "Unpin") : L.t("固定到底栏", "Закрепить на панели", "Pin to shelf"));
        m.getMenu().add(0, 2, 1, L.t("应用信息", "О приложении", "App info"));
        if (!app.system) m.getMenu().add(0, 3, 2, L.t("卸载", "Удалить", "Uninstall"));
        m.getMenu().add(0, 4, 3, L.t("从启动器隐藏", "Скрыть из меню", "Hide from launcher"));
        m.setOnMenuItemClickListener(it -> {
            Uri pkg = Uri.parse("package:" + app.pkg);
            switch (it.getItemId()) {
                case 1: apps.togglePin(app); renderPins(); break;
                case 2: hidePanels(); open(new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, pkg)); break;
                case 3: hidePanels(); open(new Intent(Intent.ACTION_DELETE, pkg)); break;
                case 4:
                    apps.hide(app);
                    if (apps.pinned(app)) { apps.togglePin(app); renderPins(); }
                    refreshRows();
                    break;
            }
            return true;
        });
        m.show();
    }

    void renderPins() {
        if (Shelf.it != null) Shelf.it.render(true);
        if (pinRow == null) return;
        pinRow.removeAllViews();
        for (String id : apps.pinIds()) {
            Apps.App app = apps.find(id);
            if (app == null) continue;
            ImageView iv = new ImageView(a);
            iv.setImageBitmap(app.icon);
            iv.setPadding(T.px(5), T.px(5), T.px(5), T.px(5));
            iv.setBackground(T.state(T.px(22)));
            iv.setContentDescription(app.label);
            if (Build.VERSION.SDK_INT >= 26) iv.setTooltipText(app.label);
            bind(iv, app);
            LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(T.px(44), T.px(44));
            lp.leftMargin = lp.rightMargin = T.px(3);
            pinRow.addView(iv, lp);
        }
    }
    static final String LOCALSEND = "org.localsend.localsend_app";
    View sharePill;
    Apps.App localSend() { for (Apps.App x : apps.all) if (LOCALSEND.equals(x.pkg)) return x; return null; }
    public void onApps() {
        // LocalSend 第一次出现时自动固定到底栏（只做一次，之后用户可自行取消）
        Apps.App ls = localSend();
        if (ls != null && !sp.getBoolean("ls_pinned", false)) {
            sp.edit().putBoolean("ls_pinned", true).apply();
            if (!apps.pinned(ls)) apps.togglePin(ls);
        }
        renderPins(); refreshRows();
    }

    // ================= 快捷设置 =================
    LinearLayout pill(String title, String sub) {
        LinearLayout p = new LinearLayout(a);
        p.setOrientation(LinearLayout.VERTICAL);
        p.setGravity(Gravity.CENTER_VERTICAL);
        p.setPadding(T.px(18), 0, T.px(12), 0);
        p.addView(text(title, 13, T.text));
        TextView s = text(sub, 10.5f, T.text2);
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
    LinearLayout pillRow(View l, View r) {
        LinearLayout row = new LinearLayout(a);
        LinearLayout.LayoutParams x = new LinearLayout.LayoutParams(0, T.px(48), 1), y = new LinearLayout.LayoutParams(0, T.px(48), 1);
        x.rightMargin = T.px(4); y.leftMargin = T.px(4);
        row.addView(l, x);
        if (r != null) row.addView(r, y); else row.addView(new View(a), y);
        return row;
    }
    SeekBar slider(LinearLayout parent, String name, TextView hint) {
        LinearLayout row = new LinearLayout(a);
        row.setGravity(Gravity.CENTER_VERTICAL);
        TextView t = text(name, 12, T.text2);
        row.addView(t, new LinearLayout.LayoutParams(T.px(62), -2));
        SeekBar s = new SeekBar(a);
        ColorStateList acc = ColorStateList.valueOf(T.accent);
        s.setProgressTintList(acc);
        s.setThumbTintList(acc);
        s.setProgressBackgroundTintList(ColorStateList.valueOf(T.text2));
        row.addView(s, new LinearLayout.LayoutParams(0, -2, 1));
        if (hint != null) row.addView(hint);
        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(-1, T.px(40));
        parent.addView(row, lp);
        return s;
    }

    void buildQuick() {
        LinearLayout p = new LinearLayout(a);
        p.setOrientation(LinearLayout.VERTICAL);
        p.setBackground(T.round(T.panel, T.px(18)));
        p.setElevation(T.px(10));
        p.setPadding(T.px(14), T.px(14), T.px(14), T.px(12));
        p.setClickable(true);
        p.setVisibility(View.GONE);
        FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(Math.round(W * 0.27f), -2, Gravity.BOTTOM | Gravity.RIGHT);
        lp.rightMargin = T.px(10); lp.bottomMargin = T.px(60);
        root.addView(p, lp);
        quick = p;

        LinearLayout wifi = pill(L.t("网络", "Сеть", "Network"), "…");
        wifiPill = wifi; wifiSub = (TextView) wifi.getChildAt(1);
        wifi.setOnClickListener(v -> {
            hidePanels();
            try { a.startActivity(new Intent(Build.VERSION.SDK_INT >= 29 ? "android.settings.panel.action.WIFI" : Settings.ACTION_WIFI_SETTINGS)); }
            catch (RuntimeException e) { open(new Intent(Settings.ACTION_WIFI_SETTINGS)); }
        });
        LinearLayout bt = null;
        final BluetoothAdapter ba = btAdapter();
        if (ba != null) {
            bt = pill(L.t("蓝牙", "Bluetooth", "Bluetooth"), "…");
            btPill = bt; btSub = (TextView) bt.getChildAt(1);
            bt.setOnClickListener(v -> {
                try {
                    boolean ok = ba.isEnabled() ? ba.disable() : ba.enable();
                    if (!ok) throw new SecurityException();
                    btSub.setText(L.t("切换中…", "Переключение…", "Switching…"));
                    ui.postDelayed(this::refreshQuick, 1800);
                    ui.postDelayed(this::refreshQuick, 4000);
                } catch (Throwable e) { hidePanels(); open(new Intent(Settings.ACTION_BLUETOOTH_SETTINGS)); }
            });
        }
        LinearLayout theme = pill(T.dark ? L.t("深色模式", "Тёмная тема", "Dark theme") : "浅色模式", L.t("点按切换", "Нажмите для смены", "Click to switch"));
        theme.setOnClickListener(v -> { sp.edit().putBoolean("dark", !T.dark).apply(); a.rebuild(); });
        LinearLayout sys = pill(L.t("系统设置", "Настройки", "System settings"), null);
        sys.setOnClickListener(v -> { hidePanels(); open(Main.systemSettings(a)); });

        LinearLayout r1 = bt != null ? pillRow(wifi, bt) : pillRow(wifi, theme);
        LinearLayout r2 = bt != null ? pillRow(theme, sys) : pillRow(sys, null);
        LinearLayout.LayoutParams rl = new LinearLayout.LayoutParams(-1, -2);
        rl.bottomMargin = T.px(8);
        p.addView(r1, rl);
        LinearLayout.LayoutParams rl2 = new LinearLayout.LayoutParams(-1, -2);
        rl2.bottomMargin = T.px(10);
        p.addView(r2, rl2);

        // LocalSend：局域网互传文件的入口
        final LinearLayout share = pill(L.t("传文件", "Передача файлов", "File transfer"), "LocalSend");
        sharePill = share;
        share.setOnClickListener(v -> {
            Apps.App ls = localSend();
            if (ls != null) launch(ls);
            else Toast.makeText(a, L.t("还没有安装 LocalSend", "LocalSend ещё не установлен", "LocalSend is not installed yet"), Toast.LENGTH_SHORT).show();
        });
        LinearLayout.LayoutParams sl3 = new LinearLayout.LayoutParams(-1, T.px(48));
        sl3.bottomMargin = T.px(10);
        p.addView(share, sl3);

        final AudioManager am = (AudioManager) a.getSystemService(Context.AUDIO_SERVICE);
        volume = slider(p, L.t("音量", "Звук", "Volume"), null);
        volume.setMax(am.getStreamMaxVolume(AudioManager.STREAM_MUSIC));
        volume.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
            public void onProgressChanged(SeekBar s, int v, boolean user) {
                if (user) try { am.setStreamVolume(AudioManager.STREAM_MUSIC, v, 0); } catch (RuntimeException ignored) {}
            }
            public void onStartTrackingTouch(SeekBar s) {}
            public void onStopTrackingTouch(SeekBar s) {}
        });
        brightHint = text(L.t("点按授权", "Разрешить", "Allow"), 10.5f, T.accent);
        bright = slider(p, L.t("亮度", "Яркость", "Brightness"), brightHint);
        bright.setMax(255);
        ((View) bright.getParent()).setOnClickListener(v -> {
            if (!Settings.System.canWrite(a)) { hidePanels(); open(new Intent(Settings.ACTION_MANAGE_WRITE_SETTINGS, Uri.parse("package:" + a.getPackageName()))); }
        });
        bright.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
            public void onProgressChanged(SeekBar s, int v, boolean user) {
                if (!user || !Settings.System.canWrite(a)) return;
                try {
                    Settings.System.putInt(a.getContentResolver(), Settings.System.SCREEN_BRIGHTNESS_MODE, Settings.System.SCREEN_BRIGHTNESS_MODE_MANUAL);
                    Settings.System.putInt(a.getContentResolver(), Settings.System.SCREEN_BRIGHTNESS, Math.max(8, v));
                } catch (RuntimeException ignored) {}
            }
            public void onStartTrackingTouch(SeekBar s) {}
            public void onStopTrackingTouch(SeekBar s) {}
        });

        View line = new View(a);
        line.setBackgroundColor(T.chip);
        LinearLayout.LayoutParams ll = new LinearLayout.LayoutParams(-1, Math.max(1, T.px(1)));
        ll.topMargin = T.px(8); ll.bottomMargin = T.px(4);
        p.addView(line, ll);

        LinearLayout f = new LinearLayout(a);
        f.setGravity(Gravity.CENTER_VERTICAL);
        foot = text("", 12, T.text2);
        foot.setSingleLine(true);
        foot.setEllipsize(TextUtils.TruncateAt.END);
        f.addView(foot, new LinearLayout.LayoutParams(0, -2, 1));
        TextView ds = text(L.t("桌面设置", "Рабочий стол", "Desktop settings"), 12, T.text);
        ds.setPadding(T.px(12), T.px(8), T.px(12), T.px(8));
        ds.setBackground(T.state(T.px(14)));
        ds.setOnClickListener(v -> deskSettings());
        f.addView(ds);
        p.addView(f);

        chan = text("", 10.5f, T.text2);
        chan.setPadding(0, T.px(4), 0, 0);
        chan.setOnClickListener(v -> {
            if (!Settings.canDrawOverlays(a)) { hidePanels(); open(new Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:" + a.getPackageName()))); }
        });
        p.addView(chan);
    }
    static BluetoothAdapter btAdapter() { try { return BluetoothAdapter.getDefaultAdapter(); } catch (Throwable e) { return null; } }

    void refreshQuick() {
        if (quick == null) return;
        boolean net = online();
        setPill((LinearLayout) wifiPill, net);
        wifiSub.setText(netName());
        if (btPill != null) {
            boolean on = false;
            try { on = btAdapter().isEnabled(); } catch (Throwable ignored) {}
            setPill((LinearLayout) btPill, on);
            btSub.setText(on ? L.t("已开启", "Включён", "On") : L.t("已关闭", "Выключен", "Off"));
        }
        AudioManager am = (AudioManager) a.getSystemService(Context.AUDIO_SERVICE);
        volume.setProgress(am.getStreamVolume(AudioManager.STREAM_MUSIC));
        boolean can = Settings.System.canWrite(a);
        bright.setEnabled(can);
        bright.setAlpha(can ? 1f : 0.4f);
        brightHint.setVisibility(can ? View.GONE : View.VISIBLE);
        try { bright.setProgress(Settings.System.getInt(a.getContentResolver(), Settings.System.SCREEN_BRIGHTNESS)); } catch (Exception ignored) {}
        String date = new java.text.SimpleDateFormat(L.t("M月d日 EEEE", "d MMMM, EEEE", "EEE, MMM d"), L.loc).format(new Date());
        foot.setText(hasBattery && batLevel >= 0 ? L.t("电量 ", "", "") + batLevel + "% · " + date : date);
        if (sharePill != null) ((TextView) ((LinearLayout) sharePill).getChildAt(1)).setText(
            localSend() != null ? L.t("LocalSend · 与手机、电脑互传", "LocalSend · обмен с телефоном и ПК", "LocalSend · share with phones and PCs")
                                : L.t("LocalSend · 未安装", "LocalSend · не установлен", "LocalSend · not installed"));
        channel();
    }

    void deskSettings() {
        final List<String> names = new ArrayList<>();
        final List<Runnable> acts = new ArrayList<>();
        String[] walls = {L.t("极光", "Сияние", "Aurora"), L.t("暮色", "Сумерки", "Dusk"), L.t("深海", "Глубина", "Deep sea"), L.t("晨雾", "Утренний туман", "Morning mist")};
        for (int i = 0; i < walls.length; i++) {
            final int idx = i;
            names.add(L.t("壁纸：", "Обои: ", "Wallpaper: ") + walls[i]);
            acts.add(() -> { sp.edit().putString("wall", String.valueOf(idx)).apply(); loadWallpaper(); });
        }
        names.add(L.t("壁纸：从文件选择图片…", "Обои: выбрать изображение…", "Wallpaper: choose an image…"));
        acts.add(() -> {
            try { a.startActivityForResult(new Intent(Intent.ACTION_GET_CONTENT).setType("image/*").addCategory(Intent.CATEGORY_OPENABLE), REQ_WALL); }
            catch (RuntimeException e) { Toast.makeText(a, L.t("没有可用的文件选择器", "Нет приложения для выбора файлов", "No file picker available"), Toast.LENGTH_SHORT).show(); }
        });
        int hid = apps.hidden().size();
        if (hid > 0) { names.add(L.t("恢复被隐藏的应用（", "Вернуть скрытые приложения (", "Restore hidden apps (") + hid + L.t(" 个）", ")", ")")); acts.add(() -> { apps.unhideAll(); refreshRows(); }); }
        Uri me = Uri.parse("package:" + a.getPackageName());
        names.add(L.t("应用管理", "Приложения", "Manage apps")); acts.add(() -> open(new Intent(Settings.ACTION_MANAGE_APPLICATIONS_SETTINGS)));
        names.add(L.t("开发者选项", "Для разработчиков", "Developer options")); acts.add(() -> open(new Intent(Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS)));
        names.add(L.t("悬浮窗权限", "Разрешение «Поверх других приложений»", "\"Display over other apps\" permission")); acts.add(() -> open(new Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, me)));
        names.add(L.t("默认桌面", "Рабочий стол по умолчанию", "Default home app")); acts.add(() -> open(new Intent(Settings.ACTION_HOME_SETTINGS)));
        hidePanels();
        new AlertDialog.Builder(themed())
            .setTitle(L.t("桌面设置", "Настройки рабочего стола", "Desktop settings") + " · " + Main.versionName(a))
            .setItems(names.toArray(new String[0]), (d, which) -> acts.get(which).run())
            .show();
    }

    // ================= 壁纸 =================
    File wallFile() { return new File(Store.ctx(a).getFilesDir(), "wallpaper.jpg"); }
    void loadWallpaper() {
        final String w = sp.getString("wall", T.dark ? "0" : "3");
        final FrameLayout target = root;
        apps.io.execute(() -> {
            Bitmap b = null;
            try {
                if ("file".equals(w)) b = BitmapFactory.decodeFile(wallFile().getPath());
                if (b == null) b = T.wallpaper(W, H, "file".equals(w) ? 0 : Integer.parseInt(w));
            } catch (Throwable ignored) {}
            final Bitmap out = b;
            ui.post(() -> {
                if (out == null || target != root) return;
                BitmapDrawable d = new BitmapDrawable(a.getResources(), out);
                d.setFilterBitmap(true);
                root.setBackground(d);
                a.getWindow().setBackgroundDrawable(null);
            });
        });
    }
    void pickedWallpaper(final Uri uri) {
        apps.io.execute(() -> {
            try {
                BitmapFactory.Options o = new BitmapFactory.Options();
                o.inJustDecodeBounds = true;
                try (InputStream in = a.getContentResolver().openInputStream(uri)) { BitmapFactory.decodeStream(in, null, o); }
                int s = 1;
                while (o.outWidth / (s * 2) >= W && o.outHeight / (s * 2) >= H) s *= 2;
                o = new BitmapFactory.Options();
                o.inSampleSize = s;
                Bitmap src;
                try (InputStream in = a.getContentResolver().openInputStream(uri)) { src = BitmapFactory.decodeStream(in, null, o); }
                if (src == null) throw new IOException("decode");
                float k = Math.max(W / (float) src.getWidth(), H / (float) src.getHeight());
                int cw = Math.min(src.getWidth(), Math.round(W / k)), ch = Math.min(src.getHeight(), Math.round(H / k));
                Bitmap crop = Bitmap.createBitmap(src, (src.getWidth() - cw) / 2, (src.getHeight() - ch) / 2, cw, ch);
                Bitmap fit = cw > W ? Bitmap.createScaledBitmap(crop, W, H, true) : crop;
                File tmp = new File(Store.ctx(a).getFilesDir(), "wallpaper.tmp");
                try (FileOutputStream f = new FileOutputStream(tmp)) { fit.compress(Bitmap.CompressFormat.JPEG, 90, f); }
                tmp.renameTo(wallFile());
                sp.edit().putString("wall", "file").apply();
                ui.post(this::loadWallpaper);
            } catch (Throwable e) {
                ui.post(() -> Toast.makeText(a, L.t("这张图片无法用作壁纸", "Это изображение не подходит для обоев", "This image can't be used as wallpaper"), Toast.LENGTH_SHORT).show());
            }
        });
    }

    // ================= 状态栏信息 =================
    boolean online() {
        try {
            ConnectivityManager cm = (ConnectivityManager) a.getSystemService(Context.CONNECTIVITY_SERVICE);
            Network n = cm.getActiveNetwork();
            return n != null && cm.getNetworkCapabilities(n) != null;
        } catch (Throwable e) { return false; }
    }
    String netName() {
        try {
            ConnectivityManager cm = (ConnectivityManager) a.getSystemService(Context.CONNECTIVITY_SERVICE);
            NetworkCapabilities nc = cm.getNetworkCapabilities(cm.getActiveNetwork());
            if (nc == null) return L.t("未连接", "Нет сети", "Not connected");
            if (nc.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)) return L.t("有线已连接", "Проводная сеть", "Wired");
            if (nc.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) return L.t("Wi-Fi 已连接", "Wi-Fi подключён", "Wi-Fi connected");
            return L.t("已连接", "Подключено", "Connected");
        } catch (Throwable e) { return L.t("未连接", "Нет сети", "Not connected"); }
    }
    void channel() {
        String s = String.valueOf(AgentService.status);
        boolean overlay = Settings.canDrawOverlays(a);
        String key = s + overlay;
        if (key.equals(lastChan)) return;
        lastChan = key;
        boolean ok = s.contains("\u5df2\u8fde\u63a5");
        dot.setBackground(T.round(ok ? 0xFF5BD08D : 0xFFF0A020, T.px(4)));
        String st = s.contains("已连接") ? L.t("已连接 Mac ✓", "Mac подключён ✓", "Mac connected ✓")
            : s.contains("失败") ? L.t("连接 Mac 失败，重试中…", "нет связи с Mac, повтор…", "Mac unreachable, retrying…")
            : s.contains("拒绝") ? L.t("拒绝未签名指令", "отклонена неподписанная команда", "rejected an unsigned command")
            : s.contains("待命") ? L.t("待命", "ожидание", "idle") : s;
        String head = L.t("控制通道：", "Канал управления: ", "Control channel: ") + st;
        chan.setText(overlay ? head : head + "\n" + L.t("⚠ 需要「显示在其他应用上层」权限，点这里授权", "⚠ Нужно разрешение «Поверх других приложений» — нажмите здесь", "⚠ Needs \"Display over other apps\" permission — click here"));
    }
    void tick() {
        clock.setText(android.text.format.DateFormat.format("HH:mm", new Date()));
        netGlyph.on = online();
        netIcon.invalidate();
    }
    final Runnable poll = new Runnable() { public void run() { if (!resumed) return; channel(); ui.postDelayed(this, 5000); } };
    final BroadcastReceiver rx = new BroadcastReceiver() {
        public void onReceive(Context c, Intent i) {
            String act = String.valueOf(i.getAction());
            if (Intent.ACTION_BATTERY_CHANGED.equals(act)) {
                hasBattery = i.getBooleanExtra("present", false);
                int lv = i.getIntExtra("level", -1), sc = i.getIntExtra("scale", 100);
                batLevel = lv >= 0 && sc > 0 ? lv * 100 / sc : -1;
                boolean showIt = hasBattery && batLevel >= 0;
                batBox.setVisibility(showIt ? View.VISIBLE : View.GONE);
                if (showIt) { batGlyph.level = batLevel / 100f; batIcon.invalidate(); batText.setText(batLevel + "%"); }
            } else if (act.startsWith("android.intent.action.PACKAGE_")) apps.load(false);
            else if (Intent.ACTION_USER_UNLOCKED.equals(act)) { try { a.startAgent(); } catch (Throwable ignored) {} apps.load(false); }
            else tick();
        }
    };
    void resume() {
        resumed = true;
        IntentFilter f = new IntentFilter();
        f.addAction(Intent.ACTION_TIME_TICK); f.addAction(Intent.ACTION_TIME_CHANGED); f.addAction(Intent.ACTION_TIMEZONE_CHANGED);
        f.addAction(Intent.ACTION_BATTERY_CHANGED); f.addAction(ConnectivityManager.CONNECTIVITY_ACTION);
        f.addAction(Intent.ACTION_USER_UNLOCKED);
        a.registerReceiver(rx, f);
        IntentFilter pf = new IntentFilter();
        pf.addAction(Intent.ACTION_PACKAGE_ADDED); pf.addAction(Intent.ACTION_PACKAGE_REMOVED); pf.addAction(Intent.ACTION_PACKAGE_CHANGED);
        pf.addDataScheme("package");
        a.registerReceiver(rx, pf);
        tick();
        lastChan = "";
        ui.removeCallbacks(poll);
        poll.run();
        apps.load(true);
    }
    void pause() {
        resumed = false;
        ui.removeCallbacksAndMessages(null);
        try { a.unregisterReceiver(rx); } catch (RuntimeException ignored) {}
    }
}
