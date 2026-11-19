package local.cloud.launcher;

import java.io.*;
import java.net.*;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.util.*;
import java.util.concurrent.*;
import javax.net.ssl.*;

/**
 * LocalSend 协议 v2 的核心（发现 + 收 + 发），纯 Java，不含任何安卓接口，可直接在电脑的 JVM 上测试。
 * 协议原文：github.com/localsend/protocol 的 README（v2.2）。
 *
 * 发现：向 224.0.0.167:53317 组播自己的信息；听到别人 announce=true 就向对方 POST /register，
 *       失败再退回组播应答；另有「逐个地址 POST /register」的扫描方式（组播不通时用）。
 * 接收：本机开 HTTP(S) 服务，/prepare-upload → 等用户决定 → /upload 逐个文件 → /cancel。
 * 发送：对方是服务端，本机作为客户端走同样的三个接口。
 */
public final class LsCore {
    public static final int PORT = 53317;
    public static final String GROUP = "224.0.0.167";
    static final String API = "/api/localsend/v2/";

    public static final class Config {
        public String alias = "Laptop";
        public String deviceModel = "Laptop";
        public String deviceType = "desktop";
        public int port = PORT;
        public int multicastPort = PORT;
        /** 保存本机身份（钥匙、证书）的目录。 */
        public File stateDir;
        /** true = HTTPS（LocalSend 的默认），建不起来时自动退回 HTTP。 */
        public boolean https = true;
        /** 发送前先算好 sha256 一并告诉对方（对方据此校验）。大文件会多读一遍，默认关。 */
        public boolean hashOutgoing;
        /** 仅测试用：只在这一个网络接口上组播（例如 lo0）。 */
        public String onlyInterface;
        /** 仅测试用：只与这些地址通信，其余一律忽略。 */
        public String[] onlyHosts;
    }

    public static final class Device {
        public String alias, ip, fingerprint, model, type;
        public int port;
        public boolean https;
        public long seen;
        public String toString() { return alias + "@" + (https ? "https" : "http") + "://" + ip + ":" + port; }
    }

    public static final class FileMeta {
        public String id, name, type, sha256, preview;
        public long size;
    }

    /** 收到的文件存到哪里，由外面决定（安卓上是下载目录，测试里是普通文件夹）。 */
    public interface Storage { Target create(String name, String mime, long size) throws IOException; }
    public interface Target {
        OutputStream out() throws IOException;
        /** 写完后调用；返回一个能再次找到该文件的引用（路径或 content 地址）。 */
        String commit() throws IOException;
        void abort();
    }
    /** 要发出去的一个文件。sha256 可为 null。 */
    public interface Source {
        String name();
        long size();
        String mime();
        InputStream open() throws IOException;
    }
    public interface Listener {
        void onDevices();
        /** 有人想发文件过来：调用 s.accept() 或 s.decline()。 */
        void onIncoming(Incoming s);
        void onIncomingUpdate(Incoming s);
        /** 对方发来一段文字（LocalSend 把文字当作带 preview 的文本文件）。 */
        void onText(Device from, String text);
        void onOutgoingUpdate(Outgoing o);
        void log(String s);
    }

    public final Config cfg;
    public final Storage storage;
    public final Listener listener;
    public volatile String fingerprint = "";
    public volatile boolean https;
    public volatile int port;
    public volatile boolean running;

    private final Map<String, Device> devices = new LinkedHashMap<>();
    private final ExecutorService pool;
    private final ScheduledExecutorService timer;
    private final SecureRandom rnd = new SecureRandom();
    private volatile ServerSocket server;
    private volatile MulticastSocket msock;
    private volatile Incoming incoming;
    private final Set<Outgoing> outgoing = Collections.synchronizedSet(new HashSet<Outgoing>());
    private final Map<String, SSLSocketFactory> pins = new HashMap<>();
    private volatile boolean scanning;
    private String lastMulticastError;

    public LsCore(Config cfg, Storage storage, Listener listener) {
        this.cfg = cfg; this.storage = storage; this.listener = listener;
        ThreadFactory tf = new ThreadFactory() {
            public Thread newThread(Runnable r) { Thread t = new Thread(r, "localsend"); t.setDaemon(true); return t; }
        };
        pool = Executors.newCachedThreadPool(tf);
        timer = Executors.newSingleThreadScheduledExecutor(tf);
    }

    // ================= 启停 =================
    public synchronized void start() {
        if (running) return;
        running = true;
        pool.execute(new Runnable() { public void run() { boot(); } });
    }
    public synchronized void stop() {
        running = false;
        close(server); close(msock);
        Incoming in = incoming;
        if (in != null) in.cancel();
        synchronized (outgoing) { for (Outgoing o : new ArrayList<>(outgoing)) o.cancel(); }
        timer.shutdownNow();
        pool.shutdown();
    }
    private void boot() {
        SSLContext tls = null;
        if (cfg.https) {
            try {
                LsCert id = LsCert.loadOrCreate(cfg.stateDir == null ? null : new File(cfg.stateDir, "identity"));
                tls = id.serverContext();
                fingerprint = id.fingerprint;
            } catch (Throwable e) { log("https unavailable, falling back to http: " + e); tls = null; }
        }
        https = tls != null;
        if (!https) fingerprint = randomFingerprint();
        final SSLContext ctx = tls;
        pool.execute(new Runnable() { public void run() { acceptLoop(ctx); } });
        pool.execute(new Runnable() { public void run() { multicastLoop(); } });
        timer.scheduleWithFixedDelay(new Runnable() { public void run() { try { watchdog(); } catch (Throwable ignored) {} } }, 5, 5, TimeUnit.SECONDS);
    }
    /** HTTP 模式下指纹是一串随机字符，存下来保持不变。 */
    private String randomFingerprint() {
        File f = cfg.stateDir == null ? null : new File(cfg.stateDir, "fingerprint");
        try {
            if (f != null && f.isFile()) {
                try (BufferedReader r = new BufferedReader(new FileReader(f))) { String s = r.readLine(); if (s != null && s.length() >= 16) return s.trim(); }
            }
        } catch (IOException ignored) {}
        String s = token() + token();
        try { if (f != null) try (FileWriter w = new FileWriter(f)) { w.write(s); } } catch (IOException ignored) {}
        return s;
    }
    String token() { byte[] b = new byte[16]; rnd.nextBytes(b); return LsCert.hex(b); }
    void log(String s) { try { listener.log(s); } catch (Throwable ignored) {} }
    static void close(Closeable c) { try { if (c != null) c.close(); } catch (Throwable ignored) {} }
    private boolean allowed(String ip) {
        if (cfg.onlyHosts == null) return true;
        for (String h : cfg.onlyHosts) if (h.equals(ip)) return true;
        return false;
    }

