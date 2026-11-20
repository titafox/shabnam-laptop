package local.cloud.launcher;

import android.app.Activity;
import android.app.ActivityManager;
import android.content.Intent;
import android.database.Cursor;
import android.graphics.*;
import android.graphics.pdf.PdfRenderer;
import android.net.Uri;
import android.os.*;
import android.provider.OpenableColumns;
import android.text.Editable;
import android.text.TextWatcher;
import android.util.Log;
import android.util.LruCache;
import android.view.*;
import android.widget.*;
import java.io.*;
import java.nio.charset.StandardCharsets;

/**
 * 内置查看器：图片、文本（可改可存）、PDF、音视频。
 * 机器上不装第三方应用，文件管理里双击文件就落到这里；每个文件一个窗口。
 */
public class Viewer extends Activity {
    static final String TAG = "Viewer";
    static final int MAX_TEXT = 2 << 20;
    Uri uri;
    String name = "", mime = "";
    final Handler ui = new Handler(Looper.getMainLooper());
    // 文本
    EditText edit; boolean dirty, readOnly;
    // 图片
    Pic pic;
    // PDF
    PdfRenderer pdf; ParcelFileDescriptor pfd; LruCache<Integer, Bitmap> pages; HandlerThread pdfThread; Handler pdfH;
    // 音视频
    VideoView video;

