package local.cloud.launcher;

import android.content.*;
import android.content.pm.*;
import android.graphics.*;
import android.graphics.drawable.*;
import android.os.*;
import org.json.*;
import java.io.*;
import java.util.*;
import java.util.concurrent.*;

/** 应用清单、图标缓存、固定 / 隐藏 / 最近使用。所有磁盘和包管理器访问都在后台线程。 */
final class Apps {
    static final class App {
        String pkg, cls, label, low, full, ini;
        long ver;
        boolean system;
        Bitmap icon;
        String id() { return pkg + "/" + cls; }
    }
    interface Listener { void onApps(); }

    final Context c, store;
    final SharedPreferences sp;
    final Handler main = new Handler(Looper.getMainLooper());
    final ExecutorService io = Executors.newSingleThreadExecutor();
    volatile List<App> all = Collections.emptyList();
    Listener listener;
    int iconPx = 96;
    private Object translit;
    private boolean translitTried;

    Apps(Context c) {
        this.c = c.getApplicationContext();
        store = Store.ctx(this.c);
        sp = store.getSharedPreferences("desk", 0);
    }

    // ---------- 加载 ----------
    void load(boolean useCache) {
        io.execute(() -> {
            try {
                Map<String, App> known = new HashMap<>();
                if (useCache && all.isEmpty()) {
                    List<App> cached = readCache();
                    for (App a : cached) known.put(a.id(), a);
                    if (!cached.isEmpty()) publish(cached);
                } else for (App a : all) known.put(a.id(), a);
                if (AgentService.BASE.contains("127.0.0.1")) { // 仅测试构建：打印拼音键，便于在模拟器上核对
                    for (String t : new String[]{"设置", "文件管理", "F-Droid", "Kiwi Browser", "应用商店"}) {
                        App x = new App(); x.label = t; keys(x);
                        android.util.Log.i("DeskTest", t + " -> full=" + x.full + " ini=" + x.ini);
                    }
                }
                // 用户存储解锁前系统只报得出极少数应用：这时只用缓存，解锁后再扫
                if (!Store.unlocked(c)) return;
                List<App> fresh = scan(known);
                if (!same(fresh, all)) { publish(fresh); writeCache(fresh); }
            } catch (Throwable ignored) {}
        });
    }
    private void publish(List<App> list) {
        main.post(() -> { all = list; if (listener != null) listener.onApps(); });
    }
    private static boolean same(List<App> a, List<App> b) {
        if (a.size() != b.size()) return false;
        for (int i = 0; i < a.size(); i++) {
            App x = a.get(i), y = b.get(i);
            if (!x.id().equals(y.id()) || x.ver != y.ver || !x.label.equals(y.label) || (x.icon == null) != (y.icon == null)) return false;
        }
        return true;
    }
    private List<App> scan(Map<String, App> known) {
        PackageManager pm = c.getPackageManager();
        Intent q = new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER);
        List<App> out = new ArrayList<>();
        Set<String> files = new HashSet<>();
        for (ResolveInfo r : pm.queryIntentActivities(q, 0)) {
            try {
                ActivityInfo ai = r.activityInfo;
                // 桌面自己不进列表，但它自带的「文件」要进
                if (ai == null || c.getPackageName().equals(ai.packageName) && !Files.class.getName().equals(ai.name)) continue;
                App a = new App();
                a.pkg = ai.packageName; a.cls = ai.name;
                a.label = String.valueOf(r.loadLabel(pm)).trim();
                a.system = (ai.applicationInfo.flags & ApplicationInfo.FLAG_SYSTEM) != 0;
                try { a.ver = pm.getPackageInfo(a.pkg, 0).lastUpdateTime; } catch (Exception e) { a.ver = 0; }
                App old = known.get(a.id());
                if (old != null && old.label.equals(a.label)) { a.low = old.low; a.full = old.full; a.ini = old.ini; }
                else keys(a);
                File f = iconFile(a);
                files.add(f.getName());
                if (old != null && old.ver == a.ver && old.icon != null) a.icon = old.icon;
                else {
                    a.icon = f.exists() ? BitmapFactory.decodeFile(f.getPath()) : null;
                    if (a.icon == null) {
                        a.icon = render(r.loadIcon(pm));
                        try (FileOutputStream o = new FileOutputStream(f)) { a.icon.compress(Bitmap.CompressFormat.PNG, 100, o); } catch (Exception ignored) {}
                    }
                }
                out.add(a);
            } catch (Throwable ignored) {}
        }
        Collections.sort(out, (x, y) -> { int k = x.full.compareTo(y.full); return k != 0 ? k : x.label.compareTo(y.label); });
        File[] stale = iconDir().listFiles();
        if (stale != null) for (File f : stale) if (!files.contains(f.getName())) f.delete();
        return out;
    }

    // ---------- 拼音键 ----------
    private void keys(App a) {
        a.low = a.label.toLowerCase(Locale.ROOT);
        String t = a.low;
        if (Build.VERSION.SDK_INT >= 29) {
            try {
                if (!translitTried) { translitTried = true; translit = android.icu.text.Transliterator.getInstance("Any-Latin; Latin-ASCII"); }
                if (translit != null) t = ((android.icu.text.Transliterator) translit).transliterate(a.label).toLowerCase(Locale.ROOT);
            } catch (Throwable e) { translit = null; }
        }
        StringBuilder full = new StringBuilder(), ini = new StringBuilder();
        boolean start = true;
        for (int i = 0; i < t.length(); i++) {
            char ch = t.charAt(i);
            if (Character.isLetterOrDigit(ch)) { full.append(ch); if (start) ini.append(ch); start = false; }
            else start = true;
        }
        a.full = full.length() > 0 ? full.toString() : a.low;
        a.ini = ini.toString();
    }

    // ---------- 图标：统一裁成圆形 ----------
    private Bitmap render(Drawable d) {
        int s = iconPx;
        Bitmap tmp = Bitmap.createBitmap(s, s, Bitmap.Config.ARGB_8888);
        Canvas cv = new Canvas(tmp);
        if (Build.VERSION.SDK_INT >= 26 && d instanceof AdaptiveIconDrawable) {
            AdaptiveIconDrawable ad = (AdaptiveIconDrawable) d;
            int e = s / 4;
            cv.drawColor(T.plate);
            for (Drawable layer : new Drawable[]{ad.getBackground(), ad.getForeground()})
                if (layer != null) { layer.setBounds(-e, -e, s + e, s + e); layer.draw(cv); }
        } else {
            cv.drawColor(T.plate);
            int in = Math.round(s * 0.15f);
            if (d != null) { d.setBounds(in, in, s - in, s - in); d.draw(cv); }
        }
        Bitmap out = Bitmap.createBitmap(s, s, Bitmap.Config.ARGB_8888);
        Paint p = new Paint(Paint.ANTI_ALIAS_FLAG);
        p.setShader(new BitmapShader(tmp, Shader.TileMode.CLAMP, Shader.TileMode.CLAMP));
        new Canvas(out).drawCircle(s / 2f, s / 2f, s / 2f, p);
        tmp.recycle();
        return out;
    }
    private File iconDir() { File d = new File(store.getFilesDir(), "icons"); d.mkdirs(); return d; }
    private File iconFile(App a) { return new File(iconDir(), Integer.toHexString(a.id().hashCode()) + "_" + a.ver + "_" + iconPx + ".png"); }

    // ---------- 清单缓存 ----------
    private File cacheFile() { return new File(store.getFilesDir(), "apps.json"); }
    private List<App> readCache() {
        List<App> out = new ArrayList<>();
        File f = cacheFile();
        if (!f.exists()) return out;
        try {
            byte[] buf = new byte[(int) f.length()];
            try (DataInputStream in = new DataInputStream(new FileInputStream(f))) { in.readFully(buf); }
            JSONArray arr = new JSONArray(new String(buf, "UTF-8"));
            for (int i = 0; i < arr.length(); i++) {
                JSONObject o = arr.getJSONObject(i);
                App a = new App();
                a.pkg = o.getString("p"); a.cls = o.getString("c"); a.label = o.getString("l");
                a.low = o.getString("w"); a.full = o.getString("f"); a.ini = o.getString("i");
                a.ver = o.getLong("v"); a.system = o.getBoolean("s");
                a.icon = BitmapFactory.decodeFile(iconFile(a).getPath());
                out.add(a);
            }
        } catch (Throwable e) { out.clear(); }
        return out;
    }
    private void writeCache(List<App> list) {
        try {
            JSONArray arr = new JSONArray();
            for (App a : list) arr.put(new JSONObject().put("p", a.pkg).put("c", a.cls).put("l", a.label)
                .put("w", a.low).put("f", a.full).put("i", a.ini).put("v", a.ver).put("s", a.system));
            File tmp = new File(store.getFilesDir(), "apps.json.tmp");
            try (FileOutputStream o = new FileOutputStream(tmp)) { o.write(arr.toString().getBytes("UTF-8")); }
            tmp.renameTo(cacheFile());
        } catch (Throwable ignored) {}
    }

    // ---------- 查询 ----------
    Set<String> hidden() { return new HashSet<>(sp.getStringSet("hidden", Collections.<String>emptySet())); }
    List<App> visible() {
        Set<String> h = hidden();
        List<App> out = new ArrayList<>();
        for (App a : all) if (!h.contains(a.id())) out.add(a);
        return out;
    }
    App find(String id) { for (App a : all) if (a.id().equals(id)) return a; return null; }

    List<App> search(String query) {
        final String q = query.toLowerCase(Locale.ROOT).replace(" ", "");
        List<App> out = new ArrayList<>();
        final Map<App, Integer> rank = new HashMap<>();
        for (App a : visible()) {
            int r;
            if (a.low.startsWith(q)) r = 0;
            else if (a.ini.startsWith(q)) r = 1;
            else if (a.full.startsWith(q)) r = 2;
            else if (a.low.contains(q)) r = 3;
            else if (a.ini.contains(q)) r = 4;
            else if (a.full.contains(q)) r = 5;
            else continue;
            rank.put(a, r); out.add(a);
        }
        Collections.sort(out, (x, y) -> rank.get(x) - rank.get(y));
        return out;
    }

    // ---------- 固定 ----------
    List<String> pinIds() {
        String s = sp.getString("pins", null);
        List<String> out = new ArrayList<>();
        if (s != null) { for (String id : s.split("\n")) if (!id.isEmpty()) out.add(id); return out; }
        if (all.isEmpty()) return out;
        // 首次：挑设备上已装的常用应用
        String browser = null;
        try {
            ResolveInfo r = c.getPackageManager().resolveActivity(new Intent(Intent.ACTION_VIEW, android.net.Uri.parse("http://example.com")), PackageManager.MATCH_DEFAULT_ONLY);
            if (r != null && r.activityInfo != null && !"android".equals(r.activityInfo.packageName)) browser = r.activityInfo.packageName;
        } catch (Throwable ignored) {}
        String[] want = {browser, "com.kiwibrowser.browser", "com.android.documentsui", "org.fdroid.fdroid", "com.android.settings"};
        Set<String> seen = new HashSet<>();
        for (String p : want) {
            if (p == null || !seen.add(p)) continue;
            for (App a : all) if (a.pkg.equals(p)) { out.add(a.id()); break; }
        }
        savePins(out);
        return out;
    }
    void savePins(List<String> ids) { sp.edit().putString("pins", android.text.TextUtils.join("\n", ids)).apply(); }
    boolean pinned(App a) { return pinIds().contains(a.id()); }
    void togglePin(App a) {
        List<String> p = pinIds();
        if (!p.remove(a.id())) p.add(a.id());
        savePins(p);
    }
    void hide(App a) { Set<String> h = hidden(); h.add(a.id()); sp.edit().putStringSet("hidden", h).apply(); }
    void unhideAll() { sp.edit().remove("hidden").apply(); }

    // ---------- 最近使用（只记录从本桌面启动的） ----------
    private JSONObject used() { try { return new JSONObject(sp.getString("used", "{}")); } catch (JSONException e) { return new JSONObject(); } }
    void markUsed(App a) {
        try { sp.edit().putString("used", used().put(a.id(), System.currentTimeMillis()).toString()).apply(); } catch (JSONException ignored) {}
    }
    List<App> recents(int n) {
        final JSONObject u = used();
        List<App> out = new ArrayList<>();
        for (App a : visible()) if (u.optLong(a.id(), 0) > 0) out.add(a);
        Collections.sort(out, (x, y) -> Long.compare(u.optLong(y.id(), 0), u.optLong(x.id(), 0)));
        return out.size() > n ? new ArrayList<>(out.subList(0, n)) : out;
    }
}