    // ================= 设备表 =================
    public List<Device> devices() { synchronized (devices) { return new ArrayList<>(devices.values()); } }
    Map<String, Object> info(boolean withPort) {
        Map<String, Object> m = LsJson.obj("alias", cfg.alias, "version", "2.0", "deviceModel", cfg.deviceModel,
            "deviceType", cfg.deviceType, "fingerprint", fingerprint);
        if (withPort) { m.put("port", port); m.put("protocol", https ? "https" : "http"); }
        m.put("download", Boolean.FALSE);
        return m;
    }
    /** 从对方给的信息登记一台设备。port/protocol 缺失时用 defPort/defHttps。返回 null 表示是自己或内容不对。 */
    Device learn(Map<String, Object> m, String ip, int defPort, boolean defHttps) {
        String fp = LsJson.s(m, "fingerprint"), alias = LsJson.s(m, "alias");
        if (fp == null || fp.isEmpty() || alias == null || fp.equals(fingerprint) || !allowed(ip)) return null;
        Device d = new Device();
        d.alias = alias.length() > 60 ? alias.substring(0, 60) : alias;
        d.ip = ip; d.fingerprint = fp;
        d.model = LsJson.s(m, "deviceModel"); d.type = LsJson.s(m, "deviceType");
        d.port = (int) LsJson.n(m, "port", defPort);
        String proto = LsJson.s(m, "protocol");
        d.https = proto == null ? defHttps : "https".equals(proto);
        d.seen = System.currentTimeMillis();
        boolean changed;
        synchronized (devices) {
            Device old = devices.put(fp, d);
            changed = old == null || !old.ip.equals(d.ip) || old.port != d.port || old.https != d.https || !old.alias.equals(d.alias);
        }
        if (changed) { log("device: " + d); try { listener.onDevices(); } catch (Throwable ignored) {} }
        return d;
    }
    /** 清空设备表并重新喊一遍（对方应答后会重新出现），同时做一次逐地址扫描。 */
    public void refresh(boolean scan) {
        synchronized (devices) { devices.clear(); }
        try { listener.onDevices(); } catch (Throwable ignored) {}
        announce();
        if (scan) scan();
    }
    /** 网络变了（Wi-Fi 断开、重连、换了地址）：重新加入组播并再喊一遍。 */
    public void networkChanged() {
        close(msock);
        announce();
    }

