package local.cloud.launcher;

import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.graphics.*;
import android.media.MediaScannerConnection;
import android.os.Build;
import android.os.Environment;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.*;
import android.widget.Toast;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.text.SimpleDateFormat;
import java.util.Date;
import java.util.Locale;

/**
 * 截图。Shot.full(context) 截整个屏幕；Shot.region(context) 先把屏幕定格、压暗，拖出一个矩形后只留那一块。
 * 图片存到 /sdcard/Pictures/Screenshots，同时放进剪贴板。
 *
 * 取画面的办法（安卓 14/15）：IWindowManager.captureDisplay()，需要平台签名才有的 READ_FRAME_BUFFER 权限。
 * 这是系统自带截图用的同一条路，不弹「开始录制」的确认框。它不可用时退回执行 /system/bin/screencap
 * （同样靠 READ_FRAME_BUFFER，按调用者的身份检查）。
 */
final class Shot {
    static final String TAG = "Shot";
    static final Handler ui = new Handler(Looper.getMainLooper());
    static volatile boolean busy;
    static View overlay;

    static void full(final Context c) {
        final Context app = c.getApplicationContext() != null ? c.getApplicationContext() : c;
        if (busy) return;
        busy = true;
        new Thread(() -> {
            try {
                Bitmap b = capture();
                finish(app, b);
            } catch (Throwable e) {
                Log.w(TAG, "full: " + e, e);
                toast(app, failed());
            } finally { busy = false; }
        }, "shot").start();
    }

    static void region(final Context c) {
        final Context app = c.getApplicationContext() != null ? c.getApplicationContext() : c;
        if (busy || overlay != null) return;
        busy = true;
        new Thread(() -> {
            try {
                final Bitmap b = capture();
                ui.post(() -> {
                    try { pick(app, b); } catch (Throwable e) { Log.w(TAG, "overlay: " + e, e); overlay = null; toast(app, failed()); }
                });
            } catch (Throwable e) {
                Log.w(TAG, "region: " + e, e);
                toast(app, failed());
            } finally { busy = false; }
        }, "shot").start();
    }
    static String failed() { return L.t("截图失败", "Не удалось сделать снимок экрана", "Screenshot failed"); }

    // ================= 取画面 =================
    static Bitmap capture() throws Exception {
        Throwable first;
        try {
            Bitmap b = viaWindowManager();
            if (b != null) return b;
            first = new IllegalStateException("null buffer");
        } catch (Throwable e) { first = e.getCause() != null ? e.getCause() : e; }
        Log.w(TAG, "captureDisplay unavailable: " + first);
        Bitmap b = viaScreencap();
        if (b == null) throw new Exception("no capture method worked; first error: " + first);
        return b;
    }
    /** WindowManagerGlobal.getWindowManagerService().captureDisplay(0, null, listener)，全是隐藏接口，用反射。 */
    static Bitmap viaWindowManager() throws Exception {
        Object wms = Class.forName("android.view.WindowManagerGlobal").getMethod("getWindowManagerService").invoke(null);
        Class<?> sc = Class.forName("android.window.ScreenCapture");
        Object listener = sc.getMethod("createSyncCaptureListener").invoke(null);
        Class.forName("android.view.IWindowManager")
            .getMethod("captureDisplay", int.class, Class.forName("android.window.ScreenCapture$CaptureArgs"), Class.forName("android.window.ScreenCapture$ScreenCaptureListener"))
            .invoke(wms, Display.DEFAULT_DISPLAY, null, listener);
        Object buf = listener.getClass().getMethod("getBuffer").invoke(listener); // 等画面回来（系统自带超时）
        if (buf == null) return null;
        Bitmap hw = (Bitmap) buf.getClass().getMethod("asBitmap").invoke(buf);
        if (hw == null) return null;
        Bitmap soft = hw.copy(Bitmap.Config.ARGB_8888, false); // 硬件位图不能直接裁剪、压缩
        try {
            hw.recycle();
            Object hb = buf.getClass().getMethod("getHardwareBuffer").invoke(buf);
            if (hb instanceof AutoCloseable) ((AutoCloseable) hb).close();
        } catch (Throwable ignored) {}
        return soft;
    }
    static Bitmap viaScreencap() {
        Process p = null;
        try {
            final Process proc = p = new ProcessBuilder("/system/bin/screencap", "-p").redirectErrorStream(false).start();
            ui.postDelayed(() -> { try { proc.destroy(); } catch (Throwable ignored) {} }, 6000);
            try (InputStream in = proc.getInputStream()) { return BitmapFactory.decodeStream(in); }
        } catch (Throwable e) {
            Log.w(TAG, "screencap: " + e);
            return null;
        } finally {
            try { if (p != null) p.destroy(); } catch (Throwable ignored) {}
        }
    }

    // ================= 保存 + 剪贴板 =================
    static File privateDir(Context c) { return new File(c.getFilesDir(), "shots"); }

