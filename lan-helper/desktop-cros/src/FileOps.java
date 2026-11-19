package local.cloud.launcher;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.Context;
import android.content.SharedPreferences;
import android.os.Environment;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.util.Log;
import android.util.TypedValue;
import android.view.ContextThemeWrapper;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.TextView;
import org.json.JSONObject;
import java.io.*;
import java.util.*;

/** 复制 / 移动 / 删除 / 回收站。耗时操作都在后台线程，超过一瞬间就弹出带取消的进度窗。 */
final class FileOps {
    static final String TAG = "FileOps";
    static final int REPLACE = 0, KEEP_BOTH = 1, SKIP = 2;
    interface Done { void done(List<File> made, int errors); }

    static Context themed(Context c) {
        return new ContextThemeWrapper(c, T.dark ? android.R.style.Theme_DeviceDefault : android.R.style.Theme_DeviceDefault_Light);
    }

    // ================= 后台任务 + 进度窗 =================
    abstract static class Job implements Runnable {
        final Activity a;
        final Handler ui = new Handler(Looper.getMainLooper());
        final String title;
        final Done done;
        final List<File> made = Collections.synchronizedList(new ArrayList<File>());
        volatile boolean cancel;
        volatile long total, pos;
        volatile String cur = "";
        volatile int errors;
        volatile String firstError;
        AlertDialog dlg;
        ProgressBar bar;
        TextView msg;
        long lastTick;
        boolean finished;

        Job(Activity a, String title, Done done) { this.a = a; this.title = title; this.done = done; }

        void start() {
            ui.postDelayed(this::show, 350);
            new Thread(this, "file-op").start();
        }
        void show() {
            if (finished || a.isFinishing()) return;
            try {
                LinearLayout box = new LinearLayout(a);
                box.setOrientation(LinearLayout.VERTICAL);
                box.setPadding(T.px(24), T.px(14), T.px(24), T.px(4));
                msg = new TextView(a);
                msg.setTextColor(T.text2);
                msg.setTextSize(TypedValue.COMPLEX_UNIT_PX, T.px(13));
                msg.setSingleLine(true);
                msg.setEllipsize(android.text.TextUtils.TruncateAt.MIDDLE);
                box.addView(msg);
                bar = new ProgressBar(themed(a), null, android.R.attr.progressBarStyleHorizontal);
                bar.setMax(1000);
                LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(-1, -2);
                lp.topMargin = T.px(10);
                box.addView(bar, lp);
                dlg = new AlertDialog.Builder(themed(a)).setTitle(title).setView(box).setCancelable(false)
                    .setNegativeButton(L.t("取消", "Отмена", "Cancel"), (d, w) -> cancel = true).create();
                dlg.show();
                update();
            } catch (Throwable e) { Log.w(TAG, "progress: " + e); }
        }
        void update() {
            if (dlg == null || finished) return;
            long t = total, p = pos;
            bar.setIndeterminate(t <= 0);
            if (t > 0) bar.setProgress((int) Math.min(1000, p * 1000 / t));
            msg.setText(t > 0 ? cur + "  ·  " + FU.size(p) + " / " + FU.size(t) : cur);
        }
        void tick(String name) {
            if (name != null) cur = name;
            long now = SystemClock.uptimeMillis();
            if (now - lastTick < 120) return;
            lastTick = now;
            ui.post(this::update);
        }
        void fail(File f, Throwable e) {
            errors++;
            if (firstError == null) firstError = f.getName() + ": " + (e == null || e.getMessage() == null ? L.t("操作失败", "операция не удалась", "operation failed") : e.getMessage());
            Log.w(TAG, "fail " + f + ": " + e);
        }
        abstract void work() throws Exception;
        public void run() {
            try { work(); } catch (Throwable e) { errors++; if (firstError == null) firstError = String.valueOf(e.getMessage()); Log.w(TAG, "job: " + e); }
            ui.post(() -> {
                finished = true;
                try { if (dlg != null) dlg.dismiss(); } catch (Throwable ignored) {}
                try {
                    if (errors > 0 && !a.isFinishing())
                        new AlertDialog.Builder(themed(a)).setTitle(L.t("有 ", "Не удалось: ", "") + errors + L.t(" 项未完成", "", " item(s) failed"))
                            .setMessage(firstError).setPositiveButton(android.R.string.ok, null).show();
                } catch (Throwable ignored) {}
                try { if (done != null) done.done(new ArrayList<>(made), errors); } catch (Throwable e) { Log.w(TAG, "done: " + e); }
            });
        }