    // ================= 组播 =================
    List<NetworkInterface> interfaces() {
        List<NetworkInterface> out = new ArrayList<>();
        try {
            Enumeration<NetworkInterface> en = NetworkInterface.getNetworkInterfaces();
            while (en != null && en.hasMoreElements()) {
                NetworkInterface n = en.nextElement();
                try {
                    if (!n.isUp()) continue;
                    if (cfg.onlyInterface != null) { if (!cfg.onlyInterface.equals(n.getName())) continue; }
                    else if (n.isLoopback() || n.isPointToPoint() || !n.supportsMulticast()) continue;
                    boolean v4 = false;
                    for (InterfaceAddress a : n.getInterfaceAddresses()) if (a.getAddress() instanceof Inet4Address) v4 = true;
                    if (v4) out.add(n);
                } catch (SocketException ignored) {}
            }
        } catch (SocketException ignored) {}
        return out;
    }
    /** 本机在局域网里的 IPv4 地址。 */
    public List<String> localIps() {
        List<String> out = new ArrayList<>();
        for (NetworkInterface n : interfaces())
            for (InterfaceAddress a : n.getInterfaceAddresses())
                if (a.getAddress() instanceof Inet4Address) out.add(a.getAddress().getHostAddress());
        return out;
    }
    private String netSignature() {
        StringBuilder b = new StringBuilder();
        for (NetworkInterface n : interfaces()) b.append(n.getName()).append(n.getInterfaceAddresses()).append(';');
        return b.toString();
    }
    private void multicastLoop() {
        byte[] buf = new byte[8192];
        while (running) {
            MulticastSocket s = null;
            try {
                String sig = netSignature();
                List<NetworkInterface> nis = interfaces();
                if (nis.isEmpty()) { Thread.sleep(3000); continue; }
                s = new MulticastSocket(cfg.multicastPort);
                InetSocketAddress g = new InetSocketAddress(InetAddress.getByName(GROUP), cfg.multicastPort);
                int joined = 0;
                for (NetworkInterface n : nis) { try { s.joinGroup(g, n); joined++; } catch (IOException e) { log("join " + n.getName() + ": " + e); } }
                if (joined == 0) throw new IOException("no interface joined");
                s.setSoTimeout(15000);
                msock = s;
                log("multicast listening on " + joined + " interface(s)");
                announce();
                while (running && !s.isClosed()) {
                    DatagramPacket p = new DatagramPacket(buf, buf.length);
                    try { s.receive(p); }
                    catch (SocketTimeoutException e) { if (!sig.equals(netSignature())) break; continue; }
                    try { onDatagram(new String(p.getData(), 0, p.getLength(), StandardCharsets.UTF_8), p.getAddress().getHostAddress()); }
                    catch (Throwable ignored) {}
                }
            } catch (Throwable e) {
                // 常见原因：端口被同机的另一个 LocalSend 独占。隔一阵再试，日志只记一次。
                if (running && (s == null || !s.isClosed())) {
                    if (!String.valueOf(e).equals(lastMulticastError)) log("multicast: " + e);
                    lastMulticastError = String.valueOf(e);
                    try { Thread.sleep(6000); } catch (InterruptedException x) { return; }
                }
            } finally { close(s); }
            try { Thread.sleep(1500); } catch (InterruptedException e) { return; }
        }
    }
    private void onDatagram(String text, final String ip) throws IOException {
        Map<String, Object> m = LsJson.parseObject(text);
        final Device d = learn(m, ip, PORT, true);
        if (d == null || !Boolean.TRUE.equals(m.get("announce"))) return;
        // 协议：先用 HTTP 向对方登记自己，不通再退回组播应答
        pool.execute(new Runnable() { public void run() {
            try {
                Http h = request(d, "register", LsJson.str(info(true)), null, 5000);
                if (h.code == 200) return;
            } catch (Throwable ignored) {}
            sendMulticast(false);
        } });
    }
    /** 像 LocalSend 一样连喊三次，间隔拉开，避免刚连上网时丢包。 */
    public void announce() {
        if (!running || timer.isShutdown()) return;
        for (long ms : new long[]{100, 500, 2000})
            try { timer.schedule(new Runnable() { public void run() { sendMulticast(true); } }, ms, TimeUnit.MILLISECONDS); }
            catch (RejectedExecutionException ignored) {}
    }
    private void sendMulticast(boolean announce) {
        if (!running || port == 0) return;
        Map<String, Object> m = info(true);
        m.put("announce", announce);
        byte[] b = LsJson.str(m).getBytes(StandardCharsets.UTF_8);
        for (NetworkInterface n : interfaces()) {
            try (MulticastSocket s = new MulticastSocket()) {
                s.setNetworkInterface(n);
                s.setTimeToLive(1);
                s.send(new DatagramPacket(b, b.length, InetAddress.getByName(GROUP), cfg.multicastPort));
            } catch (IOException ignored) {}
        }
    }

    /** 协议 3.2：组播不通时，向本网段每个地址 POST /register。 */
    public void scan() {
        if (scanning || !running) return;
        scanning = true;
        final List<String> targets = new ArrayList<>();
        final Set<String> mine = new HashSet<>(localIps());
        for (String ip : mine) {
            String base = ip.substring(0, ip.lastIndexOf('.') + 1);
            for (int i = 1; i < 255; i++) { String t = base + i; if (!mine.contains(t) && allowed(t)) targets.add(t); }
        }
        final ExecutorService ex = Executors.newFixedThreadPool(48);
        for (final String ip : targets) ex.execute(new Runnable() { public void run() { probe(ip, PORT); } });
        ex.shutdown();
        pool.execute(new Runnable() { public void run() {
            try { ex.awaitTermination(30, TimeUnit.SECONDS); } catch (InterruptedException ignored) {}
            scanning = false;
        } });
    }
    /** 直接问某个地址是不是 LocalSend 设备（先 HTTPS，握手不成再 HTTP）。 */
    public Device probe(String ip, int p) {
        if (!allowed(ip)) return null;
        try (Socket s = new Socket()) { s.connect(new InetSocketAddress(ip, p), 700); }
        catch (IOException e) { return null; }
        for (int k = 0; k < 2; k++) {
            Device d = new Device();
            d.ip = ip; d.port = p; d.https = k == 0;
            try {
                Http h = request(d, "register", LsJson.str(info(true)), null, 4000);
                if (h.code == 200) return learn(LsJson.parseObject(h.body), ip, p, d.https);
            } catch (Throwable ignored) {}
        }
        return null;
    }

