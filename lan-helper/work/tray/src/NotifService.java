package local.cloud.launcher;

import android.service.notification.NotificationListenerService;
import android.service.notification.StatusBarNotification;
import android.util.Log;

/**
 * 通知监听服务：只负责把系统回调转给 Notif。回调都在主线程。
 * 授权（无需用户操作）：cmd notification allow_listener local.cloud.launcher/local.cloud.launcher.NotifService
 * Notif.init() 启动时也会用 MANAGE_NOTIFICATION_LISTENERS 自己授权一次。
 */
public class NotifService extends NotificationListenerService {
    static final String TAG = "NotifService";

    @Override public void onListenerConnected() {
        try { Notif.connected(this); } catch (Throwable e) { Log.w(TAG, "connected: " + e); }
    }
    @Override public void onListenerDisconnected() {
        try { Notif.disconnected(this); } catch (Throwable e) { Log.w(TAG, "disconnected: " + e); }
    }
    @Override public void onNotificationPosted(StatusBarNotification sbn, RankingMap ranking) {
        try { Notif.posted(sbn, ranking); } catch (Throwable e) { Log.w(TAG, "posted: " + e); }
    }
    @Override public void onNotificationRemoved(StatusBarNotification sbn) {
        try { Notif.removed(sbn); } catch (Throwable e) { Log.w(TAG, "removed: " + e); }
    }
}
