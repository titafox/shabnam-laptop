package local.cloud.launcher;

import android.content.*;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.*;
import android.util.Log;
import java.io.File;
import java.io.FileNotFoundException;
import java.util.ArrayList;
import java.util.List;

/**
 * 把共享存储里的文件以 content:// 地址交给别的应用（打开、安装、分享、剪贴板里的截图）。
 * 地址形如 content://local.cloud.launcher.files/storage/emulated/0/Download/a.pdf —— 路径就是文件的真实路径。
 * 不对外导出：别的应用只能访问我们逐个授权（FLAG_GRANT_*_URI_PERMISSION）的那一个文件。
 * 这个提供者跑在桌面主进程里，所以也顺带做两件事：进程一起来就开始记录剪贴板；
 * 替「文件」窗口（独立进程）转交「发送到设备」。
 */
public class FilesProvider extends ContentProvider {
    static final String AUTH = "local.cloud.launcher.files", TAG = "FilesProvider";

    public boolean onCreate() {
        try { Clip.init(getContext()); } catch (Throwable e) { Log.w(TAG, "clip: " + e); }
        return true;
    }

    static Uri uri(File f) {
        return new Uri.Builder().scheme(ContentResolver.SCHEME_CONTENT).authority(AUTH).path(f.getAbsolutePath()).build();
    }
    /** 只放行共享存储、外接存储和我们自己的截图目录。 */
    static boolean allowed(Context c, File f) {
        try {
            String p = f.getCanonicalPath();
            if (p.startsWith("/storage/") || p.startsWith("/sdcard/") || p.startsWith("/mnt/media_rw/") || p.startsWith("/data/media/")) return true;
            return FU.under(new File(p), Shot.privateDir(c).getCanonicalFile());
        } catch (Throwable e) { return false; }
    }
    private File file(Uri u) throws FileNotFoundException {
        String p = u.getPath();
        if (p == null || p.isEmpty()) throw new FileNotFoundException();
        File f = new File(p);
        if (!allowed(getContext(), f)) throw new SecurityException("path not served");
        return f;
    }

    public String getType(Uri u) {
        try { return FU.mime(file(u)); } catch (Exception e) { return null; }
    }
    public ParcelFileDescriptor openFile(Uri u, String mode) throws FileNotFoundException {
        File f = file(u);
        if (f.isDirectory()) throw new FileNotFoundException("directory");
        return ParcelFileDescriptor.open(f, ParcelFileDescriptor.parseMode(mode));
    }
    public Cursor query(Uri u, String[] cols, String s, String[] args, String sort) {
        File f;
        try { f = file(u); } catch (FileNotFoundException e) { return null; }
        if (cols == null) cols = new String[]{"_display_name", "_size", "mime_type", "last_modified"};
        MatrixCursor cur = new MatrixCursor(cols, 1);
        MatrixCursor.RowBuilder row = cur.newRow();
        for (String col : cols) {
            if ("_display_name".equals(col)) row.add(f.getName());
            else if ("_size".equals(col)) row.add(f.length());
            else if ("mime_type".equals(col)) row.add(FU.mime(f));
            else if ("last_modified".equals(col) || "date_modified".equals(col)) row.add(f.lastModified());
            else if ("_data".equals(col)) row.add(f.getAbsolutePath());
            else row.add(null);
        }
        return cur;
    }
    public Uri insert(Uri u, ContentValues v) { throw new UnsupportedOperationException(); }
    public int update(Uri u, ContentValues v, String s, String[] a) { throw new UnsupportedOperationException(); }
    public int delete(Uri u, String s, String[] a) { throw new UnsupportedOperationException(); }

    /** 进程间的小通道（只有本应用自己能调）：dark = 当前主题；send = 发送到设备。 */
    public Bundle call(String method, String arg, Bundle extras) {
        Bundle out = new Bundle();
        try {
            final Context c = getContext();
            if ("dark".equals(method)) {
                out.putBoolean("dark", Store.ctx(c).getSharedPreferences("desk", 0).getBoolean("dark", true));
            } else if ("send".equals(method) && extras != null) {
                ArrayList<String> paths = extras.getStringArrayList("paths");
                final List<Uri> uris = new ArrayList<>();
                if (paths != null) for (String p : paths) if (allowed(c, new File(p))) uris.add(uri(new File(p)));
                // Send 由另一处实现；没有它时返回 ok=false，调用方退回系统分享
                final java.lang.reflect.Method m = Class.forName("local.cloud.launcher.Send").getDeclaredMethod("sendFiles", Context.class, List.class);
                m.setAccessible(true);
                new Handler(Looper.getMainLooper()).post(() -> {
                    try { m.invoke(null, c, uris); } catch (Throwable e) { Log.w(TAG, "send: " + e); }
                });
                out.putBoolean("ok", true);
            }
        } catch (Throwable e) {
            Log.w(TAG, "call " + method + ": " + e);
        }
        return out;
    }
}
