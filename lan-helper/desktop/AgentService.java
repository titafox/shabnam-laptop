package local.cloud.launcher;
import android.app.*;
import android.content.*;
import android.content.pm.*;
import android.graphics.*;
import android.hardware.display.*;
import android.media.Image;
import android.media.ImageReader;
import android.media.projection.*;
import android.net.Uri;
import android.os.*;
import android.provider.Settings;
import android.util.DisplayMetrics;
import android.view.WindowManager;
import org.json.*;
import java.io.*;
import java.net.*;
import java.util.Scanner;

public class AgentService extends Service {
    static final String BASE = "http://10.158.25.5:8000/";
    static final String PUBKEY = "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAqggtXHbevwwjzsuRYP+w1nvt3dPmRIOpR8hrq1v1MR/kaZeG/kB8vDkn1vemDa+YI4E03pNXH2j7GGvG5UNW1orcYdDtpYQDcSN0y4SjtxN31x7zmEVPZ71dT3PM9tdy//htLiWphqku5SEs8Z8LO2rKRVO/EXIeRgHYwjnUZNLulMRFLpNFk4v5kNPYO2KS0G9wc2wJIs2MsP0xpoBIuxlvNRifa9NXFja2fslcz+lnASW+sKaLMJ5K6xYeFZnMkNxvDROXAWND9MTdlxRhpbSiGECEyvqN0U26TSLQZ6aT76UW9r8xs1M4z8MY5wFnWEvVHZuDdcWAV9NF7b3slwIDAQAB";
    static volatile String status = "待命";
    static volatile boolean hasProjection = false;
    static volatile boolean wantProjection = false;

    volatile boolean running = false;
    String previous = "";
    MediaProjection projection;
    VirtualDisplay vdisplay;
    ImageReader reader;
    int sw, sh, sdpi;

    public IBinder onBind(Intent i) { return null; }

    public int onStartCommand(Intent intent, int flags, int startId) {
        startForeground(7, notif());
        if (projection == null && intent != null && intent.hasExtra("code")) {
            try {
                MediaProjectionManager mpm = (MediaProjectionManager) getSystemService(MEDIA_PROJECTION_SERVICE);
                Intent data = intent.getParcelableExtra("data");
                projection = mpm.getMediaProjection(intent.getIntExtra("code", Activity.RESULT_CANCELED), data);
                setupProjection();
                hasProjection = true; wantProjection = false;
            } catch (Exception e) { hasProjection = false; }
        }
        if (!running) { running = true; new Thread(this::loop).start(); }
        return START_STICKY;
    }
    Notification notif() {
        String ch = "agent";
        if (Build.VERSION.SDK_INT >= 26) {
            NotificationChannel c = new NotificationChannel(ch, "开放桌面控制", NotificationManager.IMPORTANCE_MIN);
            ((NotificationManager) getSystemService(NOTIFICATION_SERVICE)).createNotificationChannel(c);
        }
        Notification.Builder b = (Build.VERSION.SDK_INT >= 26) ? new Notification.Builder(this, ch) : new Notification.Builder(this);
        return b.setContentTitle("开放桌面：控制通道常驻中").setSmallIcon(android.R.drawable.ic_menu_manage).build();
    }

