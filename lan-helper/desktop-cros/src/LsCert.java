package local.cloud.launcher;

import java.io.*;
import java.math.BigInteger;
import java.nio.charset.StandardCharsets;
import java.security.*;
import java.security.cert.CertificateException;
import java.security.cert.CertificateFactory;
import java.security.cert.X509Certificate;
import java.security.spec.PKCS8EncodedKeySpec;
import java.text.SimpleDateFormat;
import java.util.*;
import javax.net.ssl.*;

/**
 * 本机身份：一把 RSA 钥匙加一张自签名证书（协议规定 HTTPS 模式下指纹 = 证书的 SHA-256）。
 * 证书是自己按 DER 格式拼出来的，只用到 java.security，安卓和普通 JVM 上行为一致。
 */
public final class LsCert {
    public PrivateKey key;
    public X509Certificate cert;
    public String fingerprint;

    /** 读已有身份；没有或损坏就新建并保存。 */
    public static LsCert loadOrCreate(File file) throws Exception {
        if (file != null && file.isFile()) {
            try {
                Properties p = new Properties();
                try (InputStream in = new FileInputStream(file)) { p.load(in); }
                LsCert c = new LsCert();
                c.key = KeyFactory.getInstance("RSA").generatePrivate(new PKCS8EncodedKeySpec(unhex(p.getProperty("key"))));
                c.cert = (X509Certificate) CertificateFactory.getInstance("X.509").generateCertificate(new ByteArrayInputStream(unhex(p.getProperty("cert"))));
                c.fingerprint = fingerprint(c.cert);
                return c;
            } catch (Exception ignored) { /* 重新生成 */ }
        }
        LsCert c = create();
        if (file != null) {
            Properties p = new Properties();
            p.setProperty("key", hex(c.key.getEncoded()));
            p.setProperty("cert", hex(c.cert.getEncoded()));
            File tmp = new File(file.getPath() + ".tmp");
            try (OutputStream o = new FileOutputStream(tmp)) { p.store(o, null); }
            if (!tmp.renameTo(file)) throw new IOException("cannot save identity");
        }
        return c;
    }

    public static LsCert create() throws Exception {
        KeyPairGenerator g = KeyPairGenerator.getInstance("RSA");
        g.initialize(2048);
        KeyPair kp = g.generateKeyPair();
        byte[] alg = der(0x30, new byte[]{0x06, 0x09, 0x2A, (byte) 0x86, 0x48, (byte) 0x86, (byte) 0xF7, 0x0D, 0x01, 0x01, 0x0B}, new byte[]{0x05, 0x00});
        byte[] name = der(0x30, der(0x31, der(0x30, new byte[]{0x06, 0x03, 0x55, 0x04, 0x03}, der(0x0C, "LocalSend User".getBytes(StandardCharsets.UTF_8)))));
        SimpleDateFormat f = new SimpleDateFormat("yyMMddHHmmss'Z'", Locale.US);
        f.setTimeZone(TimeZone.getTimeZone("GMT"));
        long now = System.currentTimeMillis();
        byte[] validity = der(0x30,
            der(0x17, f.format(new Date(now - 86400000L)).getBytes(StandardCharsets.US_ASCII)),
            der(0x17, f.format(new Date(now + 3650L * 86400000L)).getBytes(StandardCharsets.US_ASCII)));
        byte[] tbs = der(0x30,
            der(0xA0, der(0x02, new byte[]{2})),
            der(0x02, new BigInteger(63, new SecureRandom()).add(BigInteger.ONE).toByteArray()),
            alg, name, validity, name, kp.getPublic().getEncoded());
        Signature sg = Signature.getInstance("SHA256withRSA");
        sg.initSign(kp.getPrivate());
        sg.update(tbs);
        byte[] sig = sg.sign();
        byte[] bits = new byte[sig.length + 1];
        System.arraycopy(sig, 0, bits, 1, sig.length);
        byte[] der = der(0x30, tbs, alg, der(0x03, bits));
        LsCert c = new LsCert();
        c.key = kp.getPrivate();
        c.cert = (X509Certificate) CertificateFactory.getInstance("X.509").generateCertificate(new ByteArrayInputStream(der));
        c.cert.verify(kp.getPublic());
        c.fingerprint = fingerprint(c.cert);
        return c;
    }

    /** 服务端用的 TLS 上下文。 */
    public SSLContext serverContext() throws Exception {
        char[] pw = "ls".toCharArray();
        KeyStore ks = KeyStore.getInstance(KeyStore.getDefaultType());
        ks.load(null, null);
        ks.setKeyEntry("ls", key, pw, new java.security.cert.Certificate[]{cert});
        KeyManagerFactory kmf = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm());
        kmf.init(ks, pw);
        SSLContext ctx = SSLContext.getInstance("TLS");
        ctx.init(kmf.getKeyManagers(), null, null);
        return ctx;
    }

    /** 客户端：对方都是自签名证书，不走证书链校验；给了指纹就必须与证书的 SHA-256 一致。 */
    public static SSLSocketFactory pinned(final String expect) throws GeneralSecurityException {
        TrustManager tm = new X509TrustManager() {
            public void checkClientTrusted(X509Certificate[] chain, String auth) {}
            public void checkServerTrusted(X509Certificate[] chain, String auth) throws CertificateException {
                if (chain == null || chain.length == 0) throw new CertificateException("no certificate");
                if (expect != null && !expect.isEmpty() && !expect.equalsIgnoreCase(fingerprint(chain[0])))
                    throw new CertificateException("certificate fingerprint mismatch");
            }
            public X509Certificate[] getAcceptedIssuers() { return new X509Certificate[0]; }
        };
        SSLContext ctx = SSLContext.getInstance("TLS");
        ctx.init(null, new TrustManager[]{tm}, null);
        return ctx.getSocketFactory();
    }

    public static String fingerprint(X509Certificate c) throws CertificateException {
        try { return hex(MessageDigest.getInstance("SHA-256").digest(c.getEncoded())).toUpperCase(Locale.US); }
        catch (NoSuchAlgorithmException e) { throw new CertificateException(e); }
    }

    // ---------- 小工具 ----------
    static byte[] der(int tag, byte[]... parts) {
        int n = 0;
        for (byte[] p : parts) n += p.length;
        ByteArrayOutputStream o = new ByteArrayOutputStream(n + 6);
        o.write(tag);
        if (n < 0x80) o.write(n);
        else if (n < 0x100) { o.write(0x81); o.write(n); }
        else if (n < 0x10000) { o.write(0x82); o.write(n >> 8); o.write(n); }
        else { o.write(0x83); o.write(n >> 16); o.write(n >> 8); o.write(n); }
        for (byte[] p : parts) o.write(p, 0, p.length);
        return o.toByteArray();
    }
    public static String hex(byte[] b) {
        char[] d = "0123456789abcdef".toCharArray();
        char[] o = new char[b.length * 2];
        for (int i = 0; i < b.length; i++) { o[i * 2] = d[(b[i] >> 4) & 15]; o[i * 2 + 1] = d[b[i] & 15]; }
        return new String(o);
    }
    static byte[] unhex(String s) {
        byte[] b = new byte[s.length() / 2];
        for (int i = 0; i < b.length; i++) b[i] = (byte) Integer.parseInt(s.substring(i * 2, i * 2 + 2), 16);
        return b;
    }
}