        long measure(File f) {
            if (cancel) return 0;
            if (!f.isDirectory()) return f.length();
            long n = 0;
            File[] k = f.listFiles();
            if (k != null) for (File c : k) n += measure(c);
            return n;
        }
        void copyFile(File s, File d) throws IOException {
            tick(s.getName());
            File tmp = new File(d.getParentFile(), "." + d.getName() + ".part~");
            boolean ok = false;
            try (InputStream in = new FileInputStream(s); OutputStream out = new FileOutputStream(tmp)) {
                byte[] buf = new byte[256 * 1024];
                int n;
                while ((n = in.read(buf)) > 0) {
                    if (cancel) throw new InterruptedIOException("cancelled");
                    out.write(buf, 0, n);
                    pos += n;
                    tick(null);
                }
                ok = true;
            } finally {
                if (!ok) tmp.delete();
            }
            if (d.exists() && !d.delete()) { tmp.delete(); throw new IOException(L.t("无法覆盖", "не удалось заменить", "cannot replace")); }
            if (!tmp.renameTo(d)) { tmp.delete(); throw new IOException(L.t("无法写入", "не удалось записать", "cannot write")); }
            d.setLastModified(s.lastModified());
        }
        /** 返回 false 表示有没复制成的内容（调用方据此决定移动后能不能删源文件）。 */
        boolean copyTree(File s, File d) {
            if (cancel) return false;
            try {
                if (s.isDirectory()) {
                    if (!d.isDirectory() && !d.mkdirs()) throw new IOException(L.t("无法创建文件夹", "не удалось создать папку", "cannot create folder"));
                    boolean all = true;
                    File[] k = s.listFiles();
                    if (k != null) for (File c : k) all &= copyTree(c, new File(d, c.getName()));
                    d.setLastModified(s.lastModified());
                    return all && !cancel;
                }
                copyFile(s, d);
                return true;
            } catch (InterruptedIOException e) {
                return false;
            } catch (Throwable e) {
                fail(s, e);
                return false;
            }
        }
        boolean deleteTree(File f) {
            if (cancel) return false;
            tick(f.getName());
            File[] k = f.isDirectory() ? f.listFiles() : null;
            boolean ok = true;
            if (k != null) for (File c : k) ok &= deleteTree(c);
            return ok && f.delete();
        }
    }

    // ================= 复制 / 移动 =================
    /** 在界面线程调用。先问清同名冲突怎么办，再开工。 */
    static void transfer(final Activity a, final List<File> src, final File dst, final boolean move, final Done done) {
        final List<File> list = new ArrayList<>();
        int conflicts = 0;
        for (File s : src) {
            if (FU.under(dst, s) && s.isDirectory()) {
                toast(a, L.t("不能把文件夹放进它自己里面", "Нельзя поместить папку в саму себя", "A folder can't be placed inside itself"));
                continue;
            }
            boolean sameDir = dst.equals(s.getParentFile());
            if (sameDir && move) continue; // 原地移动：什么都不用做
            if (!sameDir && new File(dst, s.getName()).exists()) conflicts++;
            list.add(s);
        }
        if (list.isEmpty()) { if (done != null) done.done(new ArrayList<File>(), 0); return; }
        if (conflicts == 0) { run(a, list, dst, move, KEEP_BOTH, done); return; }
        String[] opts = {L.t("替换", "Заменить", "Replace"), L.t("保留两者", "Сохранить оба", "Keep both"), L.t("跳过同名的", "Пропустить", "Skip these")};
        new AlertDialog.Builder(themed(a))
            .setTitle(conflicts + L.t(" 个同名项目已存在", " объект(ов) с таким именем уже есть", " item(s) with the same name already exist"))
            .setItems(opts, (d, which) -> run(a, list, dst, move, which, done))
            .setNegativeButton(L.t("取消", "Отмена", "Cancel"), null).show();
    }
    private static void run(final Activity a, final List<File> list, final File dst, final boolean move, final int policy, Done done) {
        new Job(a, move ? L.t("正在移动…", "Перемещение…", "Moving…") : L.t("正在复制…", "Копирование…", "Copying…"), done) {
            void work() {
                long sum = 0;
                for (File s : list) sum += measure(s);
                total = sum;
                for (File s : list) {
                    if (cancel) break;
                    File t = new File(dst, s.getName());
                    boolean exists = t.exists();
                    if (exists && dst.equals(s.getParentFile())) t = FU.unique(dst, s.getName()); // 同目录复制 = 建副本
                    else if (exists && policy == SKIP) { pos += measure(s); continue; }
                    else if (exists && policy == KEEP_BOTH) t = FU.unique(dst, s.getName());
                    long size = measure(s);
                    if (t.exists() && t.isDirectory() != s.isDirectory()) t = FU.unique(dst, s.getName()); // 一个是文件一个是文件夹：不硬替换
                    if (move) {
                        if (t.exists() && !t.isDirectory()) t.delete();
                        if (!t.exists() && s.renameTo(t)) { pos += size; made.add(t); tick(s.getName()); continue; }
                    }
                    boolean ok = copyTree(s, t);
                    if (t.exists()) made.add(t);
                    if (move && ok) { if (!deleteTree(s)) fail(s, null); }
                }
            }
        }.start();
    }