    // ---------- 轮询 ----------
    void loop() {
        while (running) {
            try { poll(); } catch (Exception e) { status = "连接 Mac 失败，重试中…"; }
            try { Thread.sleep(3000); } catch (InterruptedException e) { break; }
        }
    }
    HttpURLConnection connect(String path) throws IOException {
        HttpURLConnection c = (HttpURLConnection) new URL(BASE + path).openConnection();
        c.setConnectTimeout(2500); c.setReadTimeout(6000);
        c.setUseCaches(false); c.setInstanceFollowRedirects(false);
        return c;
    }
    boolean verified(String line) {
        try {
            int cut = line.lastIndexOf(' ');
            if (cut <= 0) return false;
            byte[] key = android.util.Base64.decode(PUBKEY, 0);
            java.security.Signature v = java.security.Signature.getInstance("SHA256withRSA");
            v.initVerify(java.security.KeyFactory.getInstance("RSA").generatePublic(new java.security.spec.X509EncodedKeySpec(key)));
            v.update(line.substring(0, cut).getBytes("UTF-8"));
            return v.verify(android.util.Base64.decode(line.substring(cut + 1), 0));
        } catch (Exception e) { return false; }
    }
    void poll() throws Exception {
        HttpURLConnection req = connect("diagnostic-action.txt");
        String line;
        try (Scanner s = new Scanner(req.getInputStream(), "UTF-8")) { line = s.nextLine().trim(); }
        finally { req.disconnect(); }
        if (line.equals(previous)) { status = "已连接 Mac ✓"; return; }
        if (!verified(line)) { previous = line; status = "拒绝未签名指令"; return; }
        previous = line;
        String payload = line.substring(0, line.lastIndexOf(' ')).trim();
        String[] head = payload.split(" +", 3);
        if (head.length < 2) return;
        String verb = head[1];
        String arg = head.length >= 3 ? head[2].trim() : "";
        status = "已连接 Mac ✓（执行 " + verb + "）";
        switch (verb) {
            case "install": report(payload, "install:start", null, null); install(line); return;
            case "list": report(payload, "list:ok", "packages", packagesJson()); return;
            case "inspect": report(payload, "inspect:ok", null, null); return;
            case "activities": report(payload, "activities:ok", "activities", activitiesJson(arg)); return;
            case "screenshot":
                if (!hasProjection) { report(payload, "screenshot:no-permission", null, null); return; }
                report(payload, "screenshot:start", null, null); capture(); return;
            case "shotsetup":
                wantProjection = true;
                startActivityNewTask(getPackageManager().getLaunchIntentForPackage(getPackageName()));
                report(payload, "shotsetup:requested", null, null); return;
            case "broadcast":
                try { sendBroadcast(new Intent(arg)); report(payload, "broadcast:ok", null, null); }
                catch (Exception e) { report(payload, "broadcast:err", null, null); }
                return;
            case "probe":
                try {
                    java.lang.Process p = new ProcessBuilder("sh", "-c", arg).redirectErrorStream(true).start();
                    ByteArrayOutputStream bo = new ByteArrayOutputStream();
                    byte[] buf = new byte[4096]; int n;
                    while (bo.size() < 65536 && (n = p.getInputStream().read(buf)) > 0) bo.write(buf, 0, n);
                    p.waitFor();
                    report(payload, "probe:ok", "stdout", new JSONArray().put(bo.toString("UTF-8").trim().replace("\n"," ")));
                } catch (Exception e) { report(payload, "probe:err " + e, null, null); }
                return;
        }
        // 需要拉起页面的命令
        Intent page = null;
        switch (verb) {
            case "usb-settings": page = new Intent("android.settings.USB_SETTINGS"); break;
            case "home": page = new Intent(Settings.ACTION_HOME_SETTINGS); break;
            case "open": page = new Intent(arg); break;
            case "url": page = new Intent(Intent.ACTION_VIEW, Uri.parse(arg)); break;
            case "uninstall": page = new Intent(Intent.ACTION_DELETE, Uri.parse("package:" + arg)); break;
            case "appinfo": page = new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:" + arg)); break;
            case "launch": page = getPackageManager().getLaunchIntentForPackage(arg); break;
            case "start":
                ComponentName t = ComponentName.unflattenFromString(arg);
                if (t != null) page = new Intent().setComponent(t);
                break;
        }
        boolean ok = page != null && startActivityNewTask(page);
        report(payload, (ok ? "dispatch:" : "failed:") + verb + (arg.isEmpty() ? "" : " " + arg), null, null);
    }
    boolean startActivityNewTask(Intent i) {
        if (i == null) return false;
        try { i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK); startActivity(i); return true; }
        catch (Exception e) { status = "拉起失败（可能缺悬浮窗权限）：" + e.getMessage(); return false; }
    }

    // ---------- 数据 ----------
    String readValue(InputStream in) throws IOException {
        byte[] buf = new byte[1024]; int n;
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        while (out.size() < 1024 && (n = in.read(buf, 0, 1024 - out.size())) > 0) out.write(buf, 0, n);
        return out.toString("UTF-8").replace((char) 0, ' ').trim();
    }
    JSONObject snapshot() throws Exception {
        JSONObject d = new JSONObject();
        d.put("type", "device_diagnosis").put("model", Build.MODEL).put("board", Build.BOARD)
         .put("android", Build.VERSION.RELEASE).put("probe_version", 12)
         .put("fingerprint", Build.FINGERPRINT)
         .put("overlay", Settings.canDrawOverlays(this)).put("projection", hasProjection);
        for (String key : new String[]{"adb_enabled", "adb_wifi_enabled", "development_settings_enabled"}) {
            try { d.put(key, Settings.Global.getString(getContentResolver(), key)); }
            catch (SecurityException e) { d.put(key, "restricted"); }
        }
        JSONObject props = new JSONObject();
        for (String key : new String[]{"sys.usb.config", "sys.usb.controller", "init.svc.adbd", "service.adb.tcp.port", "ro.boot.flash.locked"}) {
            try {
                java.lang.Process p = new ProcessBuilder("/system/bin/getprop", key).start();
                try (InputStream in = p.getInputStream()) { props.put(key, readValue(in)); } finally { p.destroy(); }
            } catch (Exception e) { props.put(key, "unavailable"); }
        }
        d.put("properties", props);
        return d;
    }
    JSONArray packagesJson() {
        JSONArray arr = new JSONArray();
        PackageManager pm = getPackageManager();
        for (PackageInfo pi : pm.getInstalledPackages(0)) {
            try {
                ApplicationInfo ai = pi.applicationInfo;
                arr.put(new JSONObject().put("pkg", pi.packageName).put("label", String.valueOf(pm.getApplicationLabel(ai)))
                    .put("version", pi.versionName).put("system", (ai.flags & ApplicationInfo.FLAG_SYSTEM) != 0)
                    .put("enabled", ai.enabled).put("launchable", pm.getLaunchIntentForPackage(pi.packageName) != null));
            } catch (Exception ignored) {}
        }
        return arr;
    }
    JSONArray activitiesJson(String pkg) {
        JSONArray arr = new JSONArray();
        try {
            PackageInfo pi = getPackageManager().getPackageInfo(pkg, PackageManager.GET_ACTIVITIES | PackageManager.MATCH_DISABLED_COMPONENTS);
            if (pi.activities != null) for (ActivityInfo a : pi.activities)
                arr.put(new JSONObject().put("name", a.name).put("exported", a.exported).put("enabled", a.enabled));
        } catch (Exception e) {
            try { arr.put(new JSONObject().put("error", String.valueOf(e))); } catch (Exception ignored) {}
        }
        return arr;
    }
    void report(String request, String exec, String key, JSONArray data_) {
        try {
            JSONObject d = snapshot().put("request", request).put("exec", exec);
            if (data_ != null && key != null) d.put(key, data_);
            HttpURLConnection post = connect("log");
            try {
                post.setRequestMethod("POST"); post.setDoOutput(true);
                post.setRequestProperty("Content-Type", "application/json");
                try (OutputStream o = post.getOutputStream()) { o.write(d.toString().getBytes("UTF-8")); }
                post.getResponseCode();
            } finally { post.disconnect(); }
        } catch (Exception ignored) {}
    }
    void install(String line) throws Exception {
        String[] w = line.split(" ");
        if (w.length != 4 || !w[2].matches("[0-9a-f]{64}")) return;
        SharedPreferences sp = getSharedPreferences("agent", MODE_PRIVATE);
        if (line.equals(sp.getString("delivered", ""))) return;
        if (!getPackageManager().canRequestPackageInstalls()) {
            startActivityNewTask(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:" + getPackageName())));
            return;
        }
        String name = w[2] + ".apk";
        File tmp = File.createTempFile("pkg", ".tmp", getCacheDir());
        HttpURLConnection c = connect(name);
        try {
            java.security.MessageDigest sha = java.security.MessageDigest.getInstance("SHA-256");
            try (InputStream in = c.getInputStream(); OutputStream out = new FileOutputStream(tmp)) {
                byte[] buf = new byte[32768]; int n; long size = 0;
                while ((n = in.read(buf)) != -1) {
                    if ((size += n) > 268435456L) throw new IOException("下载过大");
                    sha.update(buf, 0, n); out.write(buf, 0, n);
                    if ((size & ~0xFFFFF) != 0) status = "接收安装包 " + (size >> 20) + " MB…";
                }
            }
            StringBuilder hex = new StringBuilder();
            for (byte b : sha.digest()) hex.append(String.format("%02x", b & 255));
            if (!w[2].contentEquals(hex)) throw new IOException("校验失败");
            if (!tmp.renameTo(new File(getCacheDir(), name))) throw new IOException("保存失败");
        } finally { c.disconnect(); tmp.delete(); }
        Intent view = new Intent(Intent.ACTION_VIEW)
            .setDataAndType(Uri.parse("content://local.cloud.launcher.apk/" + name), "application/vnd.android.package-archive")
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        if (startActivityNewTask(view)) sp.edit().putString("delivered", line).apply();
    }

    // ---------- 截屏 ----------
    void setupProjection() {
        DisplayMetrics m = new DisplayMetrics();
        ((WindowManager) getSystemService(WINDOW_SERVICE)).getDefaultDisplay().getRealMetrics(m);
        sw = m.widthPixels; sh = m.heightPixels; sdpi = m.densityDpi;
        reader = ImageReader.newInstance(sw, sh, PixelFormat.RGBA_8888, 2);
        vdisplay = projection.createVirtualDisplay("shot", sw, sh, sdpi,
            DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, reader.getSurface(), null, null);
    }
    void capture() {
        try {
            Image img = null;
            for (int i = 0; i < 12 && img == null; i++) { img = reader.acquireLatestImage(); if (img == null) Thread.sleep(80); }
            if (img == null) { status = "截屏：暂无帧"; return; }
            Image.Plane p = img.getPlanes()[0];
            int rw = p.getRowStride() / p.getPixelStride();
            Bitmap full = Bitmap.createBitmap(rw, sh, Bitmap.Config.ARGB_8888);
            full.copyPixelsFromBuffer(p.getBuffer());
            img.close();
            Bitmap out = (rw != sw) ? Bitmap.createBitmap(full, 0, 0, sw, sh) : full;
            ByteArrayOutputStream bos = new ByteArrayOutputStream();
            out.compress(Bitmap.CompressFormat.PNG, 100, bos);
            HttpURLConnection c = connect("shot");
            try {
                c.setRequestMethod("POST"); c.setDoOutput(true);
                c.setRequestProperty("Content-Type", "image/png");
                c.setFixedLengthStreamingMode(bos.size());
                try (OutputStream o = c.getOutputStream()) { bos.writeTo(o); }
                c.getResponseCode();
            } finally { c.disconnect(); }
            status = "截屏已上传 Mac";
        } catch (Exception e) { status = "截屏失败：" + e.getMessage(); }
    }
    public void onDestroy() {
        running = false; hasProjection = false;
        if (vdisplay != null) vdisplay.release();
        if (projection != null) projection.stop();
        super.onDestroy();
    }
}
