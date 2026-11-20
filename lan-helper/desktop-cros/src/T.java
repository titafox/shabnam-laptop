package local.cloud.launcher;

import android.graphics.*;
import android.graphics.drawable.*;

/** 主题色与尺寸换算。设计画布宽 1280 单位，按屏幕实际宽度等比换算成像素。 */
final class T {
    static boolean dark = true;
    static float u = 1.5f;
    static int panel, text, text2, accent, onAccent, shelf, chip, hover, plate, base;

    static void apply(boolean d) {
        dark = d;
        if (d) {
            panel = 0xF2202124; text = 0xFFE8EAED; text2 = 0xFF9AA0A6; accent = 0xFF8AB4F8; onAccent = 0xFF202124;
            shelf = 0x99121419; chip = 0x24FFFFFF; hover = 0x2E8AB4F8; base = 0xFF0C1424;
        } else {
            panel = 0xF5FFFFFF; text = 0xFF202124; text2 = 0xFF5F6368; accent = 0xFF1A73E8; onAccent = 0xFFFFFFFF;
            shelf = 0xA6FFFFFF; chip = 0x14000000; hover = 0x261A73E8; base = 0xFFCFDCF2;
        }
        plate = 0xFFF1F3F4;
    }

    static int px(float v) { return Math.round(v * u); }

    static GradientDrawable round(int color, float radius) {
        GradientDrawable g = new GradientDrawable();
        g.setColor(color);
        g.setCornerRadius(radius);
        return g;
    }

    /** 悬停 / 按下 / 键盘选中时的高亮底。 */
    static Drawable state(float radius) {
        StateListDrawable s = new StateListDrawable();
        Drawable hi = round(hover, radius);
        s.addState(new int[]{android.R.attr.state_pressed}, hi);
        s.addState(new int[]{android.R.attr.state_activated}, hi);
        s.addState(new int[]{android.R.attr.state_hovered}, hi);
        s.addState(new int[0], new ColorDrawable(0));
        return s;
    }

    /** 小图形：自己画，不依赖任何图标资源。 */
    static final class Glyph extends Drawable {
        static final int HOME = 0, NET = 1, BATTERY = 2;
        final int type;
        final Paint p = new Paint(Paint.ANTI_ALIAS_FLAG);
        int color;
        float level = 1f;
        boolean on = true;

        Glyph(int type, int color) { this.type = type; this.color = color; }

        public void draw(Canvas c) {
            Rect b = getBounds();
            float w = b.width(), h = b.height(), x = b.left, y = b.top;
            p.setColor(color);
            p.setAlpha(on ? 255 : 90);
            if (type == HOME) {
                p.setStyle(Paint.Style.STROKE);
                p.setStrokeWidth(w * 0.13f);
                c.drawCircle(b.exactCenterX(), b.exactCenterY(), w * 0.30f, p);
            } else if (type == NET) {
                p.setStyle(Paint.Style.FILL);
                float r = w * 0.72f;
                RectF o = new RectF(x + w / 2 - r, y + h * 0.88f - r, x + w / 2 + r, y + h * 0.88f + r);
                c.drawArc(o, -135, 90, true, p);
            } else {
                float sw = Math.max(1f, h * 0.09f);
                p.setStyle(Paint.Style.STROKE);
                p.setStrokeWidth(sw);
                RectF o = new RectF(x + sw, y + h * 0.24f, x + w * 0.88f, y + h * 0.76f);
                c.drawRoundRect(o, sw, sw, p);
                p.setStyle(Paint.Style.FILL);
                c.drawRect(x + w * 0.90f, y + h * 0.40f, x + w, y + h * 0.60f, p);
                float in = sw * 1.6f;
                c.drawRect(o.left + in, o.top + in, o.left + in + (o.width() - 2 * in) * Math.max(0.05f, level), o.bottom - in, p);
            }
        }
        public void setAlpha(int a) {}
        public void setColorFilter(ColorFilter f) {}
        public int getOpacity() { return PixelFormat.TRANSLUCENT; }
    }

    /** 内置壁纸：线性底色加几团柔光，按半分辨率生成后拉伸。 */
    static final String[] WALLS = {"极光", "暮色", "深海", "晨雾"};
    static Bitmap wallpaper(int w, int h, int idx) {
        int[][] s = {
            {0xFF1A2848, 0xFF0C1424, 0x997AA8FF, 82, 18, 60, 0x6B40C4B2, 12, 88, 65, 0x40966EF0, 55, 60, 45},
            {0xFF2B1B3D, 0xFF120E22, 0x99FF8A65, 80, 85, 60, 0x80C06CF0, 15, 20, 65, 0x40FFD27A, 50, 55, 40},
            {0xFF06283A, 0xFF030F1A, 0x8029B6C9, 20, 20, 65, 0x664F8EF7, 85, 80, 60, 0x3334C08B, 55, 40, 45},
            {0xFFE9EEF8, 0xFFCFDCF2, 0xBFFFC48C, 82, 18, 60, 0xB382BEFF, 12, 88, 65, 0x59C8A8FF, 50, 55, 45},
        };
        int[] v = s[Math.max(0, Math.min(s.length - 1, idx))];
        int bw = Math.max(2, w / 2), bh = Math.max(2, h / 2);
        Bitmap b = Bitmap.createBitmap(bw, bh, Bitmap.Config.ARGB_8888);
        Canvas c = new Canvas(b);
        Paint p = new Paint(Paint.DITHER_FLAG);
        p.setShader(new LinearGradient(0, 0, bw, bh, v[0], v[1], Shader.TileMode.CLAMP));
        c.drawRect(0, 0, bw, bh, p);
        for (int i = 2; i + 3 < v.length; i += 4) {
            float r = bw * v[i + 3] / 100f;
            p.setShader(new RadialGradient(bw * v[i + 1] / 100f, bh * v[i + 2] / 100f, r, v[i], v[i] & 0x00FFFFFF, Shader.TileMode.CLAMP));
            c.drawRect(0, 0, bw, bh, p);
        }
        return b;
    }
}