    static boolean handles(String m) {
        return m != null && (m.startsWith("text/") || m.startsWith("image/") || m.startsWith("audio/") || m.startsWith("video/")
            || m.equals("application/pdf") || m.equals("application/json") || m.equals("application/xml") || m.equals("application/x-sh"));
    }
    protected void onCreate(Bundle state) {
        boolean dark = true;
        try { L.init(this); } catch (Throwable ignored) {}
        try {
            Bundle b = getContentResolver().call(Uri.parse("content://" + FilesProvider.AUTH), "dark", null, null);
            if (b != null && b.containsKey("dark")) dark = b.getBoolean("dark", true);
        } catch (Throwable ignored) {}
        setTheme(dark ? android.R.style.Theme_DeviceDefault_NoActionBar : android.R.style.Theme_DeviceDefault_Light_NoActionBar);
        super.onCreate(state);
        try {
            android.util.DisplayMetrics dm = new android.util.DisplayMetrics();
            ((WindowManager) getApplicationContext().getSystemService(WINDOW_SERVICE)).getDefaultDisplay().getRealMetrics(dm);
            T.u = Math.max(dm.widthPixels, dm.heightPixels) / 1280f;
            T.apply(dark);
            uri = getIntent().getData();
            if (uri == null) { finish(); return; }
            name = displayName();
            mime = String.valueOf(getIntent().getType() != null ? getIntent().getType() : getContentResolver().getType(uri));
            if (mime.equals("null") || mime.equals("*/*") || mime.equals("application/octet-stream")) mime = FU.mime(new File(name));
            title(name);
            if (mime.startsWith("image/")) image();
            else if (mime.equals("application/pdf")) pdf();
            else if (mime.startsWith("video/") || mime.startsWith("audio/")) media(mime.startsWith("audio/"));
            else text();
        } catch (Throwable e) {
            Log.w(TAG, "open: " + e);
            fail(String.valueOf(e.getMessage() != null ? e.getMessage() : e));
        }
    }
    void title(String s) {
        setTitle(s);
        try { setTaskDescription(new ActivityManager.TaskDescription(s)); } catch (Throwable ignored) {}
    }
    String displayName() {
        try (Cursor c = getContentResolver().query(uri, new String[]{OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (c != null && c.moveToFirst() && c.getString(0) != null) return c.getString(0);
        } catch (Throwable ignored) {}
        String s = uri.getLastPathSegment();
        return s == null ? "" : s.substring(s.lastIndexOf('/') + 1);
    }
    TextView label(String s, float size, int color) {
        TextView t = new TextView(this);
        t.setText(s); t.setTextColor(color); t.setTextSize(TypedValue_PX, T.px(size));
        return t;
    }
    static final int TypedValue_PX = android.util.TypedValue.COMPLEX_UNIT_PX;
    void fail(String why) {
        TextView t = label(L.t("打不开这个文件\n", "Не удалось открыть файл\n", "Could not open this file\n") + why, 13, T.text2);
        t.setGravity(Gravity.CENTER);
        t.setBackgroundColor(T.dark ? 0xFF202124 : 0xFFFFFFFF);
        setContentView(t);
    }
    int bg() { return T.dark ? 0xFF202124 : 0xFFFFFFFF; }

    // ================= 文本 =================
    void text() throws IOException {
        byte[] buf;
        boolean cut = false;
        try (InputStream in = getContentResolver().openInputStream(uri)) {
            ByteArrayOutputStream bo = new ByteArrayOutputStream();
            byte[] b = new byte[65536];
            int n;
            while ((n = in.read(b)) > 0) { bo.write(b, 0, n); if (bo.size() > MAX_TEXT) { cut = true; break; } }
            buf = bo.toByteArray();
        }
        int zeros = 0;
        for (int i = 0; i < buf.length && i < 4096; i++) if (buf[i] == 0) zeros++;
        if (zeros > 4) { fail(L.t("这不是文本文件，没有能打开它的应用", "Это не текстовый файл; нет приложения, которое может его открыть", "This is not a text file and nothing here can open it")); return; }
        readOnly = cut;
        edit = new EditText(this);
        edit.setTypeface(Typeface.MONOSPACE);
        edit.setTextSize(TypedValue_PX, T.px(13f));
        edit.setTextColor(T.text);
        edit.setBackgroundColor(bg());
        edit.setGravity(Gravity.TOP | Gravity.START);
        edit.setPadding(T.px(14), T.px(10), T.px(14), T.px(10));
        edit.setHorizontallyScrolling(false);
        edit.setInputType(android.text.InputType.TYPE_CLASS_TEXT | android.text.InputType.TYPE_TEXT_FLAG_MULTI_LINE | android.text.InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
        edit.setText(new String(buf, StandardCharsets.UTF_8));
        edit.setSelection(0);
        if (readOnly) { edit.setKeyListener(null); edit.setTextIsSelectable(true); title(name + L.t("（太大，只显示开头，只读）", " (слишком большой: показано начало, только чтение)", " (too large: beginning only, read-only)")); }
        edit.addTextChangedListener(new TextWatcher() {
            public void beforeTextChanged(CharSequence s, int a, int b, int c) {}
            public void onTextChanged(CharSequence s, int a, int b, int c) {}
            public void afterTextChanged(Editable e) { if (!dirty && !readOnly) { dirty = true; title("● " + name); } }
        });
        setContentView(edit);
        edit.requestFocus();
    }
    boolean save() {
        if (edit == null || readOnly) return false;
        try (OutputStream o = getContentResolver().openOutputStream(uri, "wt")) {
            o.write(edit.getText().toString().getBytes(StandardCharsets.UTF_8));
            dirty = false;
            title(name);
            Toast.makeText(this, L.t("已保存", "Сохранено", "Saved"), Toast.LENGTH_SHORT).show();
            return true;
        } catch (Throwable e) {
            Toast.makeText(this, L.t("保存失败：", "Не удалось сохранить: ", "Could not save: ") + e.getMessage(), Toast.LENGTH_LONG).show();
            return false;
        }
    }
    /** 有没保存的改动时，关窗口前问一句。 */
    public void onBackPressed() { if (!askSave()) super.onBackPressed(); }
    boolean askSave() {
        if (!dirty) return false;
        new android.app.AlertDialog.Builder(this)
            .setMessage(L.t("保存对「", "Сохранить изменения в «", "Save changes to \"") + name + L.t("」的修改吗？", "»?", "\"?"))
            .setPositiveButton(L.t("保存", "Сохранить", "Save"), (d, w) -> { if (save()) finish(); })
            .setNegativeButton(L.t("不保存", "Не сохранять", "Don't save"), (d, w) -> { dirty = false; finish(); })
            .setNeutralButton(L.t("取消", "Отмена", "Cancel"), null).show();
        return true;
    }

    // ================= 图片 =================
    void image() throws IOException {
        BitmapFactory.Options o = new BitmapFactory.Options();
        o.inJustDecodeBounds = true;
        try (InputStream in = getContentResolver().openInputStream(uri)) { BitmapFactory.decodeStream(in, null, o); }
        int s = 1;
        while (Math.max(o.outWidth, o.outHeight) / s > 4096) s *= 2;
        o = new BitmapFactory.Options();
        o.inSampleSize = s;
        Bitmap bm;
        try (InputStream in = getContentResolver().openInputStream(uri)) { bm = BitmapFactory.decodeStream(in, null, o); }
        if (bm == null) throw new IOException(L.t("图片格式不支持", "Формат изображения не поддерживается", "Unsupported image format"));
        pic = new Pic(bm);
        setContentView(pic);
        pic.setFocusable(true);
        pic.requestFocus();
        title(name + "  ·  " + bm.getWidth() * s + "×" + bm.getHeight() * s);
    }
    /** 滚轮缩放（以指针为中心）、按住拖动、双击在"适应窗口"和"原始大小"之间切换。 */
    final class Pic extends View {
        final Bitmap bm; final Matrix m = new Matrix(); final Paint p = new Paint(Paint.FILTER_BITMAP_FLAG | Paint.ANTI_ALIAS_FLAG);
        float scale; boolean fitted = true; float lx, ly; long lastUp;
        Pic(Bitmap b) { super(Viewer.this); bm = b; setBackgroundColor(T.dark ? 0xFF17181B : 0xFFF1F3F4); }
        float fitScale() { return Math.min(1f, Math.min(getWidth() / (float) bm.getWidth(), getHeight() / (float) bm.getHeight())); }
        void fit() {
            scale = fitScale();
            m.setScale(scale, scale);
            m.postTranslate((getWidth() - bm.getWidth() * scale) / 2f, (getHeight() - bm.getHeight() * scale) / 2f);
            fitted = true;
            invalidate();
        }
        void zoom(float f, float cx, float cy) {
            float ns = Math.max(Math.min(fitScale(), 0.05f), Math.min(16f, scale * f));
            f = ns / scale;
            if (f == 1f) return;
            m.postScale(f, f, cx, cy);
            scale = ns;
            fitted = false;
            clamp();
        }
        /** 图比窗口小的方向居中，大的方向不让拖出边。 */
        void clamp() {
            RectF r = new RectF(0, 0, bm.getWidth(), bm.getHeight());
            m.mapRect(r);
            float dx = 0, dy = 0;
            if (r.width() <= getWidth()) dx = (getWidth() - r.width()) / 2f - r.left;
            else if (r.left > 0) dx = -r.left; else if (r.right < getWidth()) dx = getWidth() - r.right;
            if (r.height() <= getHeight()) dy = (getHeight() - r.height()) / 2f - r.top;
            else if (r.top > 0) dy = -r.top; else if (r.bottom < getHeight()) dy = getHeight() - r.bottom;
            m.postTranslate(dx, dy);
            invalidate();
        }
        protected void onSizeChanged(int w, int h, int ow, int oh) { if (fitted || scale == 0) fit(); else clamp(); }
        protected void onDraw(Canvas c) { c.drawBitmap(bm, m, p); }
        public boolean onGenericMotionEvent(MotionEvent e) {
            if (e.getAction() == MotionEvent.ACTION_SCROLL) {
                float v = e.getAxisValue(MotionEvent.AXIS_VSCROLL);
                if (v != 0) zoom(v > 0 ? 1.15f : 1 / 1.15f, e.getX(), e.getY());
                return true;
            }
            return super.onGenericMotionEvent(e);
        }
        public boolean onTouchEvent(MotionEvent e) {
            switch (e.getActionMasked()) {
                case MotionEvent.ACTION_DOWN: lx = e.getX(); ly = e.getY(); return true;
                case MotionEvent.ACTION_MOVE: m.postTranslate(e.getX() - lx, e.getY() - ly); lx = e.getX(); ly = e.getY(); fitted = false; clamp(); return true;
                case MotionEvent.ACTION_UP:
                    long now = e.getEventTime();
                    if (now - lastUp < 350) { if (fitted && fitScale() < 1f) { fit(); zoom(1f / scale, e.getX(), e.getY()); } else fit(); lastUp = 0; }
                    else lastUp = now;
                    return true;
            }
            return true;
        }
    }

    // ================= PDF =================
    void pdf() throws IOException {
        pfd = getContentResolver().openFileDescriptor(uri, "r");
        pdf = new PdfRenderer(pfd);
        pdfThread = new HandlerThread("pdf");
        pdfThread.start();
        pdfH = new Handler(pdfThread.getLooper());
        pages = new LruCache<Integer, Bitmap>(24 << 20) { protected int sizeOf(Integer k, Bitmap b) { return b.getByteCount(); } };
        final int n = pdf.getPageCount();
        final float[] ratio = new float[n];   // 每页的高宽比，用来在渲染出来之前就把位置占好
        final ListView list = new ListView(this);
        list.setDivider(null);
        list.setBackgroundColor(T.dark ? 0xFF17181B : 0xFFE8EAED);
        list.setAdapter(new BaseAdapter() {
            public int getCount() { return n; }
            public Object getItem(int i) { return i; }
            public long getItemId(int i) { return i; }
            public View getView(final int i, View v, ViewGroup parent) {
                final ImageView iv = v != null ? (ImageView) v : new ImageView(Viewer.this);
                final int w = Math.max(200, Math.min(parent.getWidth() - T.px(24), T.px(900)));
                int h = Math.round(w * (ratio[i] > 0 ? ratio[i] : 1.414f));
                int side = (parent.getWidth() - w) / 2;
                iv.setPadding(side, T.px(6), side, T.px(6));
                iv.setLayoutParams(new AbsListView.LayoutParams(-1, h + T.px(12)));
                iv.setScaleType(ImageView.ScaleType.FIT_XY);
                iv.setTag(i);
                final int key = i * 8192 + Math.min(w, 8191);
                Bitmap have = pages.get(key);
                iv.setImageBitmap(have);
                if (have == null) pdfH.post(() -> {
                    if (!Integer.valueOf(i).equals(iv.getTag())) return;   // 已经滚过去了
                    try {
                        Bitmap b;
                        float r;
                        synchronized (Viewer.this) {
                            if (pdf == null) return;
                            PdfRenderer.Page pg = pdf.openPage(i);
                            try {
                                r = pg.getHeight() / (float) pg.getWidth();
                                b = Bitmap.createBitmap(w, Math.max(1, Math.round(w * r)), Bitmap.Config.ARGB_8888);
                                b.eraseColor(0xFFFFFFFF);
                                pg.render(b, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY);
                            } finally { pg.close(); }
                        }
                        final Bitmap fb = b; final float fr = r;
                        ui.post(() -> {
                            pages.put(key, fb);
                            boolean changed = Math.abs(ratio[i] - fr) > 0.001f;
                            ratio[i] = fr;
                            if (changed) ((BaseAdapter) list.getAdapter()).notifyDataSetChanged();
                            else if (Integer.valueOf(i).equals(iv.getTag())) iv.setImageBitmap(fb);
                        });
                    } catch (Throwable e) { Log.w(TAG, "page " + i + ": " + e); }
                });
                return iv;
            }
        });
        list.setOnScrollListener(new AbsListView.OnScrollListener() {
            public void onScrollStateChanged(AbsListView v, int s) {}
            public void onScroll(AbsListView v, int first, int vis, int total) { title(name + "  ·  " + (first + 1) + " / " + n); }
        });
        setContentView(list);
        list.requestFocus();
    }

    // ================= 音视频 =================
    void media(boolean audio) {
        FrameLayout root = new FrameLayout(this);
        root.setBackgroundColor(0xFF000000);
        video = new VideoView(this);
        root.addView(video, new FrameLayout.LayoutParams(-1, -1, Gravity.CENTER));
        if (audio) {
            TextView t = label("♪  " + name, 15, 0xFFE8EAED);
            t.setGravity(Gravity.CENTER);
            root.addView(t, new FrameLayout.LayoutParams(-1, -2, Gravity.CENTER));
        }
        final MediaController mc = new MediaController(this);
        mc.setAnchorView(root);
        video.setMediaController(mc);
        video.setVideoURI(uri);
        video.setOnPreparedListener(mp -> { video.start(); mc.show(audio ? 0 : 3000); });
        video.setOnErrorListener((mp, what, extra) -> { fail(L.t("这个格式放不了", "Этот формат не воспроизводится", "This format cannot be played") + " (" + what + "/" + extra + ")"); return true; });
        setContentView(root);
        video.requestFocus();
    }

    // ================= 键盘 =================
    public boolean dispatchKeyEvent(KeyEvent e) {
        if (e.getAction() == KeyEvent.ACTION_DOWN) {
            int k = e.getKeyCode();
            if (e.isCtrlPressed() && k == KeyEvent.KEYCODE_S && edit != null) { save(); return true; }
            if (e.isCtrlPressed() && k == KeyEvent.KEYCODE_W) { if (!askSave()) finish(); return true; }
            if (k == KeyEvent.KEYCODE_ESCAPE && edit == null) { finish(); return true; }
            if (pic != null) {
                float cx = pic.getWidth() / 2f, cy = pic.getHeight() / 2f;
                if (k == KeyEvent.KEYCODE_EQUALS || k == KeyEvent.KEYCODE_PLUS || k == KeyEvent.KEYCODE_NUMPAD_ADD) { pic.zoom(1.25f, cx, cy); return true; }
                if (k == KeyEvent.KEYCODE_MINUS || k == KeyEvent.KEYCODE_NUMPAD_SUBTRACT) { pic.zoom(0.8f, cx, cy); return true; }
                if (k == KeyEvent.KEYCODE_0) { pic.fit(); return true; }
                if (k == KeyEvent.KEYCODE_1) { pic.fit(); pic.zoom(1f / pic.scale, cx, cy); return true; }
            }
            if (video != null) {
                if (k == KeyEvent.KEYCODE_SPACE) { if (video.isPlaying()) video.pause(); else video.start(); return true; }
                if (k == KeyEvent.KEYCODE_DPAD_LEFT) { video.seekTo(Math.max(0, video.getCurrentPosition() - 5000)); return true; }
                if (k == KeyEvent.KEYCODE_DPAD_RIGHT) { video.seekTo(video.getCurrentPosition() + 5000); return true; }
            }
        }
        return super.dispatchKeyEvent(e);
    }

    protected void onDestroy() {
        super.onDestroy();
        try { if (pdfThread != null) pdfThread.quitSafely(); } catch (Throwable ignored) {}
        synchronized (this) {
            try { if (pdf != null) pdf.close(); } catch (Throwable ignored) {}
            pdf = null;
        }
        try { if (pfd != null) pfd.close(); } catch (Throwable ignored) {}
        try { if (video != null) video.stopPlayback(); } catch (Throwable ignored) {}
    }
}