    // ================= 删除与回收站 =================
    static File home() { return Environment.getExternalStorageDirectory(); }
    static File trashDir() { return new File(home(), ".Trash-desk"); }
    static boolean inTrash(File f) { return FU.under(f, trashDir()); }
    static boolean canTrash(File f) { return FU.under(f, home()) && !inTrash(f); }
    static SharedPreferences prefs(Context c) { return Store.ctx(c).getSharedPreferences("files", 0); }
    static JSONObject trashIndex(Context c) {
        try { return new JSONObject(prefs(c).getString("trash", "{}")); } catch (Exception e) { return new JSONObject(); }
    }
    /** 回收站里的文件名是「时间戳_原名」，原来的完整路径记在索引里。 */
    static String trashedName(String stored) {
        int i = stored.indexOf('_');
        return i > 0 && i < stored.length() - 1 && stored.substring(0, i).matches("\\d+") ? stored.substring(i + 1) : stored;
    }

    static void delete(final Activity a, final List<File> src, final boolean toTrash, Done done) {
        final Context app = a.getApplicationContext();
        new Job(a, L.t("正在删除…", "Удаление…", "Deleting…"), done) {
            void work() throws Exception {
                JSONObject idx = trashIndex(app);
                boolean changed = false;
                long stamp = System.currentTimeMillis();
                for (File s : src) {
                    if (cancel) break;
                    if (toTrash && canTrash(s)) {
                        File td = trashDir();
                        if (!td.isDirectory()) { td.mkdirs(); try { new File(td, ".nomedia").createNewFile(); } catch (IOException ignored) {} }
                        File t = new File(td, (stamp++) + "_" + s.getName());
                        tick(s.getName());
                        if (s.renameTo(t)) { idx.put(t.getName(), s.getAbsolutePath()); changed = true; continue; }
                        fail(s, null);
                    } else {
                        boolean wasTrash = inTrash(s);
                        if (!deleteTree(s)) { if (!cancel) fail(s, null); }
                        else if (wasTrash) { idx.remove(s.getName()); changed = true; }
                    }
                }
                if (changed) prefs(app).edit().putString("trash", idx.toString()).commit();
            }
        }.start();
    }
    static void restore(final Activity a, final List<File> src, Done done) {
        final Context app = a.getApplicationContext();
        new Job(a, L.t("正在还原…", "Восстановление…", "Restoring…"), done) {
            void work() throws Exception {
                JSONObject idx = trashIndex(app);
                for (File s : src) {
                    if (cancel) break;
                    String orig = idx.optString(s.getName(), null);
                    File t = orig != null ? new File(orig) : new File(home(), trashedName(s.getName()));
                    File parent = t.getParentFile();
                    if (parent != null && !parent.isDirectory()) parent.mkdirs();
                    if (t.exists()) t = FU.unique(parent, t.getName());
                    tick(t.getName());
                    if (s.renameTo(t)) { idx.remove(s.getName()); made.add(t); } else fail(s, null);
                }
                prefs(app).edit().putString("trash", idx.toString()).commit();
            }
        }.start();
    }

    static void toast(Context c, String s) {
        try { android.widget.Toast.makeText(c, s, android.widget.Toast.LENGTH_SHORT).show(); } catch (Throwable ignored) {}
    }
}