    // ================= HTTP 客户端 =================
    static final class Http { int code; String body = ""; }
    private HttpURLConnection open(Device d, String pathQuery, int readTimeout) throws IOException {
        if (!allowed(d.ip)) throw new IOException("host not allowed");
        URL u = new URL(d.https ? "https" : "http", d.ip, d.port, API + pathQuery);
        HttpURLConnection c = (HttpURLConnection) u.openConnection(Proxy.NO_PROXY);
        if (c instanceof HttpsURLConnection) {
            HttpsURLConnection hs = (HttpsURLConnection) c;
            try {
                SSLSocketFactory f;
                String key = String.valueOf(pinFor(d));
                synchronized (pins) { f = pins.get(key); if (f == null) { f = LsCert.pinned(pinFor(d)); pins.put(key, f); } }
                hs.setSSLSocketFactory(f);
            } catch (java.security.GeneralSecurityException e) { throw new IOException(e); }
            hs.setHostnameVerifier(new HostnameVerifier() { public boolean verify(String h, SSLSession s) { return true; } });
        }
        c.setConnectTimeout(5000);
        c.setReadTimeout(readTimeout);
        c.setUseCaches(false);
        return c;
    }
    /** HTTPS 设备的指纹就是它证书的 SHA-256（64 位十六进制），据此钉住证书；其它形式的指纹没法校验。 */
    private static String pinFor(Device d) {
        String f = d.fingerprint;
        return d.https && f != null && f.matches("[0-9a-fA-F]{64}") ? f : null;
    }
    private Http request(Device d, String pathQuery, String json, Outgoing o, int readTimeout) throws IOException {
        HttpURLConnection c = open(d, pathQuery, readTimeout);
        if (o != null) o.conn = c;
        try {
            byte[] b = json == null ? new byte[0] : json.getBytes(StandardCharsets.UTF_8);
            c.setRequestMethod("POST");
            c.setDoOutput(true);
            c.setFixedLengthStreamingMode(b.length);
            if (json != null) c.setRequestProperty("Content-Type", "application/json");
            try (OutputStream out = c.getOutputStream()) { out.write(b); }
            return finish(c);
        } finally { c.disconnect(); }
    }
    private static Http finish(HttpURLConnection c) throws IOException {
        Http h = new Http();
        h.code = c.getResponseCode();
        InputStream in = h.code >= 400 ? c.getErrorStream() : c.getInputStream();
        if (in != null) try {
            ByteArrayOutputStream bo = new ByteArrayOutputStream();
            byte[] buf = new byte[4096];
            int n;
            while ((n = in.read(buf)) > 0 && bo.size() < (1 << 20)) bo.write(buf, 0, n);
            h.body = new String(bo.toByteArray(), StandardCharsets.UTF_8);
        } finally { close(in); }
        return h;
    }

    // ================= 发送 =================
    public static final class Outgoing {
        public static final int WAITING = 0, SENDING = 1, DONE = 2, DECLINED = 3, BUSY = 4, PIN = 5, CANCELLED = 6, FAILED = 7;
        public final Device to;
        public final List<Source> files;
        public final String text;
        public long total;
        public volatile long done;
        public volatile int state = WAITING, sent, skipped;
        public volatile String error, current;
        volatile HttpURLConnection conn;
        volatile boolean cancelled;
        volatile String sessionId;
        final LsCore core;
        long lastNotify;
        Outgoing(LsCore core, Device to, List<Source> files, String text) { this.core = core; this.to = to; this.files = files; this.text = text; }
        public boolean active() { return state == WAITING || state == SENDING; }
        /** 用户取消：断开正在进行的请求，并通知对方。 */
        public void cancel() {
            if (!active()) return;
            cancelled = true;
            state = CANCELLED;
            final HttpURLConnection c = conn;
            final String sid = sessionId;
            try { core.pool.execute(new Runnable() { public void run() {
                try { if (c != null) c.disconnect(); } catch (Throwable ignored) {}
                try { core.request(to, "cancel" + (sid == null ? "" : "?sessionId=" + enc(sid)), null, null, 3000); } catch (Throwable ignored) {}
            } }); } catch (RejectedExecutionException ignored) {}
            core.outgoing.remove(this);
            core.notify(this);
        }
    }
    void notify(Outgoing o) { try { listener.onOutgoingUpdate(o); } catch (Throwable ignored) {} }
    static String enc(String s) { try { return URLEncoder.encode(s, "UTF-8"); } catch (UnsupportedEncodingException e) { return s; } }

