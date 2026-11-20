package local.cloud.launcher;

import android.content.Context;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageInfo;
import android.graphics.*;
import android.graphics.drawable.Drawable;
import android.media.ThumbnailUtils;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.util.LruCache;
import android.util.Size;
import android.widget.ImageView;
import java.io.File;
import java.util.Locale;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/** 文件管理器用到的图形：线条图标、文件类型图标、缩略图。全部用代码画，不带图片资源。 */
final class FI {
    static final int BACK = 0, FWD = 1, UP = 2, LIST = 3, GRID = 4, NEWDIR = 5, MORE = 6, SEARCH = 7, HOME = 8, DOWNLOAD = 9,
        PICTURE = 10, DOC = 11, RECEIVED = 12, USB = 13, TRASH = 14, EJECT = 15, GEAR = 16, CLOSE = 17, FOLDER = 18;

    /** 24×24 画布上的线条图标。 */
    static final class Glyph extends Drawable {
        final int type;
        final Paint p = new Paint(Paint.ANTI_ALIAS_FLAG);
        int alpha = 255;
        Glyph(int type, int color) {
            this.type = type;
            p.setColor(color);
            p.setStrokeCap(Paint.Cap.ROUND);
            p.setStrokeJoin(Paint.Join.ROUND);
        }
        private void poly(Canvas c, boolean close, float... v) {
            Path path = new Path();
            path.moveTo(v[0], v[1]);
            for (int i = 2; i + 1 < v.length; i += 2) path.lineTo(v[i], v[i + 1]);
            if (close) path.close();
            c.drawPath(path, p);
        }
        public void draw(Canvas c) {
            Rect b = getBounds();
            float s = Math.min(b.width(), b.height()) / 24f;
            c.save();
            c.translate(b.left + (b.width() - 24 * s) / 2f, b.top + (b.height() - 24 * s) / 2f);
            c.scale(s, s);
            p.setAlpha(alpha);
            p.setStyle(Paint.Style.STROKE);
            p.setStrokeWidth(1.8f);
            switch (type) {
                case BACK: poly(c, false, 19, 12, 6, 12); poly(c, false, 11, 7, 6, 12, 11, 17); break;
                case FWD: poly(c, false, 5, 12, 18, 12); poly(c, false, 13, 7, 18, 12, 13, 17); break;
                case UP: poly(c, false, 12, 19, 12, 6); poly(c, false, 7, 11, 12, 6, 17, 11); break;
                case LIST: for (int y = 7; y <= 17; y += 5) c.drawLine(5, y, 19, y, p); break;
                case GRID:
                    for (int i = 0; i < 4; i++) {
                        float x = i % 2 == 0 ? 5 : 13.5f, y = i < 2 ? 5 : 13.5f;
                        c.drawRoundRect(new RectF(x, y, x + 5.5f, y + 5.5f), 1.2f, 1.2f, p);
                    }
                    break;
                case NEWDIR:
                    poly(c, true, 3, 6, 9.5f, 6, 11.5f, 8.5f, 21, 8.5f, 21, 19, 3, 19);
                    c.drawLine(12, 11.2f, 12, 16.4f, p); c.drawLine(9.4f, 13.8f, 14.6f, 13.8f, p);
                    break;
                case FOLDER: poly(c, true, 3, 6, 9.5f, 6, 11.5f, 8.5f, 21, 8.5f, 21, 19, 3, 19); break;
                case MORE:
                    p.setStyle(Paint.Style.FILL);
                    for (int y = 6; y <= 18; y += 6) c.drawCircle(12, y, 1.6f, p);
                    break;
                case SEARCH: c.drawCircle(10.5f, 10.5f, 5.5f, p); c.drawLine(15, 15, 20, 20, p); break;
                case HOME: poly(c, false, 4, 11.5f, 12, 4.5f, 20, 11.5f); poly(c, false, 6.5f, 10, 6.5f, 19.5f, 17.5f, 19.5f, 17.5f, 10); break;
                case DOWNLOAD: c.drawLine(12, 4, 12, 15, p); poly(c, false, 7, 10, 12, 15, 17, 10); c.drawLine(5, 19.5f, 19, 19.5f, p); break;
                case PICTURE:
                    c.drawRoundRect(new RectF(3.5f, 5, 20.5f, 19), 2, 2, p);
                    poly(c, false, 4, 16.5f, 9, 11.5f, 12.5f, 15, 15.5f, 12.5f, 20, 16.5f);
                    c.drawCircle(15.5f, 9, 1.3f, p);
                    break;
                case DOC:
                    poly(c, true, 6, 3.5f, 13.5f, 3.5f, 18, 8, 18, 20.5f, 6, 20.5f);
                    c.drawLine(9, 12, 15, 12, p); c.drawLine(9, 16, 15, 16, p);
                    break;
                case RECEIVED:
                    c.drawLine(5, 9, 19, 9, p); poly(c, false, 15.5f, 5.5f, 19, 9, 15.5f, 12.5f);
                    c.drawLine(19, 16, 5, 16, p); poly(c, false, 8.5f, 12.5f, 5, 16, 8.5f, 19.5f);
                    break;
                case USB:
                    c.drawRoundRect(new RectF(7, 9, 17, 21), 2, 2, p);
                    poly(c, false, 9, 9, 9, 3.5f, 15, 3.5f, 15, 9);
                    break;
                case TRASH:
                    c.drawLine(5, 7, 19, 7, p);
                    poly(c, false, 7, 7, 8, 20, 16, 20, 17, 7);
                    poly(c, false, 10, 7, 10, 4, 14, 4, 14, 7);
                    break;
                case EJECT: poly(c, true, 6, 14, 12, 6, 18, 14); c.drawLine(6, 18.5f, 18, 18.5f, p); break;
                case GEAR:
                    c.drawCircle(12, 12, 3, p);
                    c.drawCircle(12, 12, 6.3f, p);
                    for (int i = 0; i < 8; i++) {
                        double a = Math.PI * i / 4;
                        float cx = (float) Math.cos(a), cy = (float) Math.sin(a);
                        c.drawLine(12 + cx * 6.3f, 12 + cy * 6.3f, 12 + cx * 9f, 12 + cy * 9f, p);
                    }
                    break;
                default: c.drawLine(6, 6, 18, 18, p); c.drawLine(18, 6, 6, 18, p);
            }
            c.restore();
        }
        public void setAlpha(int a) { alpha = a; invalidateSelf(); }
        public void setColorFilter(ColorFilter f) {}
        public int getOpacity() { return PixelFormat.TRANSLUCENT; }
    }

