package local.cloud.launcher;

import java.io.IOException;
import java.util.*;

/**
 * 极小的 JSON 读写（互传协议专用）。不用 org.json，是为了让协议核心能在普通 JVM 上直接跑测试。
 * 对象 = LinkedHashMap，数组 = ArrayList，数字 = Long 或 Double。
 */
public final class LsJson {
    private final String s;
    private int i;
    private LsJson(String s) { this.s = s; }

    public static Object parse(String text) throws IOException {
        try {
            LsJson p = new LsJson(text);
            Object v = p.value(0);
            p.ws();
            if (p.i != text.length()) throw new IOException("json: trailing data");
            return v;
        } catch (RuntimeException e) { throw new IOException("json: " + e); }
    }
    @SuppressWarnings("unchecked")
    public static Map<String, Object> parseObject(String text) throws IOException {
        Object o = parse(text);
        if (!(o instanceof Map)) throw new IOException("json: not an object");
        return (Map<String, Object>) o;
    }

    private void ws() { while (i < s.length() && s.charAt(i) <= ' ') i++; }
    private Object value(int depth) throws IOException {
        if (depth > 32) throw new IOException("json: too deep");
        ws();
        if (i >= s.length()) throw new IOException("json: unexpected end");
        char c = s.charAt(i);
        if (c == '{') {
            i++;
            Map<String, Object> m = new LinkedHashMap<>();
            ws();
            if (s.charAt(i) == '}') { i++; return m; }
            while (true) {
                ws();
                String k = string();
                ws();
                if (s.charAt(i++) != ':') throw new IOException("json: expected ':'");
                m.put(k, value(depth + 1));
                ws();
                char d = s.charAt(i++);
                if (d == '}') return m;
                if (d != ',') throw new IOException("json: expected ','");
            }
        }
        if (c == '[') {
            i++;
            List<Object> l = new ArrayList<>();
            ws();
            if (s.charAt(i) == ']') { i++; return l; }
            while (true) {
                l.add(value(depth + 1));
                ws();
                char d = s.charAt(i++);
                if (d == ']') return l;
                if (d != ',') throw new IOException("json: expected ','");
            }
        }
        if (c == '"') return string();
        if (s.startsWith("true", i)) { i += 4; return Boolean.TRUE; }
        if (s.startsWith("false", i)) { i += 5; return Boolean.FALSE; }
        if (s.startsWith("null", i)) { i += 4; return null; }
        int st = i;
        while (i < s.length() && "+-0123456789.eE".indexOf(s.charAt(i)) >= 0) i++;
        if (st == i) throw new IOException("json: unexpected '" + c + "'");
        String n = s.substring(st, i);
        if (n.indexOf('.') < 0 && n.indexOf('e') < 0 && n.indexOf('E') < 0) {
            try { return Long.parseLong(n); } catch (NumberFormatException ignored) {}
        }
        return Double.parseDouble(n);
    }
    private String string() throws IOException {
        if (s.charAt(i) != '"') throw new IOException("json: expected string");
        i++;
        StringBuilder b = new StringBuilder();
        while (true) {
            char c = s.charAt(i++);
            if (c == '"') return b.toString();
            if (c != '\\') { b.append(c); continue; }
            char e = s.charAt(i++);
            switch (e) {
                case 'n': b.append('\n'); break;
                case 't': b.append('\t'); break;
                case 'r': b.append('\r'); break;
                case 'b': b.append('\b'); break;
                case 'f': b.append('\f'); break;
                case 'u': b.append((char) Integer.parseInt(s.substring(i, i + 4), 16)); i += 4; break;
                default: b.append(e);
            }
        }
    }

    // ---------- 输出 ----------
    public static String str(Object o) { StringBuilder b = new StringBuilder(); write(b, o); return b.toString(); }
    private static void write(StringBuilder b, Object o) {
        if (o == null) b.append("null");
        else if (o instanceof Map) {
            b.append('{');
            boolean first = true;
            for (Map.Entry<?, ?> e : ((Map<?, ?>) o).entrySet()) {
                if (!first) b.append(',');
                first = false;
                quote(b, String.valueOf(e.getKey()));
                b.append(':');
                write(b, e.getValue());
            }
            b.append('}');
        } else if (o instanceof Collection) {
            b.append('[');
            boolean first = true;
            for (Object x : (Collection<?>) o) { if (!first) b.append(','); first = false; write(b, x); }
            b.append(']');
        } else if (o instanceof Number || o instanceof Boolean) b.append(o);
        else quote(b, String.valueOf(o));
    }
    private static void quote(StringBuilder b, String v) {
        b.append('"');
        for (int k = 0; k < v.length(); k++) {
            char c = v.charAt(k);
            if (c == '"' || c == '\\') b.append('\\').append(c);
            else if (c == '\n') b.append("\\n");
            else if (c == '\r') b.append("\\r");
            else if (c == '\t') b.append("\\t");
            else if (c < 0x20) b.append(String.format("\\u%04x", (int) c));
            else b.append(c);
        }
        b.append('"');
    }

    /** obj("a", 1, "b", "x") */
    public static Map<String, Object> obj(Object... kv) {
        Map<String, Object> m = new LinkedHashMap<>();
        for (int k = 0; k + 1 < kv.length; k += 2) m.put(String.valueOf(kv[k]), kv[k + 1]);
        return m;
    }
    public static String s(Map<String, Object> m, String k) { Object v = m == null ? null : m.get(k); return v == null ? null : String.valueOf(v); }
    public static long n(Map<String, Object> m, String k, long def) { Object v = m == null ? null : m.get(k); return v instanceof Number ? ((Number) v).longValue() : def; }
    @SuppressWarnings("unchecked")
    public static Map<String, Object> o(Map<String, Object> m, String k) { Object v = m == null ? null : m.get(k); return v instanceof Map ? (Map<String, Object>) v : null; }
}
