package local.cloud.launcher;

import java.io.*;
import java.security.MessageDigest;
import java.util.*;
import java.util.concurrent.*;

/**
 * 协议核心的电脑端测试（不进 APK）。全部走本机回环：组播只发在 lo0，HTTP 只连 127.0.0.1。
 *   all                     两个/三个核心实例互相发现、互传、各种拒绝/取消/校验
 *   serve PORT DIR MPORT [http]   起一个自动接受的实例，供 curl 或真实 LocalSend 来连
 */
public class LsTest {
    static int fails;
    static void check(boolean ok, String what) { System.out.println((ok ? "  ok    " : "  FAIL  ") + what); if (!ok) fails++; }

    static class Node implements LsCore.Listener {
        final String name; final File dir; LsCore core;
        volatile int policy = 1; // 1 接受 0 拒绝 2 不回应
        final BlockingQueue<LsCore.Incoming> ended = new LinkedBlockingQueue<>();
        final BlockingQueue<LsCore.Incoming> asked = new LinkedBlockingQueue<>();
        final BlockingQueue<String> texts = new LinkedBlockingQueue<>();
        final BlockingQueue<LsCore.Outgoing> outEnded = new LinkedBlockingQueue<>();
        boolean verbose;
        Node(String name, int port, int mport, boolean https, boolean hash, File root) {
            this.name = name;
            dir = new File(root, name + "-recv");
            LsCore.Config c = new LsCore.Config();
            c.alias = name; c.port = port; c.multicastPort = mport; c.https = https; c.hashOutgoing = hash;
            c.stateDir = new File(root, name + "-state"); c.stateDir.mkdirs();
            c.onlyInterface = "lo0"; c.onlyHosts = new String[]{"127.0.0.1"};
            core = new LsCore(c, LsCore.dirStorage(dir), this);
        }
        public void onDevices() {}
        public void onIncoming(final LsCore.Incoming s) {
            asked.add(s);
            if (verbose) System.out.println("[" + name + "] request from " + s.sender + ": " + s.files.size() + " file(s), " + s.total + " bytes");
            new Thread(() -> { try { Thread.sleep(150); } catch (InterruptedException e) {} if (policy == 1) s.accept(); else if (policy == 0) s.decline(); }).start();
        }
        public void onIncomingUpdate(LsCore.Incoming s) { if (!s.active()) { ended.add(s); if (verbose) System.out.println("[" + name + "] session ended state=" + s.state + " saved=" + s.saved + " err=" + s.error); } }
        public void onText(LsCore.Device from, String text) { texts.add(text); if (verbose) System.out.println("[" + name + "] text from " + from + ": " + text); }
        public void onOutgoingUpdate(LsCore.Outgoing o) { if (!o.active()) outEnded.add(o); }
        public void log(String s) { System.out.println("[" + name + "] " + s); }
        LsCore.Device find(String alias) throws Exception {
            for (int i = 0; i < 100; i++) { for (LsCore.Device d : core.devices()) if (d.alias.equals(alias)) return d; Thread.sleep(100); }
            return null;
        }
    }

    static String sha(File f) throws Exception {
        MessageDigest md = MessageDigest.getInstance("SHA-256");
        try (InputStream in = new FileInputStream(f)) { byte[] b = new byte[65536]; int n; while ((n = in.read(b)) > 0) md.update(b, 0, n); }
        return LsCert.hex(md.digest());
    }
    static File make(File dir, String name, int size, long seed) throws Exception {
        File f = new File(dir, name);
        f.getParentFile().mkdirs();
        Random r = new Random(seed);
        try (OutputStream o = new BufferedOutputStream(new FileOutputStream(f))) { byte[] b = new byte[8192]; for (int left = size; left > 0; left -= b.length) { r.nextBytes(b); o.write(b, 0, Math.min(b.length, left)); } }
        return f;
    }
    static LsCore.Source src(final File f, final String sendAs) {
        return new LsCore.Source() {
            public String name() { return sendAs; }
            public long size() { return f.length(); }
            public String mime() { return "application/octet-stream"; }
            public InputStream open() throws IOException { return new FileInputStream(f); }
        };
    }
    static LsCore.Outgoing waitOut(Node n, LsCore.Outgoing o) throws Exception {
        for (int i = 0; i < 600 && o.active(); i++) Thread.sleep(100);
        Thread.sleep(150);
        return o;
    }
    /** from → to 发三份文件并逐个比对 sha256。 */
    static void transfer(Node from, Node to, File srcDir, String tag) throws Exception {
        LsCore.Device d = from.find(to.name);
        check(d != null, tag + ": " + from.name + " discovered " + to.name + (d == null ? "" : " as " + d));
        if (d == null) return;
        check(d.port == to.core.port && d.https == to.core.https && d.fingerprint.equals(to.core.fingerprint), tag + ": discovered port/protocol/fingerprint match the peer's own");
        File a = make(srcDir, tag + "-empty.bin", 0, 1), b = make(srcDir, tag + "-照片 фото.bin", 1500, 2), c = make(srcDir, tag + "-big.bin", 20 * 1024 * 1024 + 123, 3);
        to.ended.clear();
        long t0 = System.currentTimeMillis();
        LsCore.Outgoing o = waitOut(from, from.core.send(d, Arrays.asList(src(a, a.getName()), src(b, b.getName()), src(c, "sub/dir/" + c.getName())), null));
        check(o.state == LsCore.Outgoing.DONE && o.sent == 3, tag + ": sender finished DONE, 3 files sent (state=" + o.state + " err=" + o.error + ", " + (System.currentTimeMillis() - t0) + " ms)");
        LsCore.Incoming in = to.ended.poll(5, TimeUnit.SECONDS);
        check(in != null && in.state == LsCore.Incoming.DONE && in.saved.size() == 3 && in.done == in.total, tag + ": receiver finished DONE with 3 saved files, byte count matches");
        check(sha(a).equals(sha(new File(to.dir, a.getName()))), tag + ": sha256 equal, 0-byte file");
        check(sha(b).equals(sha(new File(to.dir, b.getName()))), tag + ": sha256 equal, 1.5 KB file with CJK/Cyrillic name");
        check(sha(c).equals(sha(new File(to.dir, "sub/dir/" + c.getName()))), tag + ": sha256 equal, 20 MB file in a sub-folder");
    }

