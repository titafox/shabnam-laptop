package local.cloud.launcher;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.net.ConnectivityManager;
import android.net.LinkProperties;
import android.net.Network;
import android.net.NetworkRequest;
import android.net.Uri;
import android.net.wifi.WifiManager;
import android.os.Build;
import android.os.Environment;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.provider.MediaStore;
import android.util.Log;
import android.webkit.MimeTypeMap;
import java.io.File;
import java.io.IOException;
import java.io.OutputStream;
import java.util.Locale;

/**
 * 互传服务：随桌面启动，常驻。跑 LocalSend 协议核心（LsCore），让同一 Wi-Fi 下的手机/电脑
 * 用 LocalSend 就能看到这台笔记本并互传文件，本机不需要装 LocalSend。
 * 收到的文件存进「下载/LocalSend」。
 */
public class SendService extends Service {
    static final String TAG = "Send";
    static final String FOLDER = "LocalSend";
    static volatile LsCore core;

    final Handler ui = new Handler(Looper.getMainLooper());
    WifiManager.MulticastLock lock;
    ConnectivityManager.NetworkCallback netCb;

    /** 任何地方都可以调用；失败只记日志，绝不抛出。 */
    static void start(Context c) {
        try { c.startService(new Intent(c, SendService.class)); }
        catch (Throwable e) { Log.w(TAG, "start: " + e); }
    }

    public IBinder onBind(Intent i) { return null; }
    public int onStartCommand(Intent i, int flags, int id) { return START_STICKY; }

    public void onCreate() {
        super.onCreate();
        try { L.init(this); } catch (Throwable ignored) {}
        try { boot(); } catch (Throwable e) { Log.w(TAG, "boot: " + e); }
        try { foreground(); } catch (Throwable e) { Log.w(TAG, "foreground: " + e); }
    }

    void boot() {
        final Context app = getApplicationContext();
        Send.app = app;
        SharedPreferences sp = Store.ctx(this).getSharedPreferences("send", 0);
        LsCore.Config cfg = new LsCore.Config();
        cfg.alias = sp.getString("alias", "Laptop");
        cfg.deviceModel = "Laptop";
        cfg.deviceType = "desktop";
        cfg.stateDir = new File(Store.ctx(this).getFilesDir(), "send");
        cfg.stateDir.mkdirs();
        LsCore c = new LsCore(cfg, new Saver(app), Send.LISTENER);
        core = c;
        c.start();

        // 不拿这把锁，Wi-Fi 芯片省电时会把组播包丢掉，别人就搜不到本机
        try {
            WifiManager wm = (WifiManager) app.getSystemService(Context.WIFI_SERVICE);
            lock = wm.createMulticastLock("open-desktop-send");
            lock.setReferenceCounted(false);
            lock.acquire();
        } catch (Throwable e) { Log.w(TAG, "multicast lock: " + e); }

        // Wi-Fi 断开、重连、换地址：让核心重新加入组播并再广播一次
        try {
            ConnectivityManager cm = (ConnectivityManager) getSystemService(CONNECTIVITY_SERVICE);
            netCb = new ConnectivityManager.NetworkCallback() {
                public void onAvailable(Network n) { kick(); }
                public void onLost(Network n) { kick(); }
                public void onLinkPropertiesChanged(Network n, LinkProperties lp) { kick(); }
            };
            cm.registerNetworkCallback(new NetworkRequest.Builder().build(), netCb);
        } catch (Throwable e) { Log.w(TAG, "network callback: " + e); }
    }
    final Runnable rejoin = new Runnable() { public void run() {
        new Thread(new Runnable() { public void run() {
            try { LsCore c = core; if (c != null) c.networkChanged(); } catch (Throwable e) { Log.w(TAG, "rejoin: " + e); }
        } }, "send-net").start();
    } };
    void kick() { ui.removeCallbacks(rejoin); ui.postDelayed(rejoin, 1500); }

    void foreground() {
        NotificationManager nm = (NotificationManager) getSystemService(NOTIFICATION_SERVICE);
        Notification.Builder b;
        if (Build.VERSION.SDK_INT >= 26) {
            nm.createNotificationChannel(new NotificationChannel("send", L.t("互传", "Передача файлов", "File transfer"), NotificationManager.IMPORTANCE_MIN));
            b = new Notification.Builder(this, "send");
        } else b = new Notification.Builder(this);
        b.setSmallIcon(android.R.drawable.stat_sys_upload_done)
            .setContentTitle(L.t("互传已开启", "Передача файлов включена", "File transfer is on"))
            .setContentText(L.t("附近的 LocalSend 设备可以向这台电脑发送文件", "Устройства с LocalSend поблизости могут отправлять сюда файлы", "Nearby LocalSend devices can send files to this computer"))
            .setOngoing(true);
        startForeground(53317, b.build());
    }

    public void onDestroy() {
        try { if (netCb != null) ((ConnectivityManager) getSystemService(CONNECTIVITY_SERVICE)).unregisterNetworkCallback(netCb); } catch (Throwable ignored) {}
        try { if (lock != null) lock.release(); } catch (Throwable ignored) {}
        final LsCore c = core;
        core = null;
        if (c != null) new Thread(new Runnable() { public void run() { try { c.stop(); } catch (Throwable ignored) {} } }).start();
        super.onDestroy();
    }

    /**
     * 存文件。安卓 10 起通过系统的「下载」媒体库写入 下载/LocalSend（不需要任何存储权限，重名自动加序号）；
     * 更老的系统或媒体库不可用时直接写文件夹。
     */
    static final class Saver implements LsCore.Storage {
        final Context c;
        Saver(Context c) { this.c = c; }

        public LsCore.Target create(String name, String mime, long size) throws IOException {
            if (Build.VERSION.SDK_INT >= 29) {
                try { return media(name); }
                catch (Throwable e) { Log.w(TAG, "media store: " + e); }
            }
            File dir = new File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), FOLDER);
            return LsCore.dirStorage(dir).create(name, mime, size);
        }

        LsCore.Target media(String name) throws IOException {
            final ContentResolver cr = c.getContentResolver();
            int slash = name.lastIndexOf('/');
            String file = slash < 0 ? name : name.substring(slash + 1);
            String rel = Environment.DIRECTORY_DOWNLOADS + "/" + FOLDER + (slash < 0 ? "" : "/" + name.substring(0, slash));
            ContentValues v = new ContentValues();
            v.put(MediaStore.MediaColumns.DISPLAY_NAME, file);
            v.put(MediaStore.MediaColumns.RELATIVE_PATH, rel);
            v.put(MediaStore.MediaColumns.IS_PENDING, 1);
            // 类型按扩展名定：若照搬对方声明的类型，系统会在名字后面再补一个扩展名
            int dot = file.lastIndexOf('.');
            String mt = dot < 0 ? null : MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.substring(dot + 1).toLowerCase(Locale.US));
            if (mt != null) v.put(MediaStore.MediaColumns.MIME_TYPE, mt);
            final Uri u = cr.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, v);
            if (u == null) throw new IOException("media store refused " + file);
            return new LsCore.Target() {
                public OutputStream out() throws IOException {
                    OutputStream o = cr.openOutputStream(u);
                    if (o == null) throw new IOException("cannot open " + u);
                    return o;
                }
                public String commit() {
                    ContentValues d = new ContentValues();
                    d.put(MediaStore.MediaColumns.IS_PENDING, 0);
                    cr.update(u, d, null, null);
                    return u.toString();
                }
                public void abort() { try { cr.delete(u, null, null); } catch (Throwable ignored) {} }
            };
        }
    }
}
