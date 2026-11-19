package local.cloud.launcher;

import android.content.Context;
import android.os.Build;
import android.os.UserManager;
import java.io.File;

/**
 * 桌面自己的数据（设置、应用缓存、图标、壁纸）放在「设备级」存储里，
 * 这样开机时用户存储还没解锁，桌面就能直接显示，不用等系统的过渡界面。
 */
final class Store {
    private static Context de;
    private static boolean moved;

    static synchronized Context ctx(Context c) {
        Context app = c.getApplicationContext() != null ? c.getApplicationContext() : c;
        if (Build.VERSION.SDK_INT < 24) return app;
        if (de == null) de = app.createDeviceProtectedStorageContext();
        if (!moved && unlocked(app)) {
            moved = true;
            try {
                // 旧版本的数据在用户存储里：搬一次
                if (!new File(de.getDataDir(), "shared_prefs/desk.xml").exists()) de.moveSharedPreferencesFrom(app, "desk");
                File old = new File(app.getFilesDir(), "wallpaper.jpg"), now = new File(de.getFilesDir(), "wallpaper.jpg");
                if (old.exists() && !now.exists()) old.renameTo(now);
            } catch (Throwable ignored) {}
        }
        return de;
    }
    static boolean unlocked(Context c) {
        if (Build.VERSION.SDK_INT < 24) return true;
        try { return ((UserManager) c.getSystemService(Context.USER_SERVICE)).isUserUnlocked(); } catch (Throwable e) { return true; }
    }
}
