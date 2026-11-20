package local.cloud.launcher;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;

/**
 * 不点界面也能操作托盘功能（未导出，只有 root / 系统能发）：
 *   am broadcast -n local.cloud.launcher/.TrayCmd --es statusbar hide     隐藏系统状态栏
 *   am broadcast -n local.cloud.launcher/.TrayCmd --es statusbar show     恢复
 *   am broadcast -n local.cloud.launcher/.TrayCmd --es panel toggle       开 / 关快捷面板
 */
public class TrayCmd extends BroadcastReceiver {
    public void onReceive(final Context ctx, Intent i) {
        try {
            final Context c = ctx.getApplicationContext();
            String out = "";
            String sb = i.getStringExtra("statusbar");
            if (sb != null) {
                String err = StatusBar.set(c, "hide".equals(sb));
                out = "statusbar " + sb + (err == null ? ": ok" : ": failed: " + err);
                new Handler(Looper.getMainLooper()).postDelayed(() -> StatusBar.sync(c), 1500);
            }
            if (i.getStringExtra("panel") != null) { QuickPanel.toggle(c); out += " panel toggled"; }
            if (isOrderedBroadcast()) setResultData(out.trim());
        } catch (Throwable e) { Log.w("TrayCmd", String.valueOf(e)); }
    }
}