    /** 发文件。pin 通常为 null；对方要求口令（状态 PIN）时带上再发一次。 */
    public Outgoing send(Device to, List<Source> files, String pin) { return start(new Outgoing(this, to, new ArrayList<>(files), null), pin); }
    /** 发一段文字（对方的 LocalSend 会直接显示出来并可复制）。 */
    public Outgoing sendText(Device to, String text, String pin) { return start(new Outgoing(this, to, new ArrayList<Source>(), text), pin); }
    private Outgoing start(final Outgoing o, final String pin) {
        outgoing.add(o);
        pool.execute(new Runnable() { public void run() {
            try { transmit(o, pin); }
            catch (Throwable e) {
                // 对方取消时连接会先断、/cancel 后到：稍等一下，免得把「对方取消」误报成「失败」
                if (o.active()) try { Thread.sleep(500); } catch (InterruptedException ignored) {}
                if (o.active()) { o.state = Outgoing.FAILED; o.error = String.valueOf(e.getMessage() == null ? e : e.getMessage()); }
            }
            outgoing.remove(o);
            LsCore.this.notify(o);
        } });
        return o;
    }
    private void transmit(Outgoing o, String pin) throws IOException {
        Map<String, Object> files = new LinkedHashMap<>();
        final Map<String, Source> byId = new LinkedHashMap<>();
        final byte[] textBytes = o.text == null ? null : o.text.getBytes(StandardCharsets.UTF_8);
        if (textBytes != null) {
            String id = UUID.randomUUID().toString();
            files.put(id, LsJson.obj("id", id, "fileName", id + ".txt", "size", (long) textBytes.length, "fileType", "text/plain", "preview", o.text));
            byId.put(id, new Source() {
                public String name() { return "message.txt"; }
                public long size() { return textBytes.length; }
                public String mime() { return "text/plain"; }
                public InputStream open() { return new ByteArrayInputStream(textBytes); }
            });
            o.total = textBytes.length;
        }
        for (Source s : o.files) {
            String id = UUID.randomUUID().toString();
            long size = s.size();
            Map<String, Object> f = LsJson.obj("id", id, "fileName", s.name(), "size", size,
                "fileType", s.mime() == null ? "application/octet-stream" : s.mime());
            if (cfg.hashOutgoing) f.put("sha256", sha256(s));
            files.put(id, f);
            byId.put(id, s);
            o.total += size;
            if (o.cancelled) return;
        }
        notify(o);
        String body = LsJson.str(LsJson.obj("info", info(true), "files", files));
        // 对方要等人点「接受」，所以这一步等得久
        Http h = request(o.to, "prepare-upload" + (pin == null ? "" : "?pin=" + enc(pin)), body, o, 10 * 60 * 1000);
        if (o.cancelled) return;
        switch (h.code) {
            case 200: break;
            case 204: o.done = o.total; o.sent = byId.size(); o.state = Outgoing.DONE; return;
            case 401: o.state = Outgoing.PIN; return;
            case 403: o.state = Outgoing.DECLINED; return;
            case 409: o.state = Outgoing.BUSY; return;
            default: o.state = Outgoing.FAILED; o.error = "HTTP " + h.code; return;
        }
        Map<String, Object> r = LsJson.parseObject(h.body);
        o.sessionId = LsJson.s(r, "sessionId");
        Map<String, Object> tokens = LsJson.o(r, "files");
        if (o.sessionId == null || tokens == null) throw new IOException("bad prepare-upload response");
        o.state = Outgoing.SENDING;
        notify(o);
        int failed = 0;
        for (Map.Entry<String, Source> e : byId.entrySet()) {
            if (o.cancelled) return;
            String tk = LsJson.s(tokens, e.getKey());
            Source s = e.getValue();
            if (tk == null) { o.skipped++; o.total -= s.size(); continue; } // 对方没要这个文件
            o.current = s.name();
            int code = upload(o, e.getKey(), tk, s);
            if (o.cancelled) return;
            if (code == 200) o.sent++;
            else { failed++; o.error = "HTTP " + code; if (code == 403 || code == 409) break; }
        }
        if (o.cancelled) return;
        o.state = failed == 0 ? Outgoing.DONE : Outgoing.FAILED;
    }
    private int upload(Outgoing o, String fileId, String token, Source s) throws IOException {
        HttpURLConnection c = open(o.to, "upload?sessionId=" + enc(o.sessionId) + "&fileId=" + enc(fileId) + "&token=" + enc(token), 60000);
        o.conn = c;
        try {
            c.setRequestMethod("POST");
            c.setDoOutput(true);
            c.setFixedLengthStreamingMode(s.size());
            c.setRequestProperty("Content-Type", s.mime() == null ? "application/octet-stream" : s.mime());
            try (InputStream in = s.open(); OutputStream out = c.getOutputStream()) {
                byte[] buf = new byte[65536];
                int n;
                while ((n = in.read(buf)) > 0) {
                    if (o.cancelled) throw new IOException("cancelled");
                    out.write(buf, 0, n);
                    o.done += n;
                    long now = System.currentTimeMillis();
                    if (now - o.lastNotify > 200) { o.lastNotify = now; notify(o); }
                }
            }
            return finish(c).code;
        } finally { c.disconnect(); }
    }
    static String sha256(Source s) throws IOException {
        try (InputStream in = s.open()) {
            MessageDigest md = MessageDigest.getInstance("SHA-256");
            byte[] buf = new byte[65536];
            int n;
            while ((n = in.read(buf)) > 0) md.update(buf, 0, n);
            return LsCert.hex(md.digest());
        } catch (java.security.NoSuchAlgorithmException e) { throw new IOException(e); }
    }

    // ================= 接收 =================
    public static final class Incoming {
        public static final int WAITING = 0, RECEIVING = 1, DONE = 2, DECLINED = 3, CANCELLED = 4, FAILED = 5;
        public final String sessionId;
        public final Device sender;
        public final List<FileMeta> files = new ArrayList<>();
        public long total;
        public volatile long done;
        public volatile int state = WAITING, failed;
        public volatile String error;
        /** 已保存文件的引用（Storage.commit 的返回值）。 */
        public final List<String> saved = Collections.synchronizedList(new ArrayList<String>());
        final Map<String, String> tokens = new HashMap<>();
        final Set<String> started = new HashSet<>(), finished = new HashSet<>();
        final Set<Socket> sockets = Collections.synchronizedSet(new HashSet<Socket>());
        final CountDownLatch decided = new CountDownLatch(1);
        final LsCore core;
        volatile long touched = System.currentTimeMillis();
        long lastNotify;
        Incoming(LsCore core, Device sender) { this.core = core; this.sender = sender; sessionId = core.token(); }
        public boolean active() { return state == WAITING || state == RECEIVING; }
        public FileMeta file(String id) { for (FileMeta f : files) if (f.id.equals(id)) return f; return null; }
        public void accept() {
            synchronized (this) {
                if (state != WAITING) return;
                for (FileMeta f : files) tokens.put(f.id, core.token());
                state = RECEIVING;
                touched = System.currentTimeMillis();
            }
            decided.countDown();
            core.notify(this);
        }
        public void decline() { end(DECLINED, null, false); }
        /** 本机用户中途取消：断开正在传的连接，并尽力通知对方。 */
        public void cancel() { end(CANCELLED, null, true); }
        void end(int st, String err, boolean tellPeer) {
            synchronized (this) {
                if (!active()) return;
                state = st; error = err;
            }
            decided.countDown();
            synchronized (sockets) { for (Socket s : sockets) close(s); }
            core.release(this);
            if (tellPeer) try { core.pool.execute(new Runnable() { public void run() {
                try { core.request(sender, "cancel?sessionId=" + enc(sessionId), null, null, 3000); } catch (Throwable ignored) {}
            } }); } catch (RejectedExecutionException ignored) {}
            core.notify(this);
        }
    }
    void notify(Incoming s) { try { listener.onIncomingUpdate(s); } catch (Throwable ignored) {} }
    synchronized void release(Incoming s) { if (incoming == s) incoming = null; }
    /** 对方接受后迟迟不传、或传到一半没了动静：结束会话，免得一直挡着别人。 */
    private void watchdog() {
        Incoming s = incoming;
        if (s == null) return;
        long idle = System.currentTimeMillis() - s.touched;
        if (s.state == Incoming.RECEIVING && idle > 45000) {
            boolean some;
            synchronized (s) { some = !s.saved.isEmpty(); }
            s.end(some ? Incoming.DONE : Incoming.FAILED, "timeout", false);
        }
    }

