package local.cloud.launcher;

import android.accessibilityservice.AccessibilityService;
import android.util.Log;
import android.view.KeyEvent;
import android.view.accessibility.AccessibilityEvent;
import java.util.HashSet;
import java.util.Set;

/**
 * 全局快捷键。无障碍服务的「按键过滤」能在任何应用之前拿到实体键盘的按键：
 *   Alt+Tab / Alt+Shift+Tab 切换窗口（松开 Alt 确认），Win 单按 = 启动器，
 *   Win+←/→ 左右半屏，Win+↑ 最大化，Win+↓ 还原 / 最小化，Win+D 显示桌面，Alt+F4 关闭。
 * 不读取任何窗口内容；窗口变化事件只用来提醒 Wm 看一眼任务列表。
 */
public class Keys extends AccessibilityService {
    static final String TAG = "Keys";
    static Keys it;
    boolean metaDown, metaUsed;
    final Set<Integer> eaten = new HashSet<>();

    protected void onServiceConnected() {
        it = this;
        try { Wm.init(getApplicationContext()); } catch (Throwable e) { Log.w(TAG, "init: " + e); }
        Log.i(TAG, "connected");
        Wm.ui.postDelayed(() -> Wm.ensureHome(), 1500);
    }
    public boolean onUnbind(android.content.Intent i) { it = null; return super.onUnbind(i); }
    public void onDestroy() { it = null; super.onDestroy(); }
    public void onInterrupt() {}
    public void onAccessibilityEvent(AccessibilityEvent e) {
        try { Wm.kick(); } catch (Throwable ignored) {}
    }

    protected boolean onKeyEvent(KeyEvent ev) {
        try { return handle(ev); }
        catch (Throwable e) { Log.w(TAG, "key: " + e); return false; }
    }

    boolean handle(KeyEvent ev) {
        final int code = ev.getKeyCode();
        final boolean down = ev.getAction() == KeyEvent.ACTION_DOWN;
        final boolean meta = code == KeyEvent.KEYCODE_META_LEFT || code == KeyEvent.KEYCODE_META_RIGHT;
        final boolean alt = code == KeyEvent.KEYCODE_ALT_LEFT || code == KeyEvent.KEYCODE_ALT_RIGHT;

        if (!down) {
            if (meta) {
                boolean alone = metaDown && !metaUsed;
                metaDown = false;
                if (alone) Wm.home("launcher");
                return true;
            }
            if (alt && Switcher.showing()) { Switcher.commit(); return false; }
            return eaten.remove(code);
        }

        if (meta) {
            if (ev.getRepeatCount() == 0) { metaDown = true; metaUsed = false; }
            return true;
        }
        if (eaten.contains(code)) return true; // 按住不放的重复
        boolean first = ev.getRepeatCount() == 0;

        if (Switcher.showing()) {
            int dir = 0;
            if (code == KeyEvent.KEYCODE_TAB) dir = ev.isShiftPressed() ? -1 : 1;
            else if (code == KeyEvent.KEYCODE_DPAD_RIGHT) dir = 1;
            else if (code == KeyEvent.KEYCODE_DPAD_LEFT) dir = -1;
            if (dir != 0) { Switcher.step(this, dir); return eat(code); }
            if (code == KeyEvent.KEYCODE_ESCAPE) { Switcher.cancel(); return eat(code); }
            if (code == KeyEvent.KEYCODE_ENTER) { Switcher.commit(); return eat(code); }
            if (!alt && !isShift(code)) Switcher.cancel();
        }

        if (metaDown || ev.isMetaPressed()) {
            metaUsed = true;
            if (ev.isAltPressed() || ev.isCtrlPressed()) return false; // 系统自己的组合键不抢
            switch (code) {
                case KeyEvent.KEYCODE_DPAD_LEFT: if (first) Wm.snap(Wm.LEFT); return eat(code);
                case KeyEvent.KEYCODE_DPAD_RIGHT: if (first) Wm.snap(Wm.RIGHT); return eat(code);
                case KeyEvent.KEYCODE_DPAD_UP: if (first) Wm.snap(Wm.MAX); return eat(code);
                case KeyEvent.KEYCODE_DPAD_DOWN: if (first) Wm.snap(Wm.DOWN); return eat(code);
                case KeyEvent.KEYCODE_D: if (first) Wm.desktop(); return eat(code);
                case KeyEvent.KEYCODE_V: if (first) Clip.show(this); return eat(code);
                case KeyEvent.KEYCODE_E: if (first) Files.start(this, null, null); return eat(code);
                case KeyEvent.KEYCODE_S: if (ev.isShiftPressed()) { if (first) Shot.region(this); return eat(code); } break;
            }
            return false;
        }

        if (code == KeyEvent.KEYCODE_SYSRQ) { if (first) { if (ev.isShiftPressed()) Shot.region(this); else Shot.full(this); } return eat(code); }
        if (ev.isAltPressed() && !ev.isCtrlPressed()) {
            if (code == KeyEvent.KEYCODE_TAB) { Switcher.step(this, ev.isShiftPressed() ? -1 : 1); return eat(code); }
            if (code == KeyEvent.KEYCODE_F4) { if (first) Wm.close(); return eat(code); }
        }
        return false;
    }
    boolean eat(int code) { eaten.add(code); return true; }
    static boolean isShift(int code) { return code == KeyEvent.KEYCODE_SHIFT_LEFT || code == KeyEvent.KEYCODE_SHIFT_RIGHT; }
}
