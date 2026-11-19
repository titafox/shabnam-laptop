package local.cloud.launcher;

import android.app.Activity;
import android.app.ActivityManager;
import android.app.AlertDialog;
import android.content.*;
import android.content.pm.PackageInfo;
import android.graphics.Typeface;
import android.graphics.drawable.ColorDrawable;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.LayerDrawable;
import android.graphics.drawable.StateListDrawable;
import android.net.Uri;
import android.os.*;
import android.os.storage.StorageManager;
import android.os.storage.StorageVolume;
import android.text.*;
import android.text.style.ForegroundColorSpan;
import android.text.style.RelativeSizeSpan;
import android.util.Log;
import android.util.TypedValue;
import android.view.*;
import android.view.inputmethod.EditorInfo;
import android.widget.*;
import java.io.File;
import java.util.*;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * 「文件」：桌面自带的文件管理器，独立窗口、独立进程（出了错不会连累桌面）。
 * 左边是常用位置和外接存储，上面是路径栏，中间是列表或图标视图。
 * 鼠标：单击选中、双击打开、右键菜单、Ctrl/Shift 多选。键盘：方向键、回车、Delete、F2、Ctrl+C/X/V/A、Ctrl+F、退格返回上一级。
 */
public class Files extends Activity {
    static final String TAG = "Files";
    static final int ROW = 36;

    static final class Entry {
        File f;
        String path, name, low, sub;
        boolean dir;
        long size, time;
    }
    static final class Place {
        String label;
        File dir;
        int glyph;
        String volId; // 外接存储才有：用来弹出
    }
    static final class H { int pos; ImageView icon; TextView name, size, type, date; }
    interface Str { void got(String s); }