    private void acceptLoop(SSLContext tls) {
        int tries = 0;
        while (running) {
            ServerSocket ss = null;
            try {
                // 53317 被占（例如同机还装着 LocalSend）时往后挪几个端口，端口号会随发现信息告诉对方
                int p = cfg.port + (tries % 4);
                ss = tls != null ? tls.getServerSocketFactory().createServerSocket() : new ServerSocket();
                ss.setReuseAddress(true);
                ss.bind(new InetSocketAddress(p), 64);
                server = ss;
                port = p;
                log("listening on " + (tls != null ? "https" : "http") + " port " + p);
                announce();
                while (running) {
                    final Socket s = ss.accept();
                    pool.execute(new Runnable() { public void run() { serve(s); } });
                }
            } catch (Throwable e) {
                if (running) log("server: " + e);
                tries++;
            } finally { close(ss); }
            try { Thread.sleep(tries < 8 ? 300 : 5000); } catch (InterruptedException e) { return; }
        }
    }

    static final class Req {
        String method, path;
        final Map<String, String> query = new HashMap<>(), headers = new HashMap<>();
        InputStream body;
        String ip;
        Socket socket;
    }
    static final class Res {
        final int code; final String json;
        Res(int code, String json) { this.code = code; this.json = json; }
    }
    private static Res msg(int code, String m) { return new Res(code, LsJson.str(LsJson.obj("message", m))); }

    private void serve(Socket s) {
        try {
            s.setSoTimeout(60000);
            s.setTcpNoDelay(true);
            String ip = ((InetSocketAddress) s.getRemoteSocketAddress()).getAddress().getHostAddress();
            if (!allowed(ip)) return;
            InputStream in = new BufferedInputStream(s.getInputStream(), 65536);
            OutputStream out = s.getOutputStream();
            while (running) {
                String line = readLine(in);
                if (line == null || line.isEmpty()) return;
                String[] p = line.split(" ");
                if (p.length < 2) return;
                Req r = new Req();
                r.method = p[0]; r.ip = ip; r.socket = s;
                String target = p[1];
                int q = target.indexOf('?');
                r.path = q < 0 ? target : target.substring(0, q);
                if (q >= 0) for (String kv : target.substring(q + 1).split("&")) {
                    int e = kv.indexOf('=');
                    if (e > 0) r.query.put(URLDecoder.decode(kv.substring(0, e), "UTF-8"), URLDecoder.decode(kv.substring(e + 1), "UTF-8"));
                }
                String h;
                while ((h = readLine(in)) != null && !h.isEmpty()) {
                    int c = h.indexOf(':');
                    if (c > 0) r.headers.put(h.substring(0, c).trim().toLowerCase(Locale.US), h.substring(c + 1).trim());
                }
                if (h == null) return;
                String te = r.headers.get("transfer-encoding"), cl = r.headers.get("content-length");
                if (te != null && te.toLowerCase(Locale.US).contains("chunked")) r.body = new ChunkedIn(in);
                else r.body = new FixedIn(in, cl == null ? 0 : Long.parseLong(cl));
                if ("100-continue".equalsIgnoreCase(r.headers.get("expect"))) { out.write("HTTP/1.1 100 Continue\r\n\r\n".getBytes(StandardCharsets.US_ASCII)); out.flush(); }
                Res res;
                try { res = route(r); }
                catch (Throwable e) { log("handler: " + e); res = msg(500, String.valueOf(e)); }
                // 请求体没读完就没法在这条连接上接着收下一个请求
                boolean keep = r.body.read() < 0 && !"close".equalsIgnoreCase(r.headers.get("connection"));
                byte[] b = res.json == null ? new byte[0] : res.json.getBytes(StandardCharsets.UTF_8);
                StringBuilder sb = new StringBuilder("HTTP/1.1 ").append(res.code).append(' ').append(reason(res.code)).append("\r\n");
                if (res.code != 204) {
                    if (res.json != null) sb.append("Content-Type: application/json; charset=utf-8\r\n");
                    sb.append("Content-Length: ").append(b.length).append("\r\n");
                }
                sb.append(keep ? "Connection: keep-alive\r\n" : "Connection: close\r\n").append("\r\n");
                out.write(sb.toString().getBytes(StandardCharsets.US_ASCII));
                out.write(b);
                out.flush();
                if (!keep) return;
            }
        } catch (Throwable ignored) {
        } finally { close(s); }
    }
    private static String reason(int c) {
        switch (c) {
            case 200: return "OK"; case 204: return "No Content"; case 400: return "Bad Request"; case 401: return "Unauthorized";
            case 403: return "Forbidden"; case 404: return "Not Found"; case 409: return "Conflict"; case 422: return "Unprocessable Entity";
            case 429: return "Too Many Requests"; default: return "Internal Server Error";
        }
    }
    private static String readLine(InputStream in) throws IOException {
        ByteArrayOutputStream b = new ByteArrayOutputStream(128);
        int c;
        while ((c = in.read()) >= 0) {
            if (c == '\n') break;
            if (c != '\r') b.write(c);
            if (b.size() > 16384) throw new IOException("header too long");
        }
        if (c < 0 && b.size() == 0) return null;
        return new String(b.toByteArray(), StandardCharsets.UTF_8);
    }
    static final class FixedIn extends InputStream {
        final InputStream in; long left;
        FixedIn(InputStream in, long len) { this.in = in; left = Math.max(0, len); }
        public int read() throws IOException { if (left <= 0) return -1; int c = in.read(); if (c < 0) throw new EOFException("body cut short"); left--; return c; }
        public int read(byte[] b, int off, int len) throws IOException {
            if (left <= 0) return -1;
            int n = in.read(b, off, (int) Math.min(len, left));
            if (n < 0) throw new EOFException("body cut short");
            left -= n;
            return n;
        }
    }
    static final class ChunkedIn extends InputStream {
        final InputStream in; long left; boolean end;
        ChunkedIn(InputStream in) { this.in = in; }
        private boolean next() throws IOException {
            if (end) return false;
            if (left > 0) return true;
            String l = readLine(in);
            if (l != null && l.isEmpty()) l = readLine(in); // 上一块结尾的空行
            if (l == null) throw new EOFException("body cut short");
            int sc = l.indexOf(';');
            left = Long.parseLong((sc < 0 ? l : l.substring(0, sc)).trim(), 16);
            if (left == 0) {
                end = true;
                String t;
                while ((t = readLine(in)) != null && !t.isEmpty()) { /* trailers */ }
                return false;
            }
            return true;
        }
        public int read() throws IOException { byte[] one = new byte[1]; int n = read(one, 0, 1); return n < 0 ? -1 : one[0] & 255; }
        public int read(byte[] b, int off, int len) throws IOException {
            if (!next()) return -1;
            int n = in.read(b, off, (int) Math.min(len, left));
            if (n < 0) throw new EOFException("body cut short");
            left -= n;
            return n;
        }
    }
    private static String readBody(InputStream in, int max) throws IOException {
        ByteArrayOutputStream bo = new ByteArrayOutputStream();
        byte[] buf = new byte[8192];
        int n;
        while ((n = in.read(buf)) > 0) { bo.write(buf, 0, n); if (bo.size() > max) throw new IOException("body too large"); }
        return new String(bo.toByteArray(), StandardCharsets.UTF_8);
    }

