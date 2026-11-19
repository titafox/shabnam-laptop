package local.cloud.launcher;

import android.content.Context;
import android.net.wifi.ScanResult;
import android.net.wifi.WifiConfiguration;
import android.net.wifi.WifiManager;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import java.lang.reflect.Proxy;
import java.util.*;

/**
 * Wi-Fi 列表与直连。靠平台签名拿到的 NETWORK_SETTINGS：
 * 有了它，开关 Wi-Fi、扫描、读取已保存网络都不需要定位权限，也能调用系统设置用的 connect()。
 */
final class Wifi {
    static final String TAG = "Wifi";
    static final int OPEN = 0, PSK = 1, SAE = 2, OWE = 3, WEP = 4, EAP = 5;

    static final class Net {
        String ssid;
        int rssi = -127, sec, netId = -1;
        boolean connected;
        int bars() { return WifiManager.calculateSignalLevel(rssi, 5); }
        boolean secured() { return sec != OPEN && sec != OWE; }
    }
    interface Done { void done(boolean ok, boolean unsupported); }

    static WifiManager wm(Context c) { return (WifiManager) c.getApplicationContext().getSystemService(Context.WIFI_SERVICE); }

    static int security(String caps) {
        String s = caps == null ? "" : caps;
        if (s.contains("EAP")) return EAP;
        if (s.contains("PSK")) return PSK; // 含 WPA2/WPA3 过渡模式
        if (s.contains("SAE")) return SAE;
        if (s.contains("WEP")) return WEP;
        if (s.contains("OWE")) return OWE;
        return OPEN;
    }

    static boolean startScan(Context c) {
        try { return wm(c).startScan(); } catch (Throwable e) { Log.w(TAG, "scan: " + e); return false; }
    }

    /** 当前扫描结果：按名称去重，已连接的排最前，其余按信号强弱。 */
    static List<Net> list(Context c) {
        Map<String, Net> by = new LinkedHashMap<>();
        try {
            WifiManager w = wm(c);
            List<ScanResult> rs = w.getScanResults();
            if (rs != null) for (ScanResult r : rs) {
                String name = r.SSID;
                if (name == null || name.isEmpty()) continue;
                Net n = by.get(name);
                if (n == null) { n = new Net(); n.ssid = name; n.sec = security(r.capabilities); by.put(name, n); }
                if (r.level > n.rssi) n.rssi = r.level;
            }
            try {
                List<WifiConfiguration> saved = w.getConfiguredNetworks();
                if (saved != null) for (WifiConfiguration cf : saved) {
                    Net n = by.get(Tray.unquote(cf.SSID));
                    if (n != null && n.netId < 0) n.netId = cf.networkId;
                }
            } catch (Throwable e) { Log.w(TAG, "saved: " + e); }
            if (Tray.wifiConn && !Tray.ssid.isEmpty()) {
                Net n = by.get(Tray.ssid);
                if (n != null) n.connected = true;
            }
        } catch (Throwable e) { Log.w(TAG, "list: " + e); }
        List<Net> out = new ArrayList<>(by.values());
        Collections.sort(out, (a, b) -> a.connected != b.connected ? (a.connected ? -1 : 1) : b.rssi - a.rssi);
        return out;
    }

    static WifiConfiguration config(Net n, String pw) {
        WifiConfiguration cf = new WifiConfiguration();
        cf.SSID = "\"" + n.ssid + "\"";
        if (Build.VERSION.SDK_INT >= 30) {
            cf.setSecurityParams(n.sec == SAE ? WifiConfiguration.SECURITY_TYPE_SAE : n.sec == PSK ? WifiConfiguration.SECURITY_TYPE_PSK
                : n.sec == OWE ? WifiConfiguration.SECURITY_TYPE_OWE : WifiConfiguration.SECURITY_TYPE_OPEN);
        } else {
            cf.allowedKeyManagement.set(n.sec == PSK ? WifiConfiguration.KeyMgmt.WPA_PSK : WifiConfiguration.KeyMgmt.NONE);
        }
        if (pw != null && n.secured()) cf.preSharedKey = n.sec == PSK && pw.matches("[0-9A-Fa-f]{64}") ? pw : "\"" + pw + "\"";
        return cf;
    }

    /**
     * 连接：pw 为 null 表示用已保存的配置（或开放网络）。
     * 回调 ok 只代表系统接受了请求，真正连上要看之后的网络状态广播；
     * unsupported = 这台系统上两条直连路径都走不通，调用方应改开系统 Wi-Fi 面板。
     */
    static void connect(Context c, final Net n, String pw, final Done d) {
        final Handler ui = new Handler(Looper.getMainLooper());
        final WifiManager w = wm(c);
        final WifiConfiguration cf = pw == null && n.netId >= 0 ? null : config(n, pw);
        // 1) 系统设置用的隐藏接口 connect(config|netId, ActionListener)
        try {
            Class<?> lk = Class.forName("android.net.wifi.WifiManager$ActionListener");
            Object l = Proxy.newProxyInstance(lk.getClassLoader(), new Class<?>[]{lk}, (proxy, m, args) -> {
                String name = m.getName();
                if ("onSuccess".equals(name)) ui.post(() -> d.done(true, false));
                else if ("onFailure".equals(name)) { Log.w(TAG, "connect failed: " + (args == null ? "" : args[0])); ui.post(() -> d.done(false, false)); }
                else if ("hashCode".equals(name)) return System.identityHashCode(proxy);
                else if ("equals".equals(name)) return proxy == args[0];
                else if ("toString".equals(name)) return "OpenDesktopWifiListener";
                return null;
            });
            if (cf == null) WifiManager.class.getMethod("connect", int.class, lk).invoke(w, n.netId, l);
            else WifiManager.class.getMethod("connect", WifiConfiguration.class, lk).invoke(w, cf, l);
            return;
        } catch (Throwable e) { Log.w(TAG, "hidden connect: " + e + (e.getCause() != null ? " / " + e.getCause() : "")); }
        // 2) 旧的公开接口（对持有 NETWORK_SETTINGS 的应用仍然有效）
        try {
            int id = cf == null ? n.netId : w.addNetwork(cf);
            if (id >= 0 && w.enableNetwork(id, true)) { d.done(true, false); return; }
        } catch (Throwable e) { Log.w(TAG, "legacy connect: " + e); }
        d.done(false, true);
    }
}