    // ---------- 给桌面其他部分用的入口 ----------
    /** 打开「文件」窗口并进入 path；select 不为空时选中那个文件。 */
    static void start(Context c, String path, String select) {
        try {
            Intent i = new Intent(c, Files.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            if (path != null) i.putExtra("path", path);
            if (select != null) i.putExtra("select", select);
            c.startActivity(i);
        } catch (Throwable e) { Log.w(TAG, "start: " + e); }
    }
    /** 用别的应用打开文件（安装包则交给系统安装器）。chooser = 每次都让用户选应用。 */
    static boolean view(Context c, File f, boolean chooser) {
        try {
            Intent i = new Intent(Intent.ACTION_VIEW).setDataAndType(FilesProvider.uri(f), FU.mime(f))
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
            if (chooser) i = Intent.createChooser(i, f.getName());
            c.startActivity(i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
            return true;
        } catch (ActivityNotFoundException e) {
            FileOps.toast(c, L.t("没有可以打开这种文件的应用", "Нет приложения для открытия этого файла", "No app can open this kind of file"));
        } catch (Throwable e) {
            Log.w(TAG, "view: " + e);
            FileOps.toast(c, L.t("系统未允许打开", "Система не разрешила открыть", "The system did not allow this"));
        }
        return false;
    }

    // ---------- 状态 ----------
    static final List<File> clip = new ArrayList<>(); // 文件剪贴板（本窗口内的复制 / 剪切）
    static boolean clipCut;

    final Handler ui = new Handler(Looper.getMainLooper());
    final ExecutorService io = Executors.newSingleThreadExecutor();
    SharedPreferences sp;
    File home, dir;
    String deep; // 不为空 = 正在显示「在子文件夹里搜索」的结果
    final ArrayList<File> back = new ArrayList<>(), fwd = new ArrayList<>();
    List<Entry> raw = new ArrayList<>(), shown = new ArrayList<>();
    final List<Place> places = new ArrayList<>();
    final LinkedHashSet<String> sel = new LinkedHashSet<>();
    boolean readable = true, loading, deepCut;
    int cursor = -1, anchorPos = -1, loadGen;
    int sort; // 0 名称 1 大小 2 时间 3 类型
    boolean desc, grid, hidden, builtDark, compact, showType = true, showDate = true, muteSearch;
    String pendingSelect, typed = "";
    long typedAt, lastClickTime;
    int lastClickPos = -1, downMeta, downButtons, lastW = -1;
    float lastX, lastY, blankY;
    FileObserver watcher;

    FrameLayout root;
    View anchor, sideScroll;
    LinearLayout side, crumbs, header;
    HorizontalScrollView crumbScroll;
    EditText search;
    ListView list;
    GridView gridv;
    TextView empty, status, free, banner;
    ImageView viewBtn, backBtn, fwdBtn, upBtn, newBtn;
    final Ad listAd = new Ad(false), gridAd = new Ad(true);

    // ================= 生命周期 =================
    protected void onCreate(Bundle state) {
        boolean dark = true;
        try { L.init(this); } catch (Throwable ignored) {}
        try { dark = readDark(); } catch (Throwable ignored) {}
        setTheme(dark ? android.R.style.Theme_DeviceDefault_NoActionBar : android.R.style.Theme_DeviceDefault_Light_NoActionBar);
        super.onCreate(state);
        try {
            android.util.DisplayMetrics dm = new android.util.DisplayMetrics();
            ((WindowManager) getApplicationContext().getSystemService(WINDOW_SERVICE)).getDefaultDisplay().getRealMetrics(dm);
            T.u = Math.max(dm.widthPixels, dm.heightPixels) / 1280f;
            T.apply(dark);
            builtDark = dark;
            sp = FileOps.prefs(this);
            sort = sp.getInt("sort", 0); desc = sp.getBoolean("desc", false);
            grid = sp.getBoolean("grid", false); hidden = sp.getBoolean("hidden", false);
            home = FileOps.home();
            String title = L.t("文件", "Файлы", "Files");
            setTitle(title);
            try { setTaskDescription(new ActivityManager.TaskDescription(title)); } catch (Throwable ignored) {}
            setContentView(build());
            IntentFilter f = new IntentFilter();
            f.addAction(Intent.ACTION_MEDIA_MOUNTED); f.addAction(Intent.ACTION_MEDIA_UNMOUNTED); f.addAction(Intent.ACTION_MEDIA_EJECT);
            f.addAction(Intent.ACTION_MEDIA_REMOVED); f.addAction(Intent.ACTION_MEDIA_BAD_REMOVAL);
            f.addDataScheme("file");
            registerReceiver(volumeRx, f);
            refreshPlaces();
            String saved = state != null ? state.getString("path") : null;
            if (saved != null && new File(saved).isDirectory()) go(new File(saved), false);
            else if (!handle(getIntent())) go(home, false);
        } catch (Throwable t) {
            Log.e(TAG, "create", t);
            FileOps.toast(this, L.t("文件管理器无法启动", "Не удалось запустить файловый менеджер", "The file manager could not start"));
            finish();
        }
    }
    boolean readDark() {
        try {
            Bundle b = getContentResolver().call(Uri.parse("content://" + FilesProvider.AUTH), "dark", null, null);
            if (b != null && b.containsKey("dark")) return b.getBoolean("dark", true);
        } catch (Throwable e) { Log.w(TAG, "dark: " + e); }
        return Store.ctx(this).getSharedPreferences("desk", 0).getBoolean("dark", true);
    }
    boolean handle(Intent i) {
        if (i == null) return false;
        String p = i.getStringExtra("path");
        if (p == null) return false;
        File d = norm(new File(p));
        if (!d.isDirectory()) return false;
        String s = i.getStringExtra("select");
        go(d, dir != null);
        pendingSelect = s == null ? null : norm(new File(s)).getAbsolutePath();
        return true;
    }
    /** /sdcard/… 这类别名统一成主目录的真实路径，侧栏、路径栏和回收站才认得出来。 */
    File norm(File f) {
        String p = f.getAbsolutePath(), h = home.getAbsolutePath();
        for (String alias : new String[]{"/sdcard", "/storage/self/primary", "/mnt/sdcard"})
            if (p.equals(alias) || p.startsWith(alias + "/")) return new File(h + p.substring(alias.length()));
        try {
            String c = f.getCanonicalPath();
            if (!c.equals(p) && new File(c).exists()) return new File(c);
        } catch (Throwable ignored) {}
        return f;
    }
    protected void onNewIntent(Intent i) {
        super.onNewIntent(i);
        try { handle(i); } catch (Throwable e) { Log.w(TAG, "intent: " + e); }
    }
    protected void onSaveInstanceState(Bundle out) {
        super.onSaveInstanceState(out);
        if (dir != null) out.putString("path", dir.getAbsolutePath());
    }
    protected void onResume() {
        super.onResume();
        try {
            if (root == null) return;
            if (readDark() != builtDark) { recreate(); return; }
            checkAccess();
            refreshPlaces();
            if (dir != null) load();
        } catch (Throwable e) { Log.w(TAG, "resume: " + e); }
    }
    protected void onDestroy() {
        super.onDestroy();
        try { unregisterReceiver(volumeRx); } catch (Throwable ignored) {}
        try { if (watcher != null) watcher.stopWatching(); } catch (Throwable ignored) {}
        ui.removeCallbacksAndMessages(null);
        io.shutdownNow();
    }
    final BroadcastReceiver volumeRx = new BroadcastReceiver() {
        public void onReceive(Context c, Intent i) {
            ui.postDelayed(() -> safe(() -> {
                refreshPlaces();
                if (dir != null && !dir.isDirectory()) { back.clear(); fwd.clear(); go(home, false); }
            }), 400);
        }
    };
    void safe(Runnable r) {
        try { r.run(); } catch (Throwable e) { Log.w(TAG, "ui: " + e, e); }
    }

    // ================= 界面搭建 =================
    int bg() { return T.dark ? 0xFF202124 : 0xFFFFFFFF; }
    Context themed() {
        return new ContextThemeWrapper(this, T.dark ? android.R.style.Theme_DeviceDefault : android.R.style.Theme_DeviceDefault_Light);
    }
    TextView text(String s, float size, int color) {
        TextView t = new TextView(this);
        t.setText(s); t.setTextColor(color);
        t.setTextSize(TypedValue.COMPLEX_UNIT_PX, T.px(size));
        t.setIncludeFontPadding(false);
        return t;
    }
    /** 条目底色：选中是淡淡的强调色，鼠标悬停是中性灰。 */
    Drawable itemBg(float radius) {
        int a = T.accent & 0x00FFFFFF;
        StateListDrawable s = new StateListDrawable();
        s.addState(new int[]{android.R.attr.state_activated, android.R.attr.state_hovered}, T.round(a | 0x66000000, radius));
        s.addState(new int[]{android.R.attr.state_activated}, T.round(a | 0x47000000, radius));
        s.addState(new int[]{android.R.attr.state_pressed}, T.round(T.chip, radius));
        s.addState(new int[]{android.R.attr.state_hovered}, T.round(T.chip, radius));
        s.addState(new int[0], new ColorDrawable(0));
        return s;
    }
    ImageView button(int glyph, String tip, final Runnable act) {
        ImageView b = new ImageView(this);
        b.setImageDrawable(new FI.Glyph(glyph, T.text));
        int p = T.px(8);
        b.setPadding(p, p, p, p);
        b.setBackground(T.state(T.px(17)));
        b.setContentDescription(tip);
        if (Build.VERSION.SDK_INT >= 26) b.setTooltipText(tip);
        b.setOnClickListener(v -> safe(act));
        b.setFocusable(false);
        return b;
    }
    void enable(ImageView b, boolean on) { b.setEnabled(on); b.setAlpha(on ? 1f : 0.35f); }

    View build() {
        getWindow().setBackgroundDrawable(new ColorDrawable(bg()));
        try { getWindow().setStatusBarColor(bg()); getWindow().setNavigationBarColor(bg()); } catch (Throwable ignored) {}
        root = new FrameLayout(this);
        root.setFocusable(true);
        root.setFocusableInTouchMode(true);

        LinearLayout main = new LinearLayout(this);
        root.addView(main, new FrameLayout.LayoutParams(-1, -1));

        // ---- 左侧栏 ----
        ScrollView ss = new ScrollView(this);
        ss.setBackgroundColor(T.dark ? 0xFF17181B : 0xFFF1F3F4);
        ss.setVerticalScrollBarEnabled(false);
        ss.setFocusable(false);
        side = new LinearLayout(this);
        side.setOrientation(LinearLayout.VERTICAL);
        side.setPadding(T.px(8), T.px(10), T.px(8), T.px(10));
        ss.addView(side, new FrameLayout.LayoutParams(-1, -2));
        sideScroll = ss;
        main.addView(ss, new LinearLayout.LayoutParams(T.px(188), -1));

        LinearLayout right = new LinearLayout(this);
        right.setOrientation(LinearLayout.VERTICAL);
        main.addView(right, new LinearLayout.LayoutParams(0, -1, 1));

        // ---- 工具栏 ----
        LinearLayout bar = new LinearLayout(this);
        bar.setGravity(Gravity.CENTER_VERTICAL);
        bar.setPadding(T.px(8), 0, T.px(8), 0);
        right.addView(bar, new LinearLayout.LayoutParams(-1, T.px(48)));
        int bs = T.px(34);
        backBtn = button(FI.BACK, L.t("后退 (Alt+←)", "Назад (Alt+←)", "Back (Alt+←)"), this::goBack);
        fwdBtn = button(FI.FWD, L.t("前进 (Alt+→)", "Вперёд (Alt+→)", "Forward (Alt+→)"), this::goFwd);
        upBtn = button(FI.UP, L.t("上一级 (退格)", "Вверх (Backspace)", "Up (Backspace)"), this::goUp);
        bar.addView(backBtn, new LinearLayout.LayoutParams(bs, bs));
        bar.addView(fwdBtn, new LinearLayout.LayoutParams(bs, bs));
        bar.addView(upBtn, new LinearLayout.LayoutParams(bs, bs));

        crumbScroll = new HorizontalScrollView(this);
        crumbScroll.setHorizontalScrollBarEnabled(false);
        crumbScroll.setOverScrollMode(View.OVER_SCROLL_NEVER);
        crumbScroll.setBackground(T.round(T.chip, T.px(17)));
        crumbScroll.setFocusable(false);
        crumbs = new LinearLayout(this);
        crumbs.setGravity(Gravity.CENTER_VERTICAL);
        crumbs.setPadding(T.px(6), 0, T.px(6), 0);
        crumbScroll.addView(crumbs, new FrameLayout.LayoutParams(-2, -1));
        LinearLayout.LayoutParams cl = new LinearLayout.LayoutParams(0, bs, 1);
        cl.leftMargin = T.px(6); cl.rightMargin = T.px(8);
        bar.addView(crumbScroll, cl);

        search = new EditText(this);
        search.setHint(L.t("搜索  Ctrl+F", "Поиск  Ctrl+F", "Search  Ctrl+F"));
        search.setHintTextColor(T.text2);
        search.setTextColor(T.text);
        search.setTextSize(TypedValue.COMPLEX_UNIT_PX, T.px(13));
        search.setSingleLine(true);
        search.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
        search.setImeOptions(EditorInfo.IME_ACTION_SEARCH | EditorInfo.IME_FLAG_NO_EXTRACT_UI);
        search.setBackground(T.round(T.chip, T.px(17)));
        search.setPadding(T.px(14), 0, T.px(14), 0);
        search.setGravity(Gravity.CENTER_VERTICAL);
        search.addTextChangedListener(new TextWatcher() {
            public void beforeTextChanged(CharSequence s, int x, int y, int z) {}
            public void onTextChanged(CharSequence s, int x, int y, int z) {}
            public void afterTextChanged(Editable e) {
                if (muteSearch) return;
                safe(() -> { if (deep != null) { deep = null; renderCrumbs(); load(); } else applyFilter(); });
            }
        });
        search.setOnEditorActionListener((v, id, ev) -> { safe(Files.this::deepSearch); return true; });
        LinearLayout.LayoutParams sl = new LinearLayout.LayoutParams(T.px(180), bs);
        sl.rightMargin = T.px(4);
        bar.addView(search, sl);

        viewBtn = button(FI.GRID, "", () -> setGrid(!grid));
        bar.addView(viewBtn, new LinearLayout.LayoutParams(bs, bs));
        newBtn = button(FI.NEWDIR, L.t("新建文件夹 (Ctrl+Shift+N)", "Новая папка (Ctrl+Shift+N)", "New folder (Ctrl+Shift+N)"), this::newFolder);
        bar.addView(newBtn, new LinearLayout.LayoutParams(bs, bs));
        final ImageView more = button(FI.MORE, L.t("更多", "Ещё", "More"), null);
        more.setOnClickListener(v -> safe(() -> moreMenu(more)));
        bar.addView(more, new LinearLayout.LayoutParams(bs, bs));

        // ---- 权限提示条 ----
        banner = text("", 13, T.text);
        banner.setPadding(T.px(14), T.px(9), T.px(14), T.px(9));
        banner.setBackground(new LayerDrawable(new Drawable[]{T.round((T.accent & 0x00FFFFFF) | 0x33000000, T.px(10)), T.state(T.px(10))}));
        banner.setVisibility(View.GONE);
        banner.setOnClickListener(v -> safe(this::askAccess));
        LinearLayout.LayoutParams bl = new LinearLayout.LayoutParams(-1, -2);
        bl.leftMargin = bl.rightMargin = T.px(10); bl.bottomMargin = T.px(6);
        right.addView(banner, bl);

        // ---- 列表表头 ----
        header = new LinearLayout(this);
        header.setGravity(Gravity.CENTER_VERTICAL);
        header.setPadding(T.px(16), 0, T.px(16), 0);
        for (int i = 0; i < 4; i++) {
            final int col = new int[]{0, 1, 3, 2}[i];
            TextView t = text("", 11.5f, T.text2);
            t.setSingleLine(true);
            t.setGravity(Gravity.CENTER_VERTICAL | (col == 1 ? Gravity.RIGHT : Gravity.LEFT));
            t.setPadding(col == 0 ? T.px(38) : T.px(8), 0, T.px(8), 0);
            t.setBackground(T.state(T.px(8)));
            t.setOnClickListener(v -> safe(() -> sortBy(col)));
            t.setFocusable(false);
            header.addView(t, col == 0 ? new LinearLayout.LayoutParams(0, -1, 1) : new LinearLayout.LayoutParams(colWidth(col), -1));
        }
        right.addView(header, new LinearLayout.LayoutParams(-1, T.px(30)));

        // ---- 内容 ----
        FrameLayout content = new FrameLayout(this);
        right.addView(content, new LinearLayout.LayoutParams(-1, 0, 1));
        list = new ListView(this);
        gridv = new GridView(this);
        for (AbsListView v : new AbsListView[]{list, gridv}) {
            v.setSelector(new ColorDrawable(0));
            v.setFocusable(false);
            v.setCacheColorHint(0);
            v.setOverScrollMode(View.OVER_SCROLL_NEVER);
            v.setClipToPadding(false);
            v.setOnTouchListener((x, ev) -> { blankTouch(ev); return false; });
            v.setOnContextClickListener(x -> { safe(() -> menu(-1)); return true; });
            content.addView(v, new FrameLayout.LayoutParams(-1, -1));
        }
        list.setDivider(null);
        list.setItemsCanFocus(false);
        list.setPadding(T.px(6), 0, T.px(6), T.px(8));
        list.setAdapter(listAd);
        gridv.setColumnWidth(T.px(112));
        gridv.setNumColumns(GridView.AUTO_FIT);
        gridv.setStretchMode(GridView.STRETCH_COLUMN_WIDTH);
        gridv.setVerticalSpacing(T.px(4));
        gridv.setHorizontalSpacing(T.px(4));
        gridv.setPadding(T.px(10), T.px(6), T.px(10), T.px(10));
        gridv.setAdapter(gridAd);
        empty = text("", 14, T.text2);
        empty.setGravity(Gravity.CENTER);
        empty.setPadding(T.px(24), 0, T.px(24), 0);
        content.addView(empty, new FrameLayout.LayoutParams(-1, -2, Gravity.CENTER));

        // ---- 状态栏 ----
        LinearLayout sb = new LinearLayout(this);
        sb.setGravity(Gravity.CENTER_VERTICAL);
        sb.setPadding(T.px(16), 0, T.px(16), 0);
        status = text("", 11.5f, T.text2);
        status.setSingleLine(true);
        status.setEllipsize(TextUtils.TruncateAt.END);
        sb.addView(status, new LinearLayout.LayoutParams(0, -2, 1));
        free = text("", 11.5f, T.text2);
        free.setSingleLine(true);
        sb.addView(free);
        right.addView(sb, new LinearLayout.LayoutParams(-1, T.px(26)));

        anchor = new View(this);
        root.addView(anchor, new FrameLayout.LayoutParams(1, 1));

        root.addOnLayoutChangeListener((v, l, t, r, b, ol, ot, or, ob) -> {
            final int w = r - l;
            if (w != lastW) { lastW = w; root.post(() -> safe(() -> fit(w))); }
        });
        setGrid(grid);
        renderHeader();
        root.requestFocus();
        return root;
    }
    int colWidth(int col) { return T.px(col == 1 ? 86 : col == 3 ? 124 : 136); }

    /** 窗口变窄时：侧栏只留图标，依次收起「类型」「修改时间」两列，搜索框变短。 */
    void fit(int w) {
        boolean c = w < T.px(640), ty = w >= T.px(880), da = w >= T.px(560);
        ViewGroup.LayoutParams sl = search.getLayoutParams();
        sl.width = w < T.px(560) ? T.px(92) : w < T.px(900) ? T.px(150) : T.px(210);
        search.setLayoutParams(sl);
        search.setHint(w < T.px(560) ? L.t("搜索", "Поиск", "Search") : L.t("搜索  Ctrl+F", "Поиск  Ctrl+F", "Search  Ctrl+F"));
        newBtn.setVisibility(w < T.px(560) ? View.GONE : View.VISIBLE); // 窄窗口里「新建文件夹」只留在菜单中
        crumbScroll.post(() -> crumbScroll.fullScroll(View.FOCUS_RIGHT));
        if (c != compact) {
            compact = c;
            ViewGroup.LayoutParams lp = sideScroll.getLayoutParams();
            lp.width = T.px(c ? 52 : 188);
            sideScroll.setLayoutParams(lp);
            renderSide();
        }
        if (ty != showType || da != showDate) {
            showType = ty; showDate = da;
            renderHeader();
            listAd.notifyDataSetChanged();
        }
    }

    // ================= 侧栏 =================
    void refreshPlaces() {
        places.clear();
        place(L.t("主目录", "Домашняя папка", "Home"), home, FI.HOME);
        place(L.t("下载", "Загрузки", "Downloads"), new File(home, Environment.DIRECTORY_DOWNLOADS), FI.DOWNLOAD);
        place(L.t("图片", "Изображения", "Pictures"), new File(home, Environment.DIRECTORY_PICTURES), FI.PICTURE);
        place(L.t("文档", "Документы", "Documents"), new File(home, Environment.DIRECTORY_DOCUMENTS), FI.DOC);
        place(L.t("收到的文件", "Полученные", "Received"), new File(home, "Download/LocalSend"), FI.RECEIVED);
        try { volumes(); } catch (Throwable e) { Log.w(TAG, "volumes: " + e); }
        place(L.t("回收站", "Корзина", "Trash"), FileOps.trashDir(), FI.TRASH);
        renderSide();
    }
    Place place(String label, File d, int glyph) {
        Place p = new Place();
        p.label = label; p.dir = d; p.glyph = glyph;
        places.add(p);
        return p;
    }
    /** 外接存储（U 盘、存储卡）。优先用隐藏接口（能拿到弹出要用的卷号），不行再用公开接口。 */
    void volumes() throws Exception {
        StorageManager sm = (StorageManager) getSystemService(STORAGE_SERVICE);
        Set<String> seen = new HashSet<>();
        try {
            for (Object v : (List<?>) StorageManager.class.getMethod("getVolumes").invoke(sm)) {
                Class<?> k = v.getClass();
                if ((Integer) k.getMethod("getType").invoke(v) != 0) continue; // 0 = 公共卷（外接）
                int st = (Integer) k.getMethod("getState").invoke(v);
                if (st != 2 && st != 3) continue; // 已挂载 / 只读挂载
                File p = (File) k.getMethod("getPath").invoke(v);
                if (p == null || !p.canRead()) {
                    File in = (File) k.getMethod("getInternalPath").invoke(v);
                    if (in != null && in.canRead()) p = in;
                }
                if (p == null || !seen.add(p.getAbsolutePath())) continue;
                String label = null;
                try { label = (String) StorageManager.class.getMethod("getBestVolumeDescription", k).invoke(sm, v); } catch (Throwable ignored) {}
                if (label == null || label.isEmpty()) label = p.getName();
                place(label, p, FI.USB).volId = (String) k.getMethod("getId").invoke(v);
            }
            return;
        } catch (Throwable e) { Log.w(TAG, "hidden volumes: " + e); }
        if (Build.VERSION.SDK_INT < 24) return;
        for (StorageVolume v : sm.getStorageVolumes()) {
            if (v.isPrimary() || !v.isRemovable()) continue;
            String st = v.getState();
            if (!Environment.MEDIA_MOUNTED.equals(st) && !Environment.MEDIA_MOUNTED_READ_ONLY.equals(st)) continue;
            File p = Build.VERSION.SDK_INT >= 30 ? v.getDirectory() : new File((String) StorageVolume.class.getMethod("getPath").invoke(v));
            if (p == null || !seen.add(p.getAbsolutePath())) continue;
            place(v.getDescription(this), p, FI.USB);
        }
    }
    Place activePlace() {
        Place best = null;
        if (dir == null) return null;
        for (Place p : places)
            if (FU.under(dir, p.dir) && (best == null || p.dir.getAbsolutePath().length() > best.dir.getAbsolutePath().length())) best = p;
        return best;
    }
    void renderSide() {
        if (side == null) return;
        side.removeAllViews();
        Place act = activePlace();
        for (final Place p : places) {
            if (p.glyph == FI.USB || p.glyph == FI.TRASH) {
                boolean firstUsb = p.glyph == FI.USB && places.indexOf(p) > 0 && places.get(places.indexOf(p) - 1).glyph != FI.USB;
                if (firstUsb || p.glyph == FI.TRASH) {
                    View line = new View(this);
                    line.setBackgroundColor(T.chip);
                    LinearLayout.LayoutParams ll = new LinearLayout.LayoutParams(-1, Math.max(1, T.px(1)));
                    ll.topMargin = ll.bottomMargin = T.px(6);
                    ll.leftMargin = ll.rightMargin = T.px(6);
                    side.addView(line, ll);
                }
            }
            LinearLayout row = new LinearLayout(this);
            row.setGravity(Gravity.CENTER_VERTICAL);
            row.setPadding(T.px(compact ? 9 : 10), 0, T.px(4), 0);
            row.setBackground(p == act ? new LayerDrawable(new Drawable[]{T.round(T.chip, T.px(10)), T.state(T.px(10))}) : T.state(T.px(10)));
            ImageView iv = new ImageView(this);
            iv.setImageDrawable(new FI.Glyph(p.glyph, p == act ? T.accent : T.text));
            row.addView(iv, new LinearLayout.LayoutParams(T.px(18), T.px(18)));
            if (!compact) {
                TextView t = text(p.label, 13, T.text);
                t.setSingleLine(true);
                t.setEllipsize(TextUtils.TruncateAt.END);
                t.setPadding(T.px(11), 0, T.px(4), 0);
                row.addView(t, new LinearLayout.LayoutParams(0, -2, 1));
                if (p.volId != null) {
                    ImageView ej = button(FI.EJECT, L.t("弹出", "Извлечь", "Eject"), () -> eject(p));
                    ej.setPadding(T.px(6), T.px(6), T.px(6), T.px(6));
                    row.addView(ej, new LinearLayout.LayoutParams(T.px(28), T.px(28)));
                }
            }
            row.setContentDescription(p.label);
            if (Build.VERSION.SDK_INT >= 26) row.setTooltipText(p.label);
            row.setFocusable(false);
            row.setOnClickListener(v -> safe(() -> {
                if (!p.dir.isDirectory() && p.glyph != FI.TRASH) p.dir.mkdirs();
                go(p.dir, true);
            }));
            row.setOnContextClickListener(v -> {
                safe(() -> {
                    if (p.volId == null && p.glyph != FI.TRASH) return;
                    PopupMenu m = new PopupMenu(themed(), v);
                    if (p.volId != null) m.getMenu().add(0, 1, 0, L.t("弹出", "Извлечь", "Eject"));
                    else m.getMenu().add(0, 2, 0, L.t("清空回收站", "Очистить корзину", "Empty trash"));
                    m.setOnMenuItemClickListener(mi -> { safe(() -> { if (mi.getItemId() == 1) eject(p); else emptyTrash(); }); return true; });
                    m.show();
                });
                return true;
            });
            LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(-1, T.px(38));
            lp.bottomMargin = T.px(2);
            side.addView(row, lp);
        }
    }
    void eject(final Place p) {
        if (p.volId == null) return;
        if (dir != null && FU.under(dir, p.dir)) { back.clear(); fwd.clear(); go(home, false); }
        final StorageManager sm = (StorageManager) getSystemService(STORAGE_SERVICE);
        new Thread(() -> {
            String err = null;
            try { StorageManager.class.getMethod("unmount", String.class).invoke(sm, p.volId); }
            catch (Throwable e) { err = String.valueOf(e.getCause() != null ? e.getCause() : e); Log.w(TAG, "eject: " + err); }
            final String msg = err == null ? p.label + L.t(" 已弹出，可以拔下了", ": можно извлечь", " ejected, safe to remove")
                : L.t("弹出失败，请到系统设置的「存储」里操作", "Не удалось извлечь — используйте «Хранилище» в настройках", "Could not eject — use Storage in system settings");
            ui.post(() -> safe(() -> { FileOps.toast(Files.this, msg); refreshPlaces(); }));
        }, "eject").start();
    }

    // ================= 路径栏与导航 =================
    boolean isRoot(File d) {
        for (Place p : places) if ((p.glyph == FI.HOME || p.glyph == FI.USB || p.glyph == FI.TRASH) && p.dir.equals(d)) return true;
        return d.getParentFile() == null;
    }
    void renderCrumbs() {
        crumbs.removeAllViews();
        Place rootPlace = null;
        for (Place p : places)
            if ((p.glyph == FI.HOME || p.glyph == FI.USB || p.glyph == FI.TRASH) && FU.under(dir, p.dir)
                && (rootPlace == null || p.dir.getAbsolutePath().length() > rootPlace.dir.getAbsolutePath().length())) rootPlace = p;
        List<File> chain = new ArrayList<>();
        File stop = rootPlace != null ? rootPlace.dir : null;
        for (File d = dir; d != null; d = d.getParentFile()) {
            chain.add(0, d);
            if (d.equals(stop)) break;
        }
        for (int i = 0; i < chain.size(); i++) {
            final File d = chain.get(i);
            boolean last = i == chain.size() - 1 && deep == null;
            if (i > 0) crumbs.addView(text("›", 13, T.text2));
            String label = d.equals(stop) ? rootPlace.label : d.getParentFile() == null ? "/" : d.getName();
            TextView t = text(label, 13, last ? T.text : T.text2);
            if (last) t.setTypeface(Typeface.DEFAULT_BOLD);
            t.setSingleLine(true);
            t.setGravity(Gravity.CENTER_VERTICAL);
            t.setPadding(T.px(8), 0, T.px(8), 0);
            t.setBackground(T.state(T.px(12)));
            t.setFocusable(false);
            t.setOnClickListener(v -> safe(() -> go(d, true)));
            crumbs.addView(t, new LinearLayout.LayoutParams(-2, T.px(26)));
        }
        if (deep != null) {
            crumbs.addView(text("›", 13, T.text2));
            TextView t = text(L.t("搜索：", "Поиск: ", "Search: ") + deep, 13, T.text);
            t.setTypeface(Typeface.DEFAULT_BOLD);
            t.setPadding(T.px(8), 0, T.px(8), 0);
            crumbs.addView(t);
        }
        crumbScroll.post(() -> crumbScroll.fullScroll(View.FOCUS_RIGHT));
        enable(backBtn, !back.isEmpty());
        enable(fwdBtn, !fwd.isEmpty());
        enable(upBtn, deep != null || !isRoot(dir));
    }
    void go(File d, boolean push) {
        if (d == null) return;
        if (push && dir != null && (!d.equals(dir) || deep != null)) {
            if (!d.equals(dir)) { back.add(dir); if (back.size() > 60) back.remove(0); }
            fwd.clear();
        }
        dir = d;
        deep = null;
        muteSearch = true;
        search.setText("");
        muteSearch = false;
        sel.clear();
        cursor = anchorPos = -1;
        raw = new ArrayList<>();
        shown = new ArrayList<>();
        loading = true;
        notifyData();
        renderCrumbs();
        renderSide();
        watch();
        root.requestFocus();
        list.setSelection(0);
        gridv.setSelection(0);
        load();
    }
    void goBack() {
        if (back.isEmpty()) return;
        File from = dir;
        fwd.add(dir);
        go(back.remove(back.size() - 1), false);
        if (dir.equals(from.getParentFile())) pendingSelect = from.getAbsolutePath();
    }
    void goFwd() {
        if (fwd.isEmpty()) return;
        back.add(dir);
        go(fwd.remove(fwd.size() - 1), false);
    }
    void goUp() {
        if (deep != null) { exitSearch(); return; }
        if (isRoot(dir)) return;
        File from = dir;
        pendingSelect = from.getAbsolutePath();
        go(dir.getParentFile(), true);
        pendingSelect = from.getAbsolutePath();
    }
    public void onBackPressed() {
        try {
            if (deep != null || search.getText().length() > 0) { exitSearch(); return; }
            if (!back.isEmpty()) { goBack(); return; }
        } catch (Throwable e) { Log.w(TAG, "back: " + e); }
        super.onBackPressed();
    }
    void watch() {
        try { if (watcher != null) watcher.stopWatching(); } catch (Throwable ignored) {}
        watcher = null;
        if (dir == null || deep != null) return;
        try {
            watcher = new FileObserver(dir.getPath(), FileObserver.CREATE | FileObserver.DELETE | FileObserver.MOVED_FROM
                | FileObserver.MOVED_TO | FileObserver.CLOSE_WRITE) {
                public void onEvent(int ev, String p) { ui.removeCallbacks(reload); ui.postDelayed(reload, 400); }
            };
            watcher.startWatching();
        } catch (Throwable e) { Log.w(TAG, "watch: " + e); }
    }
    final Runnable reload = () -> safe(this::load);

    // ================= 读取与排序 =================
    Entry entry(File f) {
        Entry e = new Entry();
        e.f = f; e.path = f.getAbsolutePath(); e.name = f.getName();
        e.dir = f.isDirectory();
        e.size = e.dir ? -1 : f.length();
        e.time = f.lastModified();
        e.low = e.name.toLowerCase(Locale.ROOT);
        return e;
    }
    void load() {
        final int g = ++loadGen;
        final File d = dir;
        final String q = deep;
        final boolean trash = FileOps.trashDir().equals(d), hid = hidden;
        final Context app = getApplicationContext();
        io.execute(() -> {
            final List<Entry> out = new ArrayList<>();
            boolean ok = true, cut = false;
            try {
                if (q != null) {
                    List<File> found = Find.walk(d, q, 60000, 400, 6000, hid, () -> g != loadGen);
                    cut = found.size() >= 400;
                    int base = d.getAbsolutePath().length();
                    for (File f : found) {
                        Entry e = entry(f);
                        String parent = f.getParent();
                        e.sub = parent != null && parent.length() > base ? parent.substring(base + 1) : null;
                        out.add(e);
                    }
                } else {
                    File[] fs = d.listFiles();
                    if (fs == null) ok = trash && !d.exists();
                    else {
                        org.json.JSONObject idx = trash ? FileOps.trashIndex(app) : null;
                        for (File f : fs) {
                            if (g != loadGen) return;
                            Entry e = entry(f);
                            if (trash) {
                                if (e.name.equals(".nomedia")) continue;
                                String orig = idx.optString(e.name, null);
                                e.name = FileOps.trashedName(e.name);
                                e.low = e.name.toLowerCase(Locale.ROOT);
                                e.sub = orig != null ? new File(orig).getParent() : null;
                            }
                            out.add(e);
                        }
                    }
                }
            } catch (Throwable e) { Log.w(TAG, "list: " + e); ok = false; }
            final boolean fok = ok, fcut = cut;
            ui.post(() -> safe(() -> {
                if (g != loadGen) return;
                raw = out; readable = fok; loading = false; deepCut = fcut;
                applyFilter();
            }));
        });
    }
    boolean inTrash() { return dir != null && FileOps.trashDir().equals(dir); }
    void applyFilter() {
        String f = deep == null ? search.getText().toString().trim().toLowerCase(Locale.ROOT) : "";
        boolean trash = inTrash();
        List<Entry> out = new ArrayList<>();
        for (Entry e : raw) {
            if (!hidden && !trash && e.name.startsWith(".")) continue;
            if (!f.isEmpty() && !e.low.contains(f)) continue;
            out.add(e);
        }
        final int s = sort;
        final boolean ds = desc;
        Collections.sort(out, (x, y) -> {
            if (x.dir != y.dir) return x.dir ? -1 : 1;
            int k = 0;
            if (s == 1) k = Long.compare(x.size, y.size);
            else if (s == 2) k = Long.compare(x.time, y.time);
            else if (s == 3) k = FU.ext(x.low).compareTo(FU.ext(y.low));
            if (k == 0) k = FU.natural(x.low, y.low);
            return ds ? -k : k;
        });
        String keep = cursor >= 0 && cursor < shown.size() ? shown.get(cursor).path : null;
        shown = out;
        Set<String> present = new HashSet<>();
        for (Entry e : out) present.add(e.path);
        sel.retainAll(present);
        cursor = -1;
        String want = pendingSelect != null ? pendingSelect : keep;
        if (want != null) for (int i = 0; i < out.size(); i++) if (out.get(i).path.equals(want)) { cursor = i; break; }
        if (pendingSelect != null && !loading) {
            if (cursor >= 0) { sel.clear(); sel.add(pendingSelect); anchorPos = cursor; final int at = cursor; ui.post(() -> safe(() -> ensureVisible(at))); }
            pendingSelect = null;
        }
        if (anchorPos >= out.size()) anchorPos = -1;
        notifyData();
    }
    void notifyData() {
        listAd.notifyDataSetChanged();
        gridAd.notifyDataSetChanged();
        updateStatus();
    }
    void updateStatus() {
        String e = "";
        if (loading && shown.isEmpty()) e = deep != null ? L.t("正在搜索…", "Идёт поиск…", "Searching…") : L.t("正在读取…", "Загрузка…", "Loading…");
        else if (!readable) e = L.t("无法读取这个文件夹", "Не удалось открыть эту папку", "This folder can't be read");
        else if (shown.isEmpty()) {
            if (deep != null || search.getText().length() > 0) e = L.t("没有匹配的文件", "Ничего не найдено", "No matching files");
            else if (inTrash()) e = L.t("回收站是空的", "Корзина пуста", "Trash is empty");
            else e = L.t("这个文件夹是空的", "Эта папка пуста", "This folder is empty");
        }
        empty.setText(e);
        empty.setVisibility(e.isEmpty() ? View.GONE : View.VISIBLE);
        StringBuilder s = new StringBuilder();
        s.append(shown.size()).append(L.t(" 项", " объект(ов)", shown.size() == 1 ? " item" : " items"));
        if (deepCut && deep != null) s.append(L.t("（只显示前 400 个）", " (первые 400)", " (first 400 shown)"));
        if (!sel.isEmpty()) {
            long sum = 0;
            boolean files = false;
            for (Entry x : shown) if (!x.dir && sel.contains(x.path)) { sum += x.size; files = true; }
            s.append(L.t("  ·  已选 ", "  ·  выбрано: ", "  ·  ")).append(sel.size()).append(L.t(" 项", "", " selected"));
            if (files) s.append("  ·  ").append(FU.size(sum));
        }
        if (!clip.isEmpty()) s.append("  ·  ").append(clipCut ? L.t("已剪切 ", "вырезано: ", "cut: ") : L.t("已复制 ", "скопировано: ", "copied: ")).append(clip.size());
        status.setText(s);
        String fr = "";
        try {
            if (dir != null && deep == null) { long u = dir.getUsableSpace(); if (u > 0) fr = L.t("可用 ", "Свободно ", "Free ") + FU.size(u); }
        } catch (Throwable ignored) {}
        free.setText(fr);
    }
    void renderHeader() {
        String[] names = {L.t("名称", "Имя", "Name"), L.t("大小", "Размер", "Size"), L.t("修改时间", "Изменён", "Modified"), L.t("类型", "Тип", "Type")};
        int[] cols = {0, 1, 3, 2};
        for (int i = 0; i < 4; i++) {
            TextView t = (TextView) header.getChildAt(i);
            int col = cols[i];
            t.setText(names[col] + (sort == col ? (desc ? "  ▼" : "  ▲") : ""));
            t.setTextColor(sort == col ? T.text : T.text2);
            t.setVisibility(col == 3 && !showType || col == 2 && !showDate ? View.GONE : View.VISIBLE);
        }
    }
    void sortBy(int col) {
        if (sort == col) desc = !desc;
        else { sort = col; desc = col == 2 || col == 1; }
        sp.edit().putInt("sort", sort).putBoolean("desc", desc).apply();
        renderHeader();
        applyFilter();
    }
    void setGrid(boolean g) {
        grid = g;
        sp.edit().putBoolean("grid", g).apply();
        list.setVisibility(g ? View.GONE : View.VISIBLE);
        gridv.setVisibility(g ? View.VISIBLE : View.GONE);
        header.setVisibility(g ? View.GONE : View.VISIBLE);
        viewBtn.setImageDrawable(new FI.Glyph(g ? FI.LIST : FI.GRID, T.text));
        String tip = g ? L.t("列表视图 (Ctrl+1)", "Список (Ctrl+1)", "List view (Ctrl+1)") : L.t("图标视图 (Ctrl+2)", "Значки (Ctrl+2)", "Icon view (Ctrl+2)");
        viewBtn.setContentDescription(tip);
        if (Build.VERSION.SDK_INT >= 26) viewBtn.setTooltipText(tip);
        notifyData();
        final int at = cursor;
        if (at >= 0) ui.post(() -> safe(() -> ensureVisible(at)));
    }

    // ================= 列表与图标视图 =================
    final class Ad extends BaseAdapter {
        final boolean g;
        Ad(boolean g) { this.g = g; }
        public int getCount() { return shown.size(); }
        public Object getItem(int i) { return shown.get(i); }
        public long getItemId(int i) { return i; }
        public View getView(int pos, View v, ViewGroup parent) {
            try { return g ? cell(pos, v) : row(pos, v); }
            catch (Throwable t) { Log.w(TAG, "bind: " + t); return v != null ? v : new View(Files.this); }
        }
    }
    void wire(View v, final H h) {
        v.setTag(h);
        v.setFocusable(false);
        v.setOnClickListener(x -> safe(() -> click(h.pos)));
        v.setOnLongClickListener(x -> { safe(() -> menu(h.pos)); return true; });
        v.setOnContextClickListener(x -> { safe(() -> menu(h.pos)); return true; });
    }
    CharSequence nameOf(Entry e) {
        if (e.sub == null || e.sub.isEmpty()) return e.name;
        SpannableString s = new SpannableString(e.name + "   " + e.sub);
        int from = e.name.length();
        s.setSpan(new ForegroundColorSpan(T.text2), from, s.length(), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
        s.setSpan(new RelativeSizeSpan(0.82f), from, s.length(), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
        return s;
    }
    boolean isCut(Entry e) {
        if (!clipCut || clip.isEmpty()) return false;
        for (File f : clip) if (f.getAbsolutePath().equals(e.path)) return true;
        return false;
    }
    View row(int pos, View v) {
        LinearLayout row = (LinearLayout) v;
        H h;
        if (row == null) {
            row = new LinearLayout(this);
            row.setGravity(Gravity.CENTER_VERTICAL);
            row.setPadding(T.px(10), 0, T.px(10), 0);
            row.setBackground(itemBg(T.px(8)));
            row.setLayoutParams(new AbsListView.LayoutParams(-1, T.px(ROW)));
            h = new H();
            h.icon = new ImageView(this);
            h.icon.setScaleType(ImageView.ScaleType.FIT_CENTER);
            LinearLayout.LayoutParams il = new LinearLayout.LayoutParams(T.px(26), T.px(26));
            il.rightMargin = T.px(12);
            row.addView(h.icon, il);
            h.name = text("", 13.5f, T.text);
            h.name.setSingleLine(true);
            row.addView(h.name, new LinearLayout.LayoutParams(0, -2, 1));
            h.size = text("", 12, T.text2);
            h.size.setSingleLine(true);
            h.size.setGravity(Gravity.RIGHT);
            h.size.setPadding(T.px(8), 0, T.px(8), 0);
            row.addView(h.size, new LinearLayout.LayoutParams(colWidth(1), -2));
            h.type = text("", 12, T.text2);
            h.type.setSingleLine(true);
            h.type.setEllipsize(TextUtils.TruncateAt.END);
            h.type.setPadding(T.px(8), 0, T.px(8), 0);
            row.addView(h.type, new LinearLayout.LayoutParams(colWidth(3), -2));
            h.date = text("", 12, T.text2);
            h.date.setSingleLine(true);
            h.date.setPadding(T.px(8), 0, T.px(8), 0);
            row.addView(h.date, new LinearLayout.LayoutParams(colWidth(2), -2));
            wire(row, h);
        } else h = (H) row.getTag();
        Entry e = shown.get(pos);
        h.pos = pos;
        FI.bind(h.icon, e.f, e.name, e.dir, e.time, T.px(40));
        h.name.setEllipsize(e.sub == null ? TextUtils.TruncateAt.MIDDLE : TextUtils.TruncateAt.END);
        h.name.setText(nameOf(e));
        h.size.setText(e.dir ? "" : FU.size(e.size));
        h.type.setText(FU.typeLabel(e.name, e.dir));
        h.date.setText(FU.date(e.time));
        h.type.setVisibility(showType ? View.VISIBLE : View.GONE);
        h.date.setVisibility(showDate ? View.VISIBLE : View.GONE);
        row.setActivated(sel.contains(e.path));
        row.setAlpha(isCut(e) ? 0.45f : 1f);
        return row;
    }
    View cell(int pos, View v) {
        LinearLayout cell = (LinearLayout) v;
        H h;
        if (cell == null) {
            cell = new LinearLayout(this);
            cell.setOrientation(LinearLayout.VERTICAL);
            cell.setGravity(Gravity.CENTER_HORIZONTAL);
            cell.setPadding(T.px(6), T.px(8), T.px(6), T.px(6));
            cell.setBackground(itemBg(T.px(10)));
            cell.setLayoutParams(new AbsListView.LayoutParams(-1, -2));
            h = new H();
            h.icon = new ImageView(this);
            h.icon.setScaleType(ImageView.ScaleType.FIT_CENTER);
            cell.addView(h.icon, new LinearLayout.LayoutParams(T.px(76), T.px(68)));
            h.name = text("", 12, T.text);
            h.name.setLines(2);
            h.name.setEllipsize(TextUtils.TruncateAt.END);
            h.name.setGravity(Gravity.CENTER_HORIZONTAL);
            h.name.setPadding(0, T.px(6), 0, 0);
            h.name.setLineSpacing(T.px(2), 1f);
            cell.addView(h.name, new LinearLayout.LayoutParams(-1, -2));
            wire(cell, h);
        } else h = (H) cell.getTag();
        Entry e = shown.get(pos);
        h.pos = pos;
        FI.bind(h.icon, e.f, e.name, e.dir, e.time, T.px(112));
        h.name.setText(e.name);
        if (Build.VERSION.SDK_INT >= 26) cell.setTooltipText(e.sub == null ? e.name : e.name + "\n" + e.sub);
        cell.setActivated(sel.contains(e.path));
        cell.setAlpha(isCut(e) ? 0.45f : 1f);
        return cell;
    }
    AbsListView active() { return grid ? gridv : list; }

    // ================= 鼠标 =================
    public boolean dispatchTouchEvent(MotionEvent ev) {
        try {
            lastX = ev.getX(); lastY = ev.getY();
            if (ev.getActionMasked() == MotionEvent.ACTION_DOWN) {
                downMeta = ev.getMetaState();
                downButtons = ev.getButtonState();
                if (search.hasFocus()) {
                    int[] loc = new int[2];
                    search.getLocationInWindow(loc);
                    if (lastX < loc[0] || lastX > loc[0] + search.getWidth() || lastY < loc[1] || lastY > loc[1] + search.getHeight()) root.requestFocus();
                }
            }
            return super.dispatchTouchEvent(ev);
        } catch (Throwable e) { Log.w(TAG, "touch: " + e, e); return true; }
    }
    public boolean dispatchGenericMotionEvent(MotionEvent ev) {
        try {
            lastX = ev.getX(); lastY = ev.getY();
            return super.dispatchGenericMotionEvent(ev);
        } catch (Throwable e) { Log.w(TAG, "motion: " + e, e); return true; }
    }
    /** 点在列表的空白处：取消选择。 */
    void blankTouch(MotionEvent ev) {
        int a = ev.getActionMasked();
        if (a == MotionEvent.ACTION_DOWN) blankY = ev.getY();
        else if (a == MotionEvent.ACTION_UP && Math.abs(ev.getY() - blankY) < T.px(8)
            && (downButtons & MotionEvent.BUTTON_SECONDARY) == 0 && (downMeta & (KeyEvent.META_CTRL_ON | KeyEvent.META_SHIFT_ON)) == 0) {
            if (!sel.isEmpty()) { sel.clear(); notifyData(); }
        }
    }
    void click(int pos) {
        if (pos < 0 || pos >= shown.size()) return;
        if ((downButtons & (MotionEvent.BUTTON_SECONDARY | MotionEvent.BUTTON_TERTIARY)) != 0) return;
        boolean ctrl = (downMeta & KeyEvent.META_CTRL_ON) != 0, shift = (downMeta & KeyEvent.META_SHIFT_ON) != 0;
        long now = SystemClock.uptimeMillis();
        if (!ctrl && !shift && pos == lastClickPos && now - lastClickTime < 450) {
            lastClickTime = 0;
            open(shown.get(pos));
            return;
        }
        lastClickPos = pos; lastClickTime = now;
        Entry e = shown.get(pos);
        if (shift && anchorPos >= 0 && anchorPos < shown.size()) range(anchorPos, pos, ctrl);
        else if (ctrl) { if (!sel.remove(e.path)) sel.add(e.path); anchorPos = pos; }
        else { sel.clear(); sel.add(e.path); anchorPos = pos; }
        cursor = pos;
        notifyData();
    }
    void range(int a, int b, boolean add) {
        if (!add) sel.clear();
        for (int i = Math.min(a, b); i <= Math.max(a, b) && i < shown.size(); i++) sel.add(shown.get(i).path);
    }
    void moveTo(int pos, boolean shift) {
        if (shown.isEmpty()) return;
        pos = Math.max(0, Math.min(shown.size() - 1, pos));
        if (shift && anchorPos >= 0) range(anchorPos, pos, false);
        else { sel.clear(); sel.add(shown.get(pos).path); anchorPos = pos; }
        cursor = pos;
        notifyData();
        ensureVisible(pos);
    }
    void ensureVisible(int pos) {
        if (pos < 0 || pos >= shown.size()) return;
        if (grid) { gridv.smoothScrollToPosition(pos); return; }
        int first = list.getFirstVisiblePosition(), last = list.getLastVisiblePosition();
        if (pos <= first) list.setSelection(pos);
        else if (pos >= last) list.setSelectionFromTop(pos, Math.max(0, list.getHeight() - T.px(ROW) - list.getPaddingBottom()));
    }
    List<Entry> picked() {
        List<Entry> out = new ArrayList<>();
        for (Entry e : shown) if (sel.contains(e.path)) out.add(e);
        return out;
    }
    List<File> pickedFiles() {
        List<File> out = new ArrayList<>();
        for (Entry e : picked()) out.add(e.f);
        return out;
    }

    // ================= 键盘 =================
    public boolean dispatchKeyEvent(KeyEvent e) {
        try {
            if (e.getAction() == KeyEvent.ACTION_DOWN && root != null && key(e)) return true;
        } catch (Throwable t) { Log.w(TAG, "key: " + t, t); return true; }
        return super.dispatchKeyEvent(e);
    }
    boolean key(KeyEvent e) {
        int c = e.getKeyCode();
        boolean ctrl = e.isCtrlPressed(), shift = e.isShiftPressed(), alt = e.isAltPressed();
        if (search.hasFocus()) {
            if (c == KeyEvent.KEYCODE_ESCAPE) { exitSearch(); return true; }
            if (c == KeyEvent.KEYCODE_ENTER || c == KeyEvent.KEYCODE_NUMPAD_ENTER) { deepSearch(); return true; }
            if (c == KeyEvent.KEYCODE_DPAD_DOWN || c == KeyEvent.KEYCODE_TAB) {
                root.requestFocus();
                if (!shown.isEmpty()) moveTo(Math.max(0, cursor), false);
                return true;
            }
            if (ctrl && c == KeyEvent.KEYCODE_F) { search.selectAll(); return true; }
            return false;
        }
        if (e.isMetaPressed()) return false;
        if (ctrl) {
            switch (c) {
                case KeyEvent.KEYCODE_A: selectAll(); return true;
                case KeyEvent.KEYCODE_C: copy(false); return true;
                case KeyEvent.KEYCODE_X: copy(true); return true;
                case KeyEvent.KEYCODE_V: paste(dir); return true;
                case KeyEvent.KEYCODE_F: search.requestFocus(); search.selectAll(); return true;
                case KeyEvent.KEYCODE_H: toggleHidden(); return true;
                case KeyEvent.KEYCODE_L: goToPath(); return true;
                case KeyEvent.KEYCODE_R: load(); return true;
                case KeyEvent.KEYCODE_N: if (shift) { newFolder(); return true; } return false;
                case KeyEvent.KEYCODE_1: setGrid(false); return true;
                case KeyEvent.KEYCODE_2: setGrid(true); return true;
                default: return false;
            }
        }
        if (alt) {
            switch (c) {
                case KeyEvent.KEYCODE_DPAD_LEFT: if (!back.isEmpty()) goBack(); else goUp(); return true;
                case KeyEvent.KEYCODE_DPAD_RIGHT: goFwd(); return true;
                case KeyEvent.KEYCODE_DPAD_UP: goUp(); return true;
                case KeyEvent.KEYCODE_ENTER: props(); return true;
                default: return false;
            }
        }
        int cols = grid ? Math.max(1, gridv.getNumColumns()) : 1;
        int cur = cursor < 0 ? -1 : cursor;
        switch (c) {
            case KeyEvent.KEYCODE_DPAD_DOWN: moveTo(cur < 0 ? 0 : cur + cols >= shown.size() ? (grid ? cur : shown.size() - 1) : cur + cols, shift); return true;
            case KeyEvent.KEYCODE_DPAD_UP: moveTo(cur < 0 ? 0 : cur - cols < 0 ? cur : cur - cols, shift); return true;
            case KeyEvent.KEYCODE_DPAD_RIGHT: if (grid) moveTo(cur + 1, shift); return true;
            case KeyEvent.KEYCODE_DPAD_LEFT: if (grid) moveTo(Math.max(0, cur - 1), shift); return true;
            case KeyEvent.KEYCODE_MOVE_HOME: moveTo(0, shift); return true;
            case KeyEvent.KEYCODE_MOVE_END: moveTo(shown.size() - 1, shift); return true;
            case KeyEvent.KEYCODE_PAGE_DOWN: moveTo(Math.max(0, cur) + page(cols), shift); return true;
            case KeyEvent.KEYCODE_PAGE_UP: moveTo(Math.max(0, cur) - page(cols), shift); return true;
            case KeyEvent.KEYCODE_ENTER:
            case KeyEvent.KEYCODE_NUMPAD_ENTER:
                if (e.getRepeatCount() == 0 && cur >= 0 && cur < shown.size()) open(shown.get(cur));
                return true;
            case KeyEvent.KEYCODE_DEL: if (e.getRepeatCount() == 0) goUp(); return true;
            case KeyEvent.KEYCODE_FORWARD_DEL: if (e.getRepeatCount() == 0) delete(shift); return true;
            case KeyEvent.KEYCODE_F2: rename(); return true;
            case KeyEvent.KEYCODE_F5: load(); return true;
            case KeyEvent.KEYCODE_FORWARD: goFwd(); return true;
            case KeyEvent.KEYCODE_MENU: menuAtCursor(); return true;
            case KeyEvent.KEYCODE_F10: if (shift) { menuAtCursor(); return true; } return false;
            case KeyEvent.KEYCODE_ESCAPE:
                if (deep != null || search.getText().length() > 0) exitSearch();
                else if (!sel.isEmpty()) { sel.clear(); notifyData(); }
                else if (clipCut && !clip.isEmpty()) { clip.clear(); clipCut = false; notifyData(); }
                return true;
        }
        // 直接打字：跳到名字以这些字开头的文件
        int ch = e.getUnicodeChar();
        long now = SystemClock.uptimeMillis();
        if (now - typedAt > 900) typed = "";
        if (ch > 32 && !Character.isISOControl(ch) || ch == 32 && !typed.isEmpty()) {
            typed += new String(Character.toChars(ch)).toLowerCase(Locale.ROOT);
            typedAt = now;
            for (int pass = 0; pass < 2; pass++)
                for (int i = 0; i < shown.size(); i++) {
                    String n = shown.get(i).low;
                    if (pass == 0 ? n.startsWith(typed) : n.contains(typed)) { moveTo(i, false); return true; }
                }
            return true;
        }
        return false;
    }
    int page(int cols) {
        AbsListView v = active();
        int vis = Math.max(1, v.getLastVisiblePosition() - v.getFirstVisiblePosition());
        return grid ? Math.max(cols, vis / cols * cols) : vis;
    }

    // ================= 搜索 =================
    void deepSearch() {
        String q = search.getText().toString().trim();
        if (q.isEmpty()) { exitSearch(); return; }
        deep = q;
        root.requestFocus(); // 结果出来后方向键、回车直接作用在列表上
        sel.clear();
        cursor = anchorPos = -1;
        raw = new ArrayList<>();
        shown = new ArrayList<>();
        loading = true;
        notifyData();
        renderCrumbs();
        watch();
        load();
    }
    void exitSearch() {
        boolean was = deep != null;
        deep = null;
        muteSearch = true;
        search.setText("");
        muteSearch = false;
        root.requestFocus();
        if (was) { renderCrumbs(); watch(); loading = true; raw = new ArrayList<>(); load(); }
        applyFilter();
    }

    // ================= 动作 =================
    void open(Entry e) {
        if (inTrash()) { props(); return; }
        if (e.dir) { go(e.f, true); return; }
        view(this, e.f, false);
    }
    void selectAll() {
        for (Entry e : shown) sel.add(e.path);
        notifyData();
    }
    void toggleHidden() {
        hidden = !hidden;
        sp.edit().putBoolean("hidden", hidden).apply();
        applyFilter();
    }
    void copy(boolean cut) {
        List<File> fs = pickedFiles();
        if (fs.isEmpty() || inTrash()) return;
        clip.clear();
        clip.addAll(fs);
        clipCut = cut;
        notifyData();
    }
    void paste(File into) {
        if (clip.isEmpty() || into == null || inTrash() || deep != null && into.equals(dir)) return;
        final boolean cut = clipCut;
        final List<File> src = new ArrayList<>(clip);
        FileOps.transfer(this, src, into, cut, (made, errors) -> {
            if (cut) { clip.clear(); clipCut = false; }
            sel.clear();
            for (File f : made) sel.add(f.getAbsolutePath());
            if (made.size() == 1) pendingSelect = made.get(0).getAbsolutePath();
            load();
        });
    }
    void delete(boolean permanent) {
        final List<File> fs = pickedFiles();
        if (fs.isEmpty()) return;
        boolean trashable = !permanent && !inTrash();
        if (trashable) for (File f : fs) if (!FileOps.canTrash(f)) { trashable = false; break; }
        final boolean toTrash = trashable;
        String what = fs.size() == 1 ? "「" + fs.get(0).getName() + "」" : fs.size() + L.t(" 个项目", " объект(ов)", " items");
        if (L.lang != 0 && fs.size() == 1) what = "\"" + fs.get(0).getName() + "\"";
        String msg = toTrash ? L.t("把" + what + "移到回收站？", "Переместить " + what + " в корзину?", "Move " + what + " to Trash?")
            : L.t("永久删除" + what + "？此操作无法撤销。", "Удалить " + what + " навсегда? Это действие нельзя отменить.", "Permanently delete " + what + "? This can't be undone.");
        enterConfirms(new AlertDialog.Builder(themed()).setTitle(L.t("删除", "Удаление", "Delete")).setMessage(msg)
            .setPositiveButton(toTrash ? L.t("移到回收站", "В корзину", "Move to Trash") : L.t("永久删除", "Удалить", "Delete"),
                (d, w) -> safe(() -> FileOps.delete(Files.this, fs, toTrash, (made, errors) -> load())))
            .setNegativeButton(L.t("取消", "Отмена", "Cancel"), null).show());
    }
    void emptyTrash() {
        final File[] all = FileOps.trashDir().listFiles();
        if (all == null || all.length == 0) { FileOps.toast(this, L.t("回收站是空的", "Корзина пуста", "Trash is empty")); return; }
        enterConfirms(new AlertDialog.Builder(themed()).setTitle(L.t("清空回收站", "Очистить корзину", "Empty trash"))
            .setMessage(L.t("永久删除回收站里的全部内容？此操作无法撤销。", "Удалить всё содержимое корзины навсегда? Это действие нельзя отменить.", "Permanently delete everything in Trash? This can't be undone."))
            .setPositiveButton(L.t("清空", "Очистить", "Empty"), (d, w) -> safe(() -> {
                List<File> fs = new ArrayList<>();
                for (File f : all) if (!f.getName().equals(".nomedia")) fs.add(f);
                FileOps.delete(Files.this, fs, false, (made, errors) -> load());
            }))
            .setNegativeButton(L.t("取消", "Отмена", "Cancel"), null).show());
    }
    /** 确认框一出来就把焦点放在确定按钮上：按回车即确认，Esc 取消。 */
    static void enterConfirms(AlertDialog d) {
        try {
            Button b = d.getButton(DialogInterface.BUTTON_POSITIVE);
            if (b == null) return;
            b.setFocusableInTouchMode(true);
            b.requestFocus();
        } catch (Throwable ignored) {}
    }
    void restore() {
        List<File> fs = pickedFiles();
        if (fs.isEmpty()) return;
        FileOps.restore(this, fs, (made, errors) -> load());
    }
    void ask(String title, String initial, int selEnd, String ok, final Str cb) {
        final EditText in = new EditText(themed());
        in.setSingleLine(true);
        in.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
        in.setText(initial);
        final int end = Math.max(0, Math.min(initial.length(), selEnd));
        FrameLayout box = new FrameLayout(this);
        box.setPadding(T.px(20), T.px(8), T.px(20), 0);
        box.addView(in, new FrameLayout.LayoutParams(-1, -2));
        final AlertDialog d = new AlertDialog.Builder(themed()).setTitle(title).setView(box)
            .setPositiveButton(ok, (x, w) -> safe(() -> cb.got(in.getText().toString().trim())))
            .setNegativeButton(L.t("取消", "Отмена", "Cancel"), null).create();
        in.setOnEditorActionListener((v, id, ev) -> {
            if (ev != null && ev.getAction() != KeyEvent.ACTION_DOWN) return true;
            d.dismiss();
            safe(() -> cb.got(in.getText().toString().trim()));
            return true;
        });
        // 没有实体键盘时才主动叫出屏幕键盘
        if (d.getWindow() != null && getResources().getConfiguration().keyboard == android.content.res.Configuration.KEYBOARD_NOKEYS)
            d.getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_STATE_VISIBLE);
        d.show();
        in.requestFocus();
        // 取得焦点时系统会把光标挪到末尾，所以选区要等它之后再设（重命名时只选中扩展名前面的部分）
        final Runnable pick = () -> {
            try { if (in.getText().toString().equals(initial) && !in.hasSelection()) in.setSelection(0, Math.min(end, in.length())); } catch (Throwable ignored) {}
        };
        in.post(pick);
        in.postDelayed(pick, 300); // 有的输入法接手输入框时会把选区清掉：没动过字就再选一次
    }
    boolean badName(String n) {
        if (n.isEmpty() || n.contains("/") || n.equals(".") || n.equals("..")) {
            FileOps.toast(this, L.t("这个名字不能用", "Такое имя использовать нельзя", "That name can't be used"));
            return true;
        }
        return false;
    }
    void rename() {
        List<Entry> es = picked();
        if (es.size() != 1 || inTrash()) return;
        final Entry e = es.get(0);
        int dot = e.dir ? -1 : e.name.lastIndexOf('.');
        ask(L.t("重命名", "Переименовать", "Rename"), e.name, dot > 0 ? dot : e.name.length(), L.t("重命名", "Переименовать", "Rename"), n -> {
            if (n.equals(e.name) || badName(n)) return;
            File t = new File(e.f.getParentFile(), n);
            // 只改大小写时目标「已存在」其实就是它自己
            if (t.exists() && !n.equalsIgnoreCase(e.name)) { FileOps.toast(this, L.t("已有同名的项目", "Объект с таким именем уже есть", "An item with that name already exists")); return; }
            if (!e.f.renameTo(t)) { FileOps.toast(this, L.t("重命名失败", "Не удалось переименовать", "Could not rename")); return; }
            pendingSelect = t.getAbsolutePath();
            load();
        });
    }
    void newFolder() {
        if (inTrash() || deep != null) return;
        String base = L.t("新建文件夹", "Новая папка", "New folder");
        String name = FU.unique(dir, base).getName();
        ask(L.t("新建文件夹", "Новая папка", "New folder"), name, name.length(), L.t("创建", "Создать", "Create"), n -> {
            if (badName(n)) return;
            File t = new File(dir, n);
            if (t.exists()) { FileOps.toast(this, L.t("已有同名的项目", "Объект с таким именем уже есть", "An item with that name already exists")); return; }
            if (!t.mkdir()) { FileOps.toast(this, L.t("无法在这里创建文件夹", "Не удалось создать папку здесь", "Could not create a folder here")); return; }
            pendingSelect = t.getAbsolutePath();
            load();
        });
    }
    void goToPath() {
        String p = dir.getAbsolutePath();
        ask(L.t("转到文件夹", "Перейти к папке", "Go to folder"), p, p.length(), L.t("转到", "Перейти", "Go"), n -> {
            File d = norm(new File(n.startsWith("~") ? home.getAbsolutePath() + n.substring(1) : n));
            String keep = null;
            if (d.isFile()) { keep = d.getAbsolutePath(); d = d.getParentFile(); }
            if (d == null || !d.isDirectory()) { FileOps.toast(this, L.t("没有这个文件夹", "Такой папки нет", "No such folder")); return; }
            go(d, true);
            pendingSelect = keep;
        });
    }
    /** 发送到设备：交给桌面主进程里的 Send；没有它就走系统分享（优先 LocalSend）。 */
    void send() {
        ArrayList<String> paths = new ArrayList<>();
        ArrayList<Uri> uris = new ArrayList<>();
        for (Entry e : picked()) if (!e.dir) { paths.add(e.path); uris.add(FilesProvider.uri(e.f)); }
        if (paths.isEmpty()) { FileOps.toast(this, L.t("请先选中要发送的文件（不支持整个文件夹）", "Сначала выберите файлы (папки целиком не отправляются)", "Select files to send first (whole folders aren't supported)")); return; }
        try {
            Bundle ex = new Bundle();
            ex.putStringArrayList("paths", paths);
            Bundle r = getContentResolver().call(Uri.parse("content://" + FilesProvider.AUTH), "send", null, ex);
            if (r != null && r.getBoolean("ok", false)) return;
        } catch (Throwable e) { Log.w(TAG, "send: " + e); }
        try {
            Intent i;
            if (uris.size() == 1) i = new Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_STREAM, uris.get(0)).setType(FU.mime(new File(paths.get(0))));
            else i = new Intent(Intent.ACTION_SEND_MULTIPLE).putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris).setType("*/*");
            ClipData cd = ClipData.newRawUri("", uris.get(0));
            for (int k = 1; k < uris.size(); k++) cd.addItem(new ClipData.Item(uris.get(k)));
            i.setClipData(cd);
            i.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            boolean ls = false;
            try { ls = getPackageManager().getLaunchIntentForPackage(Desk.LOCALSEND) != null; } catch (Throwable ignored) {}
            if (ls) {
                try { startActivity(new Intent(i).setPackage(Desk.LOCALSEND)); return; } catch (ActivityNotFoundException ignored) {}
            }
            startActivity(Intent.createChooser(i, L.t("发送到设备", "Отправить на устройство", "Send to device")));
        } catch (Throwable e) {
            Log.w(TAG, "share: " + e);
            FileOps.toast(this, L.t("没有可以发送文件的应用", "Нет приложения для отправки файлов", "No app available to send files"));
        }
    }
    void props() {
        final List<Entry> es = picked();
        final boolean self = es.isEmpty();
        final List<File> fs = new ArrayList<>();
        StringBuilder s = new StringBuilder();
        String sep = L.t("：", ": ", ": ");
        if (self || es.size() == 1) {
            Entry e = self ? entry(dir) : es.get(0);
            fs.add(e.f);
            s.append(L.t("名称", "Имя", "Name")).append(sep).append(self ? e.f.getName() : e.name).append('\n');
            s.append(L.t("位置", "Расположение", "Location")).append(sep).append(e.f.getParent()).append('\n');
            if (e.sub != null && inTrash()) s.append(L.t("原位置", "Исходное место", "Original location")).append(sep).append(e.sub).append('\n');
            s.append(L.t("类型", "Тип", "Type")).append(sep).append(FU.typeLabel(e.name, e.dir)).append('\n');
            s.append(L.t("修改时间", "Изменён", "Modified")).append(sep).append(FU.date(e.time)).append('\n');
            if (!e.dir && FU.kind(e.name, false) == FU.APK) {
                try {
                    PackageInfo pi = getPackageManager().getPackageArchiveInfo(e.path, 0);
                    if (pi != null) s.append(L.t("应用包名", "Пакет", "Package")).append(sep).append(pi.packageName).append('\n')
                        .append(L.t("版本", "Версия", "Version")).append(sep).append(pi.versionName).append('\n');
                } catch (Throwable ignored) {}
            }
            if (!e.f.canWrite()) s.append(L.t("只读", "Только чтение", "Read-only")).append('\n');
        } else {
            for (Entry e : es) fs.add(e.f);
            s.append(es.size()).append(L.t(" 个项目", " объект(ов)", " items")).append('\n');
            s.append(L.t("位置", "Расположение", "Location")).append(sep).append(dir.getAbsolutePath()).append('\n');
        }
        final String head = s.toString();
        final String sizeLabel = L.t("大小", "Размер", "Size") + sep;
        final TextView tv = text(head + sizeLabel + "…", 13.5f, T.text);
        tv.setLineSpacing(T.px(6), 1f);
        tv.setPadding(T.px(24), T.px(14), T.px(24), T.px(4));
        tv.setTextIsSelectable(true);
        final AlertDialog d = new AlertDialog.Builder(themed()).setTitle(L.t("属性", "Свойства", "Properties")).setView(tv)
            .setPositiveButton(android.R.string.ok, null).show();
        new Thread(() -> {
            final long[] acc = new long[3]; // 字节数、文件数、文件夹数
            for (File f : fs) measure(f, acc, d);
            ui.post(() -> safe(() -> {
                if (!d.isShowing()) return;
                String extra = acc[1] + acc[2] > 1 || fs.size() == 1 && fs.get(0).isDirectory()
                    ? L.t("（" + acc[1] + " 个文件，" + acc[2] + " 个文件夹）", " (файлов: " + acc[1] + ", папок: " + acc[2] + ")", " (" + acc[1] + " files, " + acc[2] + " folders)") : "";
                tv.setText(head + sizeLabel + FU.size(acc[0]) + extra);
            }));
        }, "props").start();
    }
    void measure(File f, long[] acc, AlertDialog d) {
        if (!d.isShowing() && acc[1] + acc[2] > 200) return; // 窗口关了就别再数了
        if (f.isDirectory()) {
            File[] k = f.listFiles();
            if (k != null) for (File c : k) { if (c.isDirectory()) acc[2]++; measure(c, acc, d); }
        } else { acc[0] += f.length(); acc[1]++; }
    }

    // ================= 菜单 =================
    PopupMenu popupAt() {
        int[] loc = new int[2];
        root.getLocationInWindow(loc);
        anchor.setX(lastX - loc[0]);
        anchor.setY(lastY - loc[1]);
        return new PopupMenu(themed(), anchor, Gravity.NO_GRAVITY);
    }
    void menuAtCursor() {
        AbsListView v = active();
        View c = cursor >= 0 ? v.getChildAt(cursor - v.getFirstVisiblePosition()) : null;
        int[] loc = new int[2];
        (c != null ? c : v).getLocationInWindow(loc);
        lastX = loc[0] + T.px(48);
        lastY = loc[1] + (c != null ? c.getHeight() : T.px(20));
        menu(c != null && sel.contains(shown.get(cursor).path) ? cursor : -1);
    }
    /** pos ≥ 0：在某个条目上点右键；-1：在空白处。 */
    void menu(int pos) {
        if (pos >= shown.size()) return;
        if (pos >= 0) {
            Entry e = shown.get(pos);
            if (!sel.contains(e.path)) { sel.clear(); sel.add(e.path); anchorPos = pos; }
            cursor = pos;
            notifyData();
        }
        final List<Entry> es = pos >= 0 ? picked() : new ArrayList<Entry>();
        final List<Runnable> acts = new ArrayList<>();
        PopupMenu m = popupAt();
        Menu mm = m.getMenu();
        boolean trash = inTrash();
        final Entry one = es.size() == 1 ? es.get(0) : null;
        if (trash) {
            if (!es.isEmpty()) {
                item(mm, acts, L.t("还原", "Восстановить", "Restore"), this::restore);
                item(mm, acts, L.t("永久删除", "Удалить навсегда", "Delete permanently"), () -> delete(true));
                item(mm, acts, L.t("属性", "Свойства", "Properties"), this::props);
            } else {
                item(mm, acts, L.t("清空回收站", "Очистить корзину", "Empty trash"), this::emptyTrash);
                item(mm, acts, L.t("刷新", "Обновить", "Refresh"), this::load);
            }
        } else if (!es.isEmpty()) {
            if (one != null) {
                boolean apk = !one.dir && FU.kind(one.name, false) == FU.APK;
                item(mm, acts, apk ? L.t("安装", "Установить", "Install") : L.t("打开", "Открыть", "Open"), () -> open(one));
                if (!one.dir) item(mm, acts, L.t("打开方式…", "Открыть с помощью…", "Open with…"), () -> view(Files.this, one.f, true));
                if (deep != null) item(mm, acts, L.t("打开所在文件夹", "Открыть папку с файлом", "Open containing folder"), () -> {
                    String p = one.path;
                    go(one.f.getParentFile(), true);
                    pendingSelect = p;
                });
            }
            item(mm, acts, L.t("剪切", "Вырезать", "Cut") + "\tCtrl+X", () -> copy(true));
            item(mm, acts, L.t("复制", "Копировать", "Copy") + "\tCtrl+C", () -> copy(false));
            if (one != null && one.dir && !clip.isEmpty()) item(mm, acts, L.t("粘贴到此文件夹", "Вставить в эту папку", "Paste into folder"), () -> paste(one.f));
            if (one != null) item(mm, acts, L.t("重命名", "Переименовать", "Rename") + "\tF2", this::rename);
            item(mm, acts, L.t("删除", "Удалить", "Delete") + "\tDel", () -> delete(false));
            item(mm, acts, L.t("发送到设备", "Отправить на устройство", "Send to device"), this::send);
            item(mm, acts, L.t("属性", "Свойства", "Properties"), this::props);
        } else {
            if (deep == null) {
                item(mm, acts, L.t("新建文件夹", "Новая папка", "New folder"), this::newFolder);
                if (!clip.isEmpty()) item(mm, acts, L.t("粘贴", "Вставить", "Paste") + "\tCtrl+V", () -> paste(dir));
            }
            item(mm, acts, L.t("全选", "Выделить всё", "Select all") + "\tCtrl+A", this::selectAll);
            item(mm, acts, L.t("刷新", "Обновить", "Refresh") + "\tF5", this::load);
            item(mm, acts, L.t("属性", "Свойства", "Properties"), this::props);
        }
        m.setOnMenuItemClickListener(mi -> { safe(acts.get(mi.getItemId())); return true; });
        m.show();
    }
    void item(Menu m, List<Runnable> acts, String title, Runnable r) {
        int tab = title.indexOf('\t');
        MenuItem mi = m.add(0, acts.size(), acts.size(), tab > 0 ? title.substring(0, tab) : title);
        acts.add(r);
    }
    void moreMenu(View at) {
        final List<Runnable> acts = new ArrayList<>();
        PopupMenu m = new PopupMenu(themed(), at);
        Menu mm = m.getMenu();
        item(mm, acts, hidden ? L.t("不显示隐藏文件", "Не показывать скрытые файлы", "Don't show hidden files") : L.t("显示隐藏文件", "Показывать скрытые файлы", "Show hidden files"), this::toggleHidden);
        String[] names = {L.t("按名称排序", "Сортировать по имени", "Sort by name"), L.t("按大小排序", "Сортировать по размеру", "Sort by size"),
            L.t("按修改时间排序", "Сортировать по дате", "Sort by date"), L.t("按类型排序", "Сортировать по типу", "Sort by type")};
        for (int i = 0; i < 4; i++) {
            final int col = i;
            item(mm, acts, (sort == i ? "✓ " : "") + names[i], () -> sortBy(col));
        }
        if (!inTrash() && deep == null) item(mm, acts, L.t("新建文件夹", "Новая папка", "New folder"), this::newFolder);
        item(mm, acts, L.t("全选", "Выделить всё", "Select all"), this::selectAll);
        item(mm, acts, L.t("转到文件夹…", "Перейти к папке…", "Go to folder…"), this::goToPath);
        item(mm, acts, L.t("刷新", "Обновить", "Refresh"), this::load);
        if (inTrash()) item(mm, acts, L.t("清空回收站", "Очистить корзину", "Empty trash"), this::emptyTrash);
        m.setOnMenuItemClickListener(mi -> { safe(acts.get(mi.getItemId())); return true; });
        m.show();
    }

    // ================= 存储访问权限 =================
    boolean hasAccess() {
        if (Build.VERSION.SDK_INT >= 30) return Environment.isExternalStorageManager();
        return checkSelfPermission(android.Manifest.permission.WRITE_EXTERNAL_STORAGE) == android.content.pm.PackageManager.PERMISSION_GRANTED;
    }
    void checkAccess() {
        boolean ok = true;
        try { ok = hasAccess(); } catch (Throwable ignored) {}
        banner.setVisibility(ok ? View.GONE : View.VISIBLE);
        if (!ok) banner.setText(L.t("还没有「所有文件」访问权限，看不到全部文件。点这里去授权。",
            "Нет доступа ко всем файлам, поэтому видны не все файлы. Нажмите здесь, чтобы разрешить.",
            "\"All files\" access hasn't been granted, so not every file is visible. Click here to allow it."));
    }
    void askAccess() {
        if (Build.VERSION.SDK_INT < 30) {
            requestPermissions(new String[]{android.Manifest.permission.READ_EXTERNAL_STORAGE, android.Manifest.permission.WRITE_EXTERNAL_STORAGE}, 7);
            return;
        }
        try { startActivity(new Intent("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION", Uri.parse("package:" + getPackageName()))); }
        catch (Throwable e) {
            try { startActivity(new Intent("android.settings.MANAGE_ALL_FILES_ACCESS_PERMISSION")); }
            catch (Throwable e2) { FileOps.toast(this, L.t("系统未允许打开", "Система не разрешила открыть", "The system did not allow this")); }
        }
    }
    public void onRequestPermissionsResult(int code, String[] perms, int[] res) {
        super.onRequestPermissionsResult(code, perms, res);
        safe(() -> { checkAccess(); load(); });
    }
}