    private Res route(Req r) throws Exception {
        if (!r.path.startsWith(API)) return msg(404, "Not found");
        String a = r.path.substring(API.length());
        if (a.equals("info") && r.method.equals("GET")) return new Res(200, LsJson.str(info(false)));
        if (!r.method.equals("POST")) return msg(404, "Not found");
        if (a.equals("register")) {
            Map<String, Object> m;
            try { m = LsJson.parseObject(readBody(r.body, 1 << 16)); } catch (IOException e) { return msg(400, "Invalid body"); }
            learn(m, r.ip, PORT, true);
            return new Res(200, LsJson.str(info(false)));
        }
        if (a.equals("prepare-upload")) return prepare(r);
        if (a.equals("upload")) return receive(r);
        if (a.equals("cancel")) return cancelled(r);
        return msg(404, "Not found");
    }

    private Res prepare(Req r) throws Exception {
        Map<String, Object> m, info, files;
        try {
            m = LsJson.parseObject(readBody(r.body, 32 << 20));
            info = LsJson.o(m, "info"); files = LsJson.o(m, "files");
            if (info == null || files == null || files.isEmpty() || LsJson.s(info, "alias") == null) return msg(400, "Invalid body");
        } catch (IOException e) { return msg(400, "Invalid body"); }
        Device d = learn(info, r.ip, PORT, true);
        if (d == null) { // 没带指纹的发送方也照常接待，只是不进设备表
            d = new Device();
            d.alias = LsJson.s(info, "alias"); d.ip = r.ip; d.fingerprint = String.valueOf(LsJson.s(info, "fingerprint"));
            d.port = (int) LsJson.n(info, "port", PORT); d.https = !"http".equals(LsJson.s(info, "protocol"));
            d.model = LsJson.s(info, "deviceModel"); d.type = LsJson.s(info, "deviceType");
        }
        Incoming s = new Incoming(this, d);
        for (Object v : files.values()) {
            if (!(v instanceof Map)) return msg(400, "Invalid body");
            @SuppressWarnings("unchecked") Map<String, Object> f = (Map<String, Object>) v;
            FileMeta fm = new FileMeta();
            fm.id = LsJson.s(f, "id"); fm.name = LsJson.s(f, "fileName"); fm.type = LsJson.s(f, "fileType");
            fm.size = LsJson.n(f, "size", 0); fm.sha256 = LsJson.s(f, "sha256"); fm.preview = LsJson.s(f, "preview");
            if (fm.id == null || fm.name == null) return msg(400, "Invalid body");
            s.files.add(fm);
            s.total += Math.max(0, fm.size);
        }
        // 文字消息：只有一个文本文件且带 preview。显示出来即可，回 204 表示不需要再传。
        if (s.files.size() == 1 && s.files.get(0).preview != null && String.valueOf(s.files.get(0).type).startsWith("text/")) {
            try { listener.onText(d, s.files.get(0).preview); } catch (Throwable ignored) {}
            return new Res(204, null);
        }
        synchronized (this) {
            if (incoming != null && incoming.active()) return msg(409, "Blocked by another session");
            incoming = s;
        }
        try { listener.onIncoming(s); } catch (Throwable e) { s.decline(); }
        boolean ok = s.decided.await(10, TimeUnit.MINUTES);
        if (!ok) s.end(Incoming.DECLINED, "timeout", false);
        if (s.state != Incoming.RECEIVING) return msg(403, "Rejected");
        s.touched = System.currentTimeMillis();
        return new Res(200, LsJson.str(LsJson.obj("sessionId", s.sessionId, "files", new LinkedHashMap<String, Object>(s.tokens))));
    }

