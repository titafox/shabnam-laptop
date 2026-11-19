package local.cloud.launcher;

import android.app.Activity;
import android.content.ClipData;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import android.widget.Toast;
import java.util.ArrayList;
import java.util.List;

/** 透明的过渡页：只负责调出系统的文件选择器（可多选），把选中的文件交给互传。 */
public class SendPick extends Activity {
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        if (state != null) return;
        try {
            startActivityForResult(new Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
                .setType("*/*").putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true), 1);
        } catch (Throwable e) {
            Send.pendingTarget = null;
            try { Toast.makeText(this, L.t("没有可用的文件选择器", "Нет приложения для выбора файлов", "No file picker available"), Toast.LENGTH_SHORT).show(); } catch (Throwable ignored) {}
            finish();
        }
    }
    protected void onActivityResult(int req, int res, Intent data) {
        super.onActivityResult(req, res, data);
        try {
            List<Uri> uris = new ArrayList<>();
            if (res == RESULT_OK && data != null) {
                ClipData cd = data.getClipData();
                if (cd != null) for (int i = 0; i < cd.getItemCount(); i++) { Uri u = cd.getItemAt(i).getUri(); if (u != null) uris.add(u); }
                else if (data.getData() != null) uris.add(data.getData());
            }
            if (uris.isEmpty()) Send.pendingTarget = null;
            else Send.sendFiles(getApplicationContext(), uris);
        } catch (Throwable ignored) {}
        finish();
    }
}
