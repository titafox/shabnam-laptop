package local.cloud.launcher;

import android.content.ContentResolver;
import android.content.Context;
import android.content.SharedPreferences;
import android.content.res.Resources;
import android.provider.Settings;
import android.util.Log;
import android.util.TypedValue;
import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.List;

/**
 * 隐藏系统顶部状态栏：给 framework 资源注册一个「运行时叠加层」（fabricated overlay），
 * 把 android:dimen/status_bar_height_default 改成 0。安卓 14 QPR 以后状态栏高度只看这一个值
 * （本机厂商叠加层早已把 status_bar_height_portrait 设成 0，却不起作用，正是这个原因）。
 *
 * 叠加层归本应用所有，标识是  local.cloud.launcher:OpenDesktopNoStatusBar ，状态存在系统的叠加层管理器里，
 * 重启后仍然有效。撤销只要一条命令：
 *     cmd overlay disable --user 0 local.cloud.launcher:OpenDesktopNoStatusBar
 * 需要平台签名带来的 CHANGE_OVERLAY_PACKAGES。全部走反射，失败只返回错误文字。
 */
final class StatusBar {
    static final String TAG = "StatusBar";
    static final String NAME = "OpenDesktopNoStatusBar";
    static final String RES = "android:dimen/status_bar_height_default";
    static final String HUN = "heads_up_notifications_enabled";

    static Method m(Class<?> k, String name, Class<?>... types) throws Exception {
        Method x;
        try { x = k.getMethod(name, types); } catch (NoSuchMethodException e) { x = k.getDeclaredMethod(name, types); }
        x.setAccessible(true);
        return x;
    }
    static int user() { return android.os.Process.myUserHandle().hashCode(); }

    static Object overlay(Context c) throws Exception {
        Class<?> fk = Class.forName("android.content.om.FabricatedOverlay");
        Object ov = fk.getConstructor(String.class, String.class).newInstance(NAME, "android");
        m(fk, "setOwningPackage", String.class).invoke(ov, c.getPackageName());
        return ov;
    }

    /** 状态栏现在是否被隐藏（我们的叠加层已启用，或别的途径把高度改成了 0）。 */
    static boolean hidden(Context c) {
        try {
            Object ov = overlay(c);
            Object id = m(ov.getClass(), "getIdentifier").invoke(ov);
            Object om = c.getSystemService("overlay");
            Object info = m(om.getClass(), "getOverlayInfo", id.getClass(), android.os.UserHandle.class).invoke(om, id, android.os.Process.myUserHandle());
            if (info != null && Boolean.TRUE.equals(m(info.getClass(), "isEnabled").invoke(info))) return true;
        } catch (Throwable e) { Log.w(TAG, "query: " + e); }
        try {
            Resources r = c.getApplicationContext().getResources();
            int id = r.getIdentifier("status_bar_height_default", "dimen", "android");
            return id != 0 && r.getDimensionPixelSize(id) == 0;
        } catch (Throwable e) { return false; }
    }

    /** 隐藏 / 恢复。成功返回 null，失败返回原因。 */
    static String set(Context c, boolean hide) {
        try {
            Class<?> bk = Class.forName("android.content.om.OverlayManagerTransaction$Builder");
            Class<?> tk = Class.forName("android.content.om.OverlayManagerTransaction");
            Object ov = overlay(c);
            Class<?> fk = ov.getClass();
            Object id = m(fk, "getIdentifier").invoke(ov);
            Constructor<?> bc = bk.getDeclaredConstructor();
            bc.setAccessible(true);
            Object b = bc.newInstance();
            if (hide) {
                zeroHeight(fk, ov);
                m(bk, "registerFabricatedOverlay", fk).invoke(b, ov);
                m(bk, "setEnabled", id.getClass(), boolean.class, int.class).invoke(b, id, true, user());
            } else {
                m(bk, "unregisterFabricatedOverlay", id.getClass()).invoke(b, id);
            }
            Object tx = m(bk, "build").invoke(b);
            Object om = c.getSystemService("overlay");
            m(om.getClass(), "commit", tk).invoke(om, tx);
            Log.i(TAG, "status bar " + (hide ? "hidden" : "restored"));
            return null;
        } catch (Throwable e) {
            Throwable x = e.getCause() != null ? e.getCause() : e;
            Log.w(TAG, "set(" + hide + "): " + x);
            return String.valueOf(x);
        }
    }

    /** 往叠加层里写入「高度 = 0dp」。整数版 setResourceValue 只收 0x10–0x1f 的类型，尺寸要用浮点版。 */
    @SuppressWarnings("unchecked")
    static void zeroHeight(Class<?> fk, Object ov) throws Exception {
        try {
            m(fk, "setResourceValue", String.class, float.class, int.class, String.class).invoke(ov, RES, 0.5f, TypedValue.COMPLEX_UNIT_DIP, null);
            return;
        } catch (Throwable e) { Log.w(TAG, "dimension setter: " + e); }
        // 退路：直接往内部条目表里加一条 TYPE_DIMENSION
        Field f = fk.getDeclaredField("mOverlay");
        f.setAccessible(true);
        Object internal = f.get(ov);
        Object entry = Class.forName("android.os.FabricatedOverlayInternalEntry").getConstructor().newInstance();
        entry.getClass().getField("resourceName").set(entry, RES);
        entry.getClass().getField("dataType").setInt(entry, TypedValue.TYPE_DIMENSION);
        entry.getClass().getField("data").setInt(entry, TypedValue.COMPLEX_UNIT_DIP); // 数值 0，单位 dp
        ((List<Object>) internal.getClass().getField("entries").get(internal)).add(entry);
    }

    /**
     * 状态栏隐藏期间关掉系统自带的横幅通知（它会从屏幕顶部弹出，和我们右下角的卡片重复），恢复时再打开。
     * 只在「我们自己关的」情况下才会重新打开，不动用户自己的设置。
     */
    /**
     * 状态栏压到 1 像素后，时钟和图标的最上面一行像素还会露出来（屏幕顶部的残影）。
     * 隐藏期间让系统不画这些内容；标志跟着本进程走，进程没了系统自动撤销，所以每次启动都要再设一遍。
     */
    static void blank(Context c, boolean on) {
        try {
            Object sb = c.getSystemService("statusbar");
            int flags = on ? 0x00800000 /* 时钟 */ | 0x00100000 /* 右侧系统图标 */ | 0x00020000 /* 通知图标 */ : 0;
            m(sb.getClass(), "disable", int.class).invoke(sb, flags);
        } catch (Throwable e) { Log.w(TAG, "blank(" + on + "): " + (e.getCause() != null ? e.getCause() : e)); }
    }

    static void sync(Context c) {
        try {
            boolean h = hidden(c);
            blank(c, h);
            SharedPreferences sp = Store.ctx(c).getSharedPreferences("desk", 0);
            boolean mine = sp.getBoolean("hun_off", false);
            ContentResolver cr = c.getContentResolver();
            if (h && !mine) {
                if (Notif.live() && Settings.Global.getInt(cr, HUN, 1) != 0) {
                    Settings.Global.putInt(cr, HUN, 0);
                    sp.edit().putBoolean("hun_off", true).apply();
                }
            } else if (!h && mine) {
                Settings.Global.putInt(cr, HUN, 1);
                sp.edit().putBoolean("hun_off", false).apply();
            }
        } catch (Throwable e) { Log.w(TAG, "sync: " + e); }
    }
}