    private Res receive(Req r) throws Exception {
        String sid = r.query.get("sessionId"), fid = r.query.get("fileId"), tk = r.query.get("token");
        if (sid == null || fid == null || tk == null) return msg(400, "Missing parameters");
        Incoming s = incoming;
        if (s == null || !s.sessionId.equals(sid) || !s.sender.ip.equals(r.ip)) return msg(403, "Invalid token or IP address");
        FileMeta f = s.file(fid);
        synchronized (s) {
            if (f == null || !tk.equals(s.tokens.get(fid))) return msg(403, "Invalid token or IP address");
            if (s.state != Incoming.RECEIVING || !s.started.add(fid)) return msg(409, "Blocked by another session");
        }
        Target t = null;
        boolean ok = false;
        String err = null;
        int code = 500;
        s.sockets.add(r.socket);
        try {
            t = storage.create(cleanName(f.name), f.type, f.size);
            MessageDigest md = f.sha256 != null && !f.sha256.isEmpty() ? MessageDigest.getInstance("SHA-256") : null;
            try (OutputStream o = t.out()) {
                byte[] buf = new byte[65536];
                int n;
                while ((n = r.body.read(buf)) > 0) {
                    if (s.state != Incoming.RECEIVING) throw new IOException("cancelled");
                    o.write(buf, 0, n);
                    if (md != null) md.update(buf, 0, n);
                    s.done += n;
                    long now = System.currentTimeMillis();
                    s.touched = now;
                    if (now - s.lastNotify > 200) { s.lastNotify = now; notify(s); }
                }
            }
            if (md != null && !f.sha256.equalsIgnoreCase(LsCert.hex(md.digest()))) { code = 422; err = "Checksum mismatch"; }
            else { s.saved.add(t.commit()); ok = true; }
        } catch (Throwable e) { err = String.valueOf(e); }
        finally { s.sockets.remove(r.socket); }
        if (!ok && t != null) t.abort();
        boolean all;
        synchronized (s) {
            if (!ok) { s.failed++; s.error = err; }
            s.finished.add(fid);
            all = s.finished.size() >= s.tokens.size();
        }
        s.touched = System.currentTimeMillis();
        if (all) s.end(s.saved.isEmpty() ? Incoming.FAILED : Incoming.DONE, s.error, false);
        return ok ? new Res(200, null) : msg(code, err);
    }

    /** 对方取消。等待确认阶段对方还没有 sessionId，所以按来源地址认。 */
    private Res cancelled(Req r) {
        String sid = r.query.get("sessionId");
        Incoming s = incoming;
        if (s != null && s.sender.ip.equals(r.ip) && (sid == null || sid.equals(s.sessionId))) { s.end(Incoming.CANCELLED, null, false); return new Res(200, null); }
        // 也可能是接收方取消了我们正在发的那一趟
        Outgoing hit = null;
        synchronized (outgoing) { for (Outgoing o : outgoing) if (o.to.ip.equals(r.ip) && (sid == null || sid.equals(o.sessionId))) hit = o; }
        if (hit != null) {
            hit.cancelled = true;
            hit.state = Outgoing.CANCELLED;
            HttpURLConnection c = hit.conn;
            try { if (c != null) c.disconnect(); } catch (Throwable ignored) {}
            outgoing.remove(hit);
            notify(hit);
            return new Res(200, null);
        }
        return msg(403, "No session");
    }

    /** 文件名可能带目录（发整个文件夹时），也可能不怀好意：只留干净的相对路径。 */
    public static String cleanName(String n) {
        StringBuilder out = new StringBuilder();
        for (String seg : String.valueOf(n).replace('\\', '/').split("/")) {
            StringBuilder b = new StringBuilder();
            for (int i = 0; i < seg.length(); i++) {
                char c = seg.charAt(i);
                if (c >= 0x20 && c != 0x7f && ":*?\"<>|".indexOf(c) < 0) b.append(c);
            }
            String v = b.toString().trim();
            if (v.isEmpty() || v.equals(".") || v.equals("..")) continue;
            if (v.length() > 120) v = v.substring(0, 60) + v.substring(v.length() - 60);
            if (out.length() > 0) out.append('/');
            out.append(v);
        }
        return out.length() == 0 ? "file" : out.toString();
    }

    /** 存到普通文件夹：先写 .part，写完改名；重名时加 (1)、(2)…，从不覆盖已有文件。 */
    public static Storage dirStorage(final File dir) {
        return new Storage() {
            public Target create(String name, String mime, long size) throws IOException {
                final File want = new File(dir, name);
                File parent = want.getParentFile();
                if (!parent.isDirectory() && !parent.mkdirs()) throw new IOException("cannot create " + parent);
                final File part = File.createTempFile(".recv", ".part", parent);
                return new Target() {
                    public OutputStream out() throws IOException { return new FileOutputStream(part); }
                    public String commit() throws IOException {
                        String n = want.getName(), stem = n, ext = "";
                        int dot = n.lastIndexOf('.');
                        if (dot > 0) { stem = n.substring(0, dot); ext = n.substring(dot); }
                        synchronized (LsCore.class) {
                            File f = want;
                            for (int i = 1; f.exists(); i++) f = new File(want.getParentFile(), stem + " (" + i + ")" + ext);
                            if (!part.renameTo(f)) throw new IOException("cannot save " + f);
                            return f.getPath();
                        }
                    }
                    public void abort() { part.delete(); }
                };
            }
        };
    }
}
