package local.cloud.launcher;

import android.webkit.MimeTypeMap;
import java.io.File;
import java.text.SimpleDateFormat;
import java.util.*;

/** 文件管理器的小工具：类型判断、大小与日期的显示、不重名的文件名。 */
final class FU {
    static final int DIR = 0, IMAGE = 1, VIDEO = 2, AUDIO = 3, ARCHIVE = 4, APK = 5, PDF = 6, DOC = 7, OTHER = 8;
    static final String APK_MIME = "application/vnd.android.package-archive";
    private static final Set<String> IMG = set("jpg jpeg png gif webp bmp heic heif svg ico tif tiff"),
        VID = set("mp4 mkv webm avi mov 3gp m4v ts flv wmv mpg mpeg"),
        AUD = set("mp3 m4a aac ogg oga opus wav flac amr mid midi wma"),
        ARC = set("zip rar 7z tar gz tgz bz2 xz zst iso jar"),
        TXT = set("txt md log ini conf cfg json xml csv yml yaml sh py java kt c h cpp js ts css html htm sql srt prop properties toml rs go"),
        OFFICE = set("doc docx xls xlsx ppt pptx odt ods odp rtf epub");

    private static Set<String> set(String s) { return new HashSet<>(Arrays.asList(s.split(" "))); }

    static String ext(String name) {
        int i = name.lastIndexOf('.');
        return i > 0 && i < name.length() - 1 ? name.substring(i + 1).toLowerCase(Locale.ROOT) : "";
    }
    static int kind(String name, boolean dir) {
        if (dir) return DIR;
        String e = ext(name);
        if (e.isEmpty()) return OTHER;
        if (IMG.contains(e)) return IMAGE;
        if (VID.contains(e)) return VIDEO;
        if (AUD.contains(e)) return AUDIO;
        if (ARC.contains(e)) return ARCHIVE;
        if (e.equals("apk")) return APK;
        if (e.equals("pdf")) return PDF;
        if (TXT.contains(e) || OFFICE.contains(e)) return DOC;
        return OTHER;
    }
    static String mime(File f) {
        String e = ext(f.getName());
        if (e.equals("apk")) return APK_MIME;
        String m = null;
        try { m = MimeTypeMap.getSingleton().getMimeTypeFromExtension(e); } catch (Throwable ignored) {}
        if (m != null) return m;
        if (TXT.contains(e)) return "text/plain";
        switch (kind(f.getName(), false)) {
            case IMAGE: return "image/*";
            case VIDEO: return "video/*";
            case AUDIO: return "audio/*";
            default: return "*/*";
        }
    }
    static String typeLabel(String name, boolean dir) {
        String e = ext(name).toUpperCase(Locale.ROOT);
        switch (kind(name, dir)) {
            case DIR: return L.t("文件夹", "Папка", "Folder");
            case IMAGE: return L.t("图片", "Изображение", "Image") + " " + e;
            case VIDEO: return L.t("视频", "Видео", "Video") + " " + e;
            case AUDIO: return L.t("音频", "Аудио", "Audio") + " " + e;
            case ARCHIVE: return L.t("压缩包", "Архив", "Archive") + " " + e;
            case APK: return L.t("安装包", "Установочный пакет", "App package");
            case PDF: return L.t("PDF 文档", "Документ PDF", "PDF document");
            case DOC: return L.t("文档", "Документ", "Document") + " " + e;
            default: return e.isEmpty() ? L.t("文件", "Файл", "File") : L.t("文件", "Файл", "File") + " " + e;
        }
    }
    static String size(long b) {
        if (b < 0) return "";
        if (b < 1024) return b + " B";
        String[] u = {"KB", "MB", "GB", "TB"};
        double v = b;
        int i = -1;
        do { v /= 1024; i++; } while (v >= 1024 && i < u.length - 1);
        return String.format(Locale.US, v >= 100 ? "%.0f %s" : v >= 10 ? "%.1f %s" : "%.2f %s", v, u[i]);
    }
    static String date(long t) {
        if (t <= 0) return "";
        return new SimpleDateFormat("yyyy-MM-dd HH:mm", Locale.US).format(new Date(t));
    }
    /** dir 里不存在的名字：a.txt → a (2).txt → a (3).txt … */
    static File unique(File dir, String name) {
        File f = new File(dir, name);
        if (!f.exists()) return f;
        String base = name, ext = "";
        int i = name.lastIndexOf('.');
        if (i > 0) { base = name.substring(0, i); ext = name.substring(i); }
        for (int n = 2; n < 10000; n++) {
            f = new File(dir, base + " (" + n + ")" + ext);
            if (!f.exists()) return f;
        }
        return new File(dir, base + " (" + System.currentTimeMillis() + ")" + ext);
    }
    static boolean under(File f, File root) {
        String a = f.getAbsolutePath(), b = root.getAbsolutePath();
        return a.equals(b) || a.startsWith(b.endsWith("/") ? b : b + "/");
    }
    /** 按自然顺序比较：file2 排在 file10 前面。 */
    static int natural(String a, String b) {
        int i = 0, j = 0, na = a.length(), nb = b.length();
        while (i < na && j < nb) {
            char x = a.charAt(i), y = b.charAt(j);
            if (Character.isDigit(x) && Character.isDigit(y)) {
                int si = i, sj = j;
                while (si < na && a.charAt(si) == '0') si++;
                while (sj < nb && b.charAt(sj) == '0') sj++;
                int ei = si, ej = sj;
                while (ei < na && Character.isDigit(a.charAt(ei))) ei++;
                while (ej < nb && Character.isDigit(b.charAt(ej))) ej++;
                if (ei - si != ej - sj) return (ei - si) - (ej - sj);
                for (int k = 0; k < ei - si; k++) {
                    int d = a.charAt(si + k) - b.charAt(sj + k);
                    if (d != 0) return d;
                }
                i = ei; j = ej;
            } else {
                if (x != y) return x - y;
                i++; j++;
            }
        }
        return (na - i) - (nb - j);
    }
}
