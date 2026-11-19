package local.cloud.launcher;

import android.content.Context;
import android.os.Build;
import java.util.Locale;

/** 界面语言跟随系统：中文、俄语、英语。中亚几种语言的系统按俄语显示，其余按英语。 */
final class L {
    static int lang = 2;
    static Locale loc = Locale.ENGLISH;

    static void init(Context c) {
        android.content.res.Configuration cf = c.getResources().getConfiguration();
        Locale l = Build.VERSION.SDK_INT >= 24 ? cf.getLocales().get(0) : cf.locale;
        String g = l == null ? "en" : l.getLanguage();
        if ("zh".equals(g)) { lang = 0; loc = Locale.CHINA; }
        else if ("ru".equals(g) || "ky".equals(g) || "tg".equals(g) || "uz".equals(g) || "kk".equals(g)) { lang = 1; loc = new Locale("ru"); }
        else { lang = 2; loc = Locale.ENGLISH; }
    }
    static String t(String zh, String ru, String en) { return lang == 0 ? zh : lang == 1 ? ru : en; }
}