    /** 文件类型图标：文件夹是一只夹子，其余是一页纸加一条带扩展名的色带。 */
    static final class FileIcon extends Drawable {
        static final int[] COLORS = {0xFF7BAAF7, 0xFF34A853, 0xFFEA4335, 0xFFA142F4, 0xFFB08D57, 0xFF3DDC84, 0xFFE94235, 0xFF4285F4, 0xFF80868B};
        final int kind;
        final String ext;
        final Paint p = new Paint(Paint.ANTI_ALIAS_FLAG);
        FileIcon(int kind, String ext) {
            this.kind = kind;
            String e = ext == null ? "" : ext.toUpperCase(Locale.ROOT);
            this.ext = e.length() > 4 ? e.substring(0, 4) : e;
        }
        public void draw(Canvas c) {
            Rect b = getBounds();
            float s = Math.min(b.width(), b.height()), x = b.left + (b.width() - s) / 2f, y = b.top + (b.height() - s) / 2f;
            p.setStyle(Paint.Style.FILL);
            if (kind == FU.DIR) {
                p.setColor(0xFF5E97F6);
                Path tab = new Path();
                tab.moveTo(x + s * 0.08f, y + s * 0.24f);
                tab.lineTo(x + s * 0.40f, y + s * 0.24f);
                tab.lineTo(x + s * 0.50f, y + s * 0.36f);
                tab.lineTo(x + s * 0.08f, y + s * 0.36f);
                tab.close();
                c.drawPath(tab, p);
                c.drawRoundRect(new RectF(x + s * 0.08f, y + s * 0.20f, x + s * 0.42f, y + s * 0.40f), s * 0.05f, s * 0.05f, p);
                p.setColor(COLORS[0]);
                c.drawRoundRect(new RectF(x + s * 0.08f, y + s * 0.32f, x + s * 0.92f, y + s * 0.82f), s * 0.07f, s * 0.07f, p);
                return;
            }
            float l = x + s * 0.20f, r = x + s * 0.80f, t = y + s * 0.08f, bt = y + s * 0.92f, fold = s * 0.20f;
            Path page = new Path();
            page.moveTo(l, t); page.lineTo(r - fold, t); page.lineTo(r, t + fold); page.lineTo(r, bt); page.lineTo(l, bt); page.close();
            p.setColor(T.dark ? 0xFFDADCE0 : 0xFFE8EAED);
            c.drawPath(page, p);
            Path corner = new Path();
            corner.moveTo(r - fold, t); corner.lineTo(r, t + fold); corner.lineTo(r - fold, t + fold); corner.close();
            p.setColor(0xFFBDC1C6);
            c.drawPath(corner, p);
            int col = COLORS[Math.max(1, Math.min(COLORS.length - 1, kind))];
            p.setColor(col);
            float bandTop = y + s * 0.56f, bandBottom = y + s * 0.82f;
            c.drawRect(l, bandTop, r, bandBottom, p);
            if (!ext.isEmpty() && s >= 20) {
                p.setColor(0xFFFFFFFF);
                p.setTypeface(Typeface.DEFAULT_BOLD);
                p.setTextAlign(Paint.Align.CENTER);
                float ts = (bandBottom - bandTop) * 0.74f;
                p.setTextSize(ts);
                float w = p.measureText(ext), max = (r - l) * 0.92f;
                if (w > max) p.setTextSize(ts * max / w);
                Paint.FontMetrics fm = p.getFontMetrics();
                c.drawText(ext, (l + r) / 2f, (bandTop + bandBottom) / 2f - (fm.ascent + fm.descent) / 2f, p);
            }
        }
        public void setAlpha(int a) {}
        public void setColorFilter(ColorFilter f) {}
        public int getOpacity() { return PixelFormat.TRANSLUCENT; }
    }