    static void finish(final Context app, Bitmap b) throws Exception {
        final File f = save(app, b);
        ui.post(() -> {
            try {
                ClipboardManager cm = (ClipboardManager) app.getSystemService(Context.CLIPBOARD_SERVICE);
                cm.setPrimaryClip(ClipData.newUri(app.getContentResolver(), "Screenshot", FilesProvider.uri(f)));
            } catch (Throwable e) { Log.w(TAG, "clip: " + e); }
            boolean shared = FU.under(f, Environment.getExternalStorageDirectory());
            toast(app, shared ? L.t("截图已保存到「图片/Screenshots」，并已复制", "Снимок сохранён в «Pictures/Screenshots» и скопирован", "Screenshot saved to Pictures/Screenshots and copied")
                : L.t("截图已复制（没有存储权限，未能存进图片文件夹）", "Снимок скопирован (нет доступа к хранилищу, в «Изображения» не сохранён)", "Screenshot copied (no storage access, so it wasn't saved to Pictures)"));
        });
    }
    static File save(Context app, Bitmap b) throws Exception {
        String name = "Screenshot_" + new SimpleDateFormat("yyyyMMdd-HHmmss", Locale.US).format(new Date()) + ".png";
        File pub = new File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_PICTURES), "Screenshots");
        Throwable err = null;
        for (File d : new File[]{pub, privateDir(app)}) {
            try {
                if (!d.isDirectory() && !d.mkdirs()) throw new Exception("mkdirs " + d);
                File f = FU.unique(d, name);
                try (FileOutputStream o = new FileOutputStream(f)) {
                    if (!b.compress(Bitmap.CompressFormat.PNG, 100, o)) throw new Exception("compress");
                }
                if (d == pub) try { MediaScannerConnection.scanFile(app, new String[]{f.getPath()}, new String[]{"image/png"}, null); } catch (Throwable ignored) {}
                else {
                    // 私有目录只是给剪贴板用的临时落脚点：只留最近几张
                    File[] old = d.listFiles();
                    if (old != null && old.length > 5) {
                        java.util.Arrays.sort(old, (x, y) -> Long.compare(x.lastModified(), y.lastModified()));
                        for (int i = 0; i < old.length - 5; i++) old[i].delete();
                    }
                }
                return f;
            } catch (Throwable e) { err = e; Log.w(TAG, "save to " + d + ": " + e); }
        }
        throw new Exception("save failed: " + err);
    }
    static void toast(final Context c, final String s) {
        ui.post(() -> { try { Toast.makeText(c, s, Toast.LENGTH_SHORT).show(); } catch (Throwable ignored) {} });
    }

    // ================= 选区 =================
    static void pick(final Context app, final Bitmap shot) {
        final WindowManager wm = (WindowManager) app.getSystemService(Context.WINDOW_SERVICE);
        final Picker v = new Picker(app, shot);
        // 2036 = TYPE_SCREENSHOT：盖得住底栏；拿不到就退回普通悬浮窗
        int[] types = {2036, Build.VERSION.SDK_INT >= 26 ? WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY : WindowManager.LayoutParams.TYPE_SYSTEM_ALERT};
        RuntimeException last = null;
        for (int type : types) {
            try {
                WindowManager.LayoutParams lp = new WindowManager.LayoutParams(-1, -1, type,
                    WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN | WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS
                        | WindowManager.LayoutParams.FLAG_FULLSCREEN, PixelFormat.OPAQUE);
                lp.setTitle("OpenDesktopScreenshot");
                lp.softInputMode = WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_HIDDEN;
                if (Build.VERSION.SDK_INT >= 28) lp.layoutInDisplayCutoutMode = 1; // SHORT_EDGES
                if (Build.VERSION.SDK_INT >= 30) lp.setFitInsetsTypes(0);
                wm.addView(v, lp);
                overlay = v;
                v.requestFocus();
                return;
            } catch (RuntimeException e) { last = e; Log.w(TAG, "window type " + type + ": " + e); }
        }
        throw last != null ? last : new IllegalStateException("no window");
    }
    static void close() {
        View v = overlay;
        overlay = null;
        if (v == null) return;
        try { ((WindowManager) v.getContext().getSystemService(Context.WINDOW_SERVICE)).removeViewImmediate(v); } catch (Throwable e) { Log.w(TAG, "close: " + e); }
    }

    static final class Picker extends View {
        final Bitmap shot;
        final Context app;
        final Paint p = new Paint(Paint.ANTI_ALIAS_FLAG | Paint.FILTER_BITMAP_FLAG);
        final RectF all = new RectF();
        float x0, y0, x1, y1;
        boolean dragging, has;

        Picker(Context c, Bitmap shot) {
            super(c);
            this.app = c; this.shot = shot;
            setFocusable(true);
            setFocusableInTouchMode(true);
            try { if (Build.VERSION.SDK_INT >= 24) setPointerIcon(PointerIcon.getSystemIcon(c, PointerIcon.TYPE_CROSSHAIR)); } catch (Throwable ignored) {}
        }
        RectF rect() { return new RectF(Math.min(x0, x1), Math.min(y0, y1), Math.max(x0, x1), Math.max(y0, y1)); }

        protected void onDraw(Canvas c) {
            all.set(0, 0, getWidth(), getHeight());
            c.drawBitmap(shot, null, all, p);
            p.setStyle(Paint.Style.FILL);
            p.setColor(0x80000000);
            if (!has) c.drawRect(all, p);
            else {
                RectF r = rect();
                c.drawRect(0, 0, all.right, r.top, p);
                c.drawRect(0, r.bottom, all.right, all.bottom, p);
                c.drawRect(0, r.top, r.left, r.bottom, p);
                c.drawRect(r.right, r.top, all.right, r.bottom, p);
                p.setStyle(Paint.Style.STROKE);
                p.setStrokeWidth(Math.max(2f, T.px(1.5f)));
                p.setColor(0xFF8AB4F8);
                c.drawRect(r, p);
                p.setStyle(Paint.Style.FILL);
                float kx = shot.getWidth() / Math.max(1f, all.width()), ky = shot.getHeight() / Math.max(1f, all.height());
                label(c, Math.round(r.width() * kx) + " × " + Math.round(r.height() * ky), r.left, r.top - T.px(8), false);
            }
            if (!dragging) label(c, L.t("拖动鼠标选择区域 · Enter 截全屏 · Esc 取消", "Выделите область мышью · Enter — весь экран · Esc — отмена", "Drag to select an area · Enter for full screen · Esc to cancel"),
                all.centerX(), T.px(40), true);
        }
        void label(Canvas c, String s, float x, float y, boolean center) {
            p.setTextSize(T.px(center ? 14 : 12));
            p.setTextAlign(center ? Paint.Align.CENTER : Paint.Align.LEFT);
            float w = p.measureText(s), pad = T.px(8), h = p.getTextSize();
            if (y - h - pad < 0) y = h + pad * 2;
            float l = center ? x - w / 2 : x;
            p.setColor(0xCC202124);
            c.drawRoundRect(new RectF(l - pad, y - h - pad / 2, l + w + pad, y + pad), pad, pad, p);
            p.setColor(0xFFFFFFFF);
            c.drawText(s, x, y, p);
        }
        public boolean onTouchEvent(MotionEvent e) {
            try {
                switch (e.getActionMasked()) {
                    case MotionEvent.ACTION_DOWN:
                        if ((e.getButtonState() & MotionEvent.BUTTON_SECONDARY) != 0) { close(); return true; }
                        x0 = x1 = e.getX(); y0 = y1 = e.getY();
                        dragging = true; has = true;
                        break;
                    case MotionEvent.ACTION_MOVE:
                        if (dragging) { x1 = clamp(e.getX(), getWidth()); y1 = clamp(e.getY(), getHeight()); }
                        break;
                    case MotionEvent.ACTION_UP:
                        if (!dragging) break;
                        dragging = false;
                        x1 = clamp(e.getX(), getWidth()); y1 = clamp(e.getY(), getHeight());
                        RectF r = rect();
                        if (r.width() < T.px(6) || r.height() < T.px(6)) { has = false; break; } // 只是点了一下：重新选
                        done(r);
                        return true;
                    case MotionEvent.ACTION_CANCEL: dragging = false; has = false; break;
                }
                invalidate();
            } catch (Throwable t) { Log.w(TAG, "touch: " + t); close(); }
            return true;
        }
        static float clamp(float v, int max) { return Math.max(0, Math.min(max, v)); }

        public boolean dispatchKeyEvent(KeyEvent e) {
            try {
                if (e.getAction() == KeyEvent.ACTION_DOWN && e.getRepeatCount() == 0) {
                    int c = e.getKeyCode();
                    if (c == KeyEvent.KEYCODE_ESCAPE || c == KeyEvent.KEYCODE_BACK) close();
                    else if (c == KeyEvent.KEYCODE_ENTER || c == KeyEvent.KEYCODE_NUMPAD_ENTER) done(null);
                }
            } catch (Throwable t) { Log.w(TAG, "key: " + t); close(); }
            return true;
        }
        void done(RectF r) {
            Bitmap out = shot;
            if (r != null) {
                float kx = shot.getWidth() / (float) Math.max(1, getWidth()), ky = shot.getHeight() / (float) Math.max(1, getHeight());
                int x = Math.max(0, Math.round(r.left * kx)), y = Math.max(0, Math.round(r.top * ky));
                int w = Math.min(shot.getWidth() - x, Math.round(r.width() * kx)), h = Math.min(shot.getHeight() - y, Math.round(r.height() * ky));
                if (w > 0 && h > 0) out = Bitmap.createBitmap(shot, x, y, w, h);
            }
            close();
            final Bitmap fin = out;
            new Thread(() -> {
                try { finish(app, fin); } catch (Throwable t) { Log.w(TAG, "save: " + t, t); toast(app, failed()); }
            }, "shot-save").start();
        }
    }
}