    public static void main(String[] args) throws Exception {
        if (args.length > 0 && args[0].equals("serve")) {
            File root = new File(args[2]);
            Node n = new Node("TestLaptop", Integer.parseInt(args[1]), Integer.parseInt(args[3]), !(args.length > 4 && args[4].equals("http")), false, root);
            n.verbose = true;
            n.core.start();
            Thread.sleep(1500);
            System.out.println("READY fingerprint=" + n.core.fingerprint + " port=" + n.core.port + " https=" + n.core.https);
            Thread.sleep(Long.MAX_VALUE);
        }
        if (args.length > 0 && args[0].equals("send")) { // send IP PORT FILE DIR WAIT_SECONDS：探测对方并发一个文件，超时就取消
            Node n = new Node("TestLaptop", 53412, 53398, true, false, new File(args[4]));
            n.core.start();
            Thread.sleep(1500);
            LsCore.Device d = n.core.probe(args[1], Integer.parseInt(args[2]));
            System.out.println("probe -> " + d + (d == null ? "" : " fingerprint=" + d.fingerprint + " model=" + d.model));
            if (d == null) System.exit(2);
            File f = new File(args[3]);
            LsCore.Outgoing o = n.core.send(d, Arrays.asList(src(f, f.getName())), null);
            for (int i = 0; i < Integer.parseInt(args[5]) * 10 && o.active(); i++) Thread.sleep(100);
            System.out.println("after wait: state=" + o.state + " (0 waiting for the user, 1 sending, 2 done, 3 declined, 4 busy, 5 pin, 7 failed) sent=" + o.sent + " err=" + o.error);
            if (o.active()) { o.cancel(); Thread.sleep(1500); System.out.println("cancelled by us: state=" + o.state); }
            System.exit(0);
        }
        File root = new File(args.length > 1 ? args[1] : "test/out/run");
        File srcDir = new File(root, "src");
        int mp = 53399;
        Node A = new Node("Alpha", 53401, mp, true, true, root), B = new Node("Bravo", 53402, mp, true, false, root), C = new Node("Charlie-http", 53403, mp, false, false, root);
        A.core.start(); B.core.start(); C.core.start();

        System.out.println("== 1. discovery over multicast (lo0) + two-way transfer, HTTPS <-> HTTPS");
        transfer(A, B, srcDir, "A2B");   // A 带 sha256，B 会校验
        transfer(B, A, srcDir, "B2A");
        check(A.core.https && B.core.https && A.core.fingerprint.matches("[0-9A-F]{64}"), "both serve HTTPS; fingerprint is the certificate SHA-256");

        System.out.println("== 2. mixed: HTTP-only peer <-> HTTPS peer");
        check(!C.core.https, "Charlie serves plain HTTP with a random fingerprint");
        transfer(A, C, srcDir, "A2C");
        transfer(C, A, srcDir, "C2A");

        System.out.println("== 3. decline -> 403");
        LsCore.Device dB = A.find("Bravo");
        File small = make(srcDir, "small.bin", 4096, 9);
        B.policy = 0;
        LsCore.Outgoing o = waitOut(A, A.core.send(dB, Arrays.asList(src(small, "declined.bin")), null));
        check(o.state == LsCore.Outgoing.DECLINED && !new File(B.dir, "declined.bin").exists(), "sender sees DECLINED, nothing saved");

        System.out.println("== 4. busy -> 409, then sender cancels while waiting");
        B.policy = 2; B.asked.clear(); B.ended.clear();
        LsCore.Outgoing w = A.core.send(dB, Arrays.asList(src(small, "waiting.bin")), null);
        check(B.asked.poll(5, TimeUnit.SECONDS) != null, "receiver was asked");
        LsCore.Device dBfromC = C.find("Bravo");
        LsCore.Outgoing busy = waitOut(C, C.core.send(dBfromC, Arrays.asList(src(small, "busy.bin")), null));
        check(busy.state == LsCore.Outgoing.BUSY, "second sender gets BUSY (409)");
        w.cancel();
        LsCore.Incoming ci = B.ended.poll(5, TimeUnit.SECONDS);
        check(ci != null && ci.state == LsCore.Incoming.CANCELLED, "receiver session ends CANCELLED after sender's /cancel");
        B.policy = 1;
        o = waitOut(A, A.core.send(dB, Arrays.asList(src(small, "after-cancel.bin")), null));
        check(o.state == LsCore.Outgoing.DONE && sha(small).equals(sha(new File(B.dir, "after-cancel.bin"))), "receiver is free again: next transfer succeeds");

        System.out.println("== 5. text message (file with preview) -> 204");
        String msg = "你好 привет hello \"quoted\"\nline2 https://example.com";
        o = waitOut(A, A.core.sendText(dB, msg, null));
        check(o.state == LsCore.Outgoing.DONE, "sender DONE on 204");
        check(msg.equals(B.texts.poll(3, TimeUnit.SECONDS)), "receiver got the identical text");

        System.out.println("== 6. checksum mismatch -> 422, file discarded");
        final int[] opens = {0};
        LsCore.Source liar = new LsCore.Source() {
            public String name() { return "tampered.bin"; }
            public long size() { return 4096; }
            public String mime() { return null; }
            public InputStream open() { byte[] b = new byte[4096]; Arrays.fill(b, (byte) opens[0]++); return new ByteArrayInputStream(b); }
        };
        B.ended.clear();
        o = waitOut(A, A.core.send(dB, Arrays.asList(liar), null));
        LsCore.Incoming bad = B.ended.poll(5, TimeUnit.SECONDS);
        check(o.state == LsCore.Outgoing.FAILED && "HTTP 422".equals(o.error), "sender sees 422 (state=" + o.state + " err=" + o.error + ")");
        check(bad != null && bad.state == LsCore.Incoming.FAILED && !new File(B.dir, "tampered.bin").exists(), "receiver FAILED and kept no file");

        System.out.println("== 7. certificate pinning");
        LsCore.Device fake = new LsCore.Device();
        fake.alias = "fake"; fake.ip = dB.ip; fake.port = dB.port; fake.https = true;
        fake.fingerprint = dB.fingerprint.substring(0, 63) + (dB.fingerprint.endsWith("0") ? "1" : "0");
        o = waitOut(A, A.core.send(fake, Arrays.asList(src(small, "pin.bin")), null));
        check(o.state == LsCore.Outgoing.FAILED && !new File(B.dir, "pin.bin").exists(), "send to a peer whose certificate does not match the announced fingerprint fails (" + o.error + ")");

        System.out.println("== 8. receiver cancels mid-transfer");
        File big = make(srcDir, "cancel-big.bin", 200 * 1024 * 1024, 5);
        B.ended.clear();
        B.asked.clear();
        final LsCore.Outgoing bo = A.core.send(dB, Arrays.asList(src(big, "cancel-big.bin")), null);
        LsCore.Incoming bi = B.asked.poll(5, TimeUnit.SECONDS);
        for (int i = 0; i < 200 && (bi == null || bi.done < 1024 * 1024); i++) { Thread.sleep(5); if (bi == null) bi = B.asked.poll(); }
        if (bi != null) bi.cancel();
        waitOut(A, bo);
        check(bi != null && bi.state == LsCore.Incoming.CANCELLED && !new File(B.dir, "cancel-big.bin").exists(), "receiver CANCELLED, partial file removed");
        check(bo.state == LsCore.Outgoing.CANCELLED || bo.state == LsCore.Outgoing.FAILED, "sender stops (state=" + bo.state + ")");
        String[] left = B.dir.list((d, n) -> n.endsWith(".part"));
        check(left == null || left.length == 0, "no .part leftovers");

        System.out.println("== 9. identity is stable, hostile names are neutralised");
        String fp = A.core.fingerprint;
        check(LsCert.loadOrCreate(new File(root, "Alpha-state/identity")).fingerprint.equals(fp), "fingerprint identical after reloading identity from disk");
        check(LsCore.cleanName("../../etc/pass:wd").equals("etc/passwd") && LsCore.cleanName("/..\\..").equals("file"), "path traversal stripped from file names");
        check(LsJson.str(LsJson.parse("{\"a\":[1,2.5,true,null,\"x\\u4f60\\n\"],\"b\":{}}")).equals("{\"a\":[1,2.5,true,null,\"x你\\n\"],\"b\":{}}"), "JSON round-trip");

        A.core.stop(); B.core.stop(); C.core.stop();
        System.out.println(fails == 0 ? "ALL PASSED" : fails + " FAILED");
        System.exit(fails == 0 ? 0 : 1);
    }
}
