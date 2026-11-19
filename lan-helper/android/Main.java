package local.helper.settings;
import android.app.Activity;
import android.os.Bundle;
import android.content.Intent;
import android.widget.*;
public class Main extends Activity {
    public void onCreate(Bundle state) {
        super.onCreate(state);
        LinearLayout list = new LinearLayout(this);
        list.setOrientation(LinearLayout.VERTICAL);
        setContentView(list);
        add(list, "无线调试", new Intent().setClassName("com.android.settings","com.android.settings.Settings$WirelessDebuggingActivity"));
        add(list, "开发者选项", new Intent("android.settings.APPLICATION_DEVELOPMENT_SETTINGS"));
        add(list, "系统设置", new Intent("android.settings.SETTINGS"));
    }
    private void add(LinearLayout list, String title, Intent intent) {
        Button button = new Button(this);
        button.setText(title);
        list.addView(button);
        button.setOnClickListener(v -> {
            try { startActivity(intent); }
            catch (android.content.ActivityNotFoundException | SecurityException e) {
                Toast.makeText(this,"系统未开放此入口",Toast.LENGTH_LONG).show();
            }
        });
    }
}