    static Drawable icon(String name, boolean dir) { return new FileIcon(FU.kind(name, dir), FU.ext(name)); }

    // ================= 缩略图 =================
    static final LruCache<String, Bitmap> cache = new LruCache<String, Bitmap>(20 * 1024) {
        protected int sizeOf(String k, Bitmap b) { return Math.max(1, b.getByteCount() / 1024); }
    };
    static final ExecutorService pool = Executors.newFixedThreadPool(2);
    static final Handler ui = new Handler(Looper.getMainLooper());

    static boolean thumbable(int kind) { return kind == FU.IMAGE || kind == FU.VIDEO || kind == FU.APK; }

    /** 先放类型图标，可出缩略图的在后台解码后换上。iv 的 tag 用来防止滚动复用时串图。 */
    static void bind(final ImageView iv, final File f, final String name, final boolean dir, final long stamp, final int px) {
        final int kind = FU.kind(name, dir);
        iv.setImageDrawable(new FileIcon(kind, FU.ext(name)));
        if (!thumbable(kind) || FU.ext(name).equals("svg")) { iv.setTag(null); return; }
        final String key = f.getPath() + "|" + stamp + "|" + px;
        iv.setTag(key);
        Bitmap hit = cache.get(key);
        if (hit != null) { iv.setImageBitmap(hit); return; }
        final Context app = iv.getContext().getApplicationContext();
        pool.execute(() -> {
            if (!key.equals(iv.getTag())) return; // 已经滚走了
            Bitmap b = cache.get(key);
            if (b == null) try { b = kind == FU.IMAGE ? image(f, px) : kind == FU.VIDEO ? video(f, px) : apk(app, f, px); } catch (Throwable ignored) {}
            if (b == null) return;
            cache.put(key, b);
            final Bitmap out = b;
            ui.post(() -> { if (key.equals(iv.getTag())) iv.setImageBitmap(out); });
        });
    }

    static Bitmap image(File f, int px) throws Exception {
        BitmapFactory.Options o = new BitmapFactory.Options();
        o.inJustDecodeBounds = true;
        BitmapFactory.decodeFile(f.getPath(), o);
        if (o.outWidth <= 0 || o.outHeight <= 0) return null;
        int s = 1;
        while (o.outWidth / (s * 2) >= px && o.outHeight / (s * 2) >= px) s *= 2;
        o = new BitmapFactory.Options();
        o.inSampleSize = s;
        o.inPreferredConfig = Bitmap.Config.RGB_565;
        Bitmap b = BitmapFactory.decodeFile(f.getPath(), o);
        if (b == null) return null;
        int rot = 0;
        try {
            String e = FU.ext(f.getName());
            if (e.equals("jpg") || e.equals("jpeg") || e.equals("heic") || e.equals("heif")) {
                int or = new android.media.ExifInterface(f.getPath()).getAttributeInt("Orientation", 1);
                rot = or == 6 ? 90 : or == 3 ? 180 : or == 8 ? 270 : 0;
            }
        } catch (Throwable ignored) {}
        return fit(b, px, rot);
    }
    static Bitmap fit(Bitmap b, int px, int rot) {
        float k = Math.min(1f, px / (float) Math.max(b.getWidth(), b.getHeight()));
        if (k >= 1f && rot == 0) return b;
        Matrix m = new Matrix();
        m.postScale(k, k);
        if (rot != 0) m.postRotate(rot);
        Bitmap out = Bitmap.createBitmap(b, 0, 0, b.getWidth(), b.getHeight(), m, true);
        if (out != b) b.recycle();
        return out;
    }
    static Bitmap video(File f, int px) throws Exception {
        Bitmap b = Build.VERSION.SDK_INT >= 29 ? ThumbnailUtils.createVideoThumbnail(f, new Size(px, px), null)
            : ThumbnailUtils.createVideoThumbnail(f.getPath(), 1 /* MINI_KIND */);
        return b == null ? null : fit(b, px, 0);
    }
    static Bitmap apk(Context c, File f, int px) {
        PackageInfo pi = c.getPackageManager().getPackageArchiveInfo(f.getPath(), 0);
        if (pi == null || pi.applicationInfo == null) return null;
        ApplicationInfo ai = pi.applicationInfo;
        ai.sourceDir = ai.publicSourceDir = f.getPath();
        Drawable d = ai.loadIcon(c.getPackageManager());
        if (d == null) return null;
        Bitmap b = Bitmap.createBitmap(px, px, Bitmap.Config.ARGB_8888);
        d.setBounds(0, 0, px, px);
        d.draw(new Canvas(b));
        return b;
    }
}
