package local.helper.home;
import android.app.Activity;
import android.os.Bundle;
import android.content.Intent;
import android.content.pm.ResolveInfo;
import android.widget.*;
import org.json.*;
import java.net.*;
import java.io.*;
import java.util.Scanner;

public class Main extends Activity {
    TextView status;
    Button start;
    String desktops;
    final String base = "http://10.158.25.5:8000/";
    public void onCreate(Bundle state) {
        super.onCreate(state);
        LinearLayout list = new LinearLayout(this);
        list.setOrientation(LinearLayout.VERTICAL);
        setContentView(list);
        StringBuilder names = new StringBuilder();
        Intent home = new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME);
        for (ResolveInfo item : getPackageManager().queryIntentActivities(home, 0))
            names.append(item.loadLabel(getPackageManager())).append("  ").append(item.activityInfo.packageName).append("\n");
        desktops = names.toString();
        status = new TextView(this);
        status.setText("系统可见的桌面\n" + desktops);
        list.addView(status);
        start = new Button(this);
        start.setText("把桌面列表发给 Mac，执行一次设置操作");
        list.addView(start);
        start.setOnClickListener(v -> { start.setEnabled(false); new Thread(this::request).start(); });
    }
    HttpURLConnection connect(String path) throws IOException {
        HttpURLConnection c = (HttpURLConnection)new URL(base + path).openConnection();
        c.setConnectTimeout(4000);
        c.setReadTimeout(4000);
        c.setInstanceFollowRedirects(false);
        c.setUseCaches(false);
        return c;
    }
    void request() {
        String report = "列表仅显示在本机";
        try {
            HttpURLConnection c = connect("log");
            try {
                c.setRequestMethod("POST");
                c.setDoOutput(true);
                c.setRequestProperty("Content-Type","application/json");
                try(OutputStream out = c.getOutputStream()) {
                    out.write(new JSONObject().put("type","home_list").put("desktops",desktops).toString().getBytes("UTF-8"));
                }
                if (c.getResponseCode()==204) report = "桌面列表已发到 Mac";
            } finally { c.disconnect(); }
        } catch(Exception ignored) {}
        final String message = report;
        try {
            HttpURLConnection c = connect("home-action.txt");
            String action;
            try (Scanner s = new Scanner(c.getInputStream(),"UTF-8")) { action=s.nextLine().trim(); }
            finally { c.disconnect(); }
            runOnUiThread(() -> {
                start.setEnabled(true);
                status.setText(message+"\n"+desktops);
                if ("open-home".equals(action)) {
                    try { startActivity(new Intent("android.settings.HOME_SETTINGS")); }
                    catch(RuntimeException e) { status.append("\n系统未开放默认桌面设置"); }
                }
            });
        } catch(Exception e) { runOnUiThread(() -> {start.setEnabled(true);status.setText(message+"\n"+desktops+"\n读取操作失败，请检查同一 Wi-Fi 和内网页面。");}); }
    }
}
