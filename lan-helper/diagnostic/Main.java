package local.helper.diagnostic;
import android.app.Activity;
import android.os.*;
import android.content.*;
import android.content.pm.*;
import android.provider.Settings;
import android.widget.*;
import org.json.*;
import java.io.*;
import java.net.*;
import java.util.Scanner;
import java.util.concurrent.*;

public class Main extends Activity {
    TextView status;
    ScheduledExecutorService worker;
    volatile int generation;
    String previous="";
    static final String BASE="http://10.158.25.5:8000/";
    public void onCreate(Bundle state) {
        super.onCreate(state);
        LinearLayout list=new LinearLayout(this);
        list.setOrientation(LinearLayout.VERTICAL);
        setContentView(list);
        status=new TextView(this);
        status.setText("开始后将固件版本、板型、设备树、启动及 USB 配置发到 Mac。保持此页打开可以接收 Mac 推送的 APK。");
        list.addView(status);
        Button start=new Button(this);
        start.setText("连接 Mac");
        list.addView(start);
        start.setOnClickListener(v -> {
            if(worker!=null)return;
            int session=++generation;
            previous="";
            worker=Executors.newSingleThreadScheduledExecutor();
            worker.scheduleWithFixedDelay(() -> poll(session),0,4,TimeUnit.SECONDS);
        });
    }
    protected void onPause(){
        generation++;
        if(worker!=null){worker.shutdownNow();worker=null;}
        super.onPause();
    }
    HttpURLConnection connect(String path) throws IOException {
        HttpURLConnection c=(HttpURLConnection)new URL(BASE+path).openConnection();
        c.setConnectTimeout(2500); c.setReadTimeout(2500);
        c.setUseCaches(false); c.setInstanceFollowRedirects(false);
        return c;
    }
    JSONObject snapshot() throws Exception {
        JSONObject data=new JSONObject();
        data.put("type","device_diagnosis").put("model",Build.MODEL).put("board",Build.BOARD).put("android",Build.VERSION.RELEASE);
        for(String key:new String[]{"adb_enabled","adb_wifi_enabled","development_settings_enabled"}){
            try {data.put(key,Settings.Global.getString(getContentResolver(),key));}
            catch(SecurityException denied){data.put(key,"restricted");}
        }
        Intent usb=registerReceiver(null,new IntentFilter("android.hardware.usb.action.USB_STATE"));
        if(usb!=null && usb.getExtras()!=null){
            JSONObject flags=new JSONObject();
            for(String k:usb.getExtras().keySet()) if(usb.getExtras().get(k) instanceof Boolean) flags.put(k,usb.getBooleanExtra(k,false));
            data.put("usb",flags);
        }
        Intent home=new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME);
        JSONArray apps=new JSONArray();
        for(ResolveInfo r:getPackageManager().queryIntentActivities(home,0)) apps.put(r.activityInfo.packageName);
        data.put("home_candidates",apps);
        ResolveInfo chosen=getPackageManager().resolveActivity(home,PackageManager.MATCH_DEFAULT_ONLY);
        if(chosen!=null)data.put("resolved_home",chosen.activityInfo.packageName);
        data.put("wifi_feature",getPackageManager().hasSystemFeature(PackageManager.FEATURE_WIFI));
        data.put("probe_version",7).put("product",Build.PRODUCT).put("device",Build.DEVICE)
            .put("hardware",Build.HARDWARE).put("bootloader",Build.BOOTLOADER)
            .put("fingerprint",Build.FINGERPRINT).put("display",Build.DISPLAY)
            .put("manufacturer",Build.MANUFACTURER).put("abis",String.join(",",Build.SUPPORTED_ABIS));
        JSONObject nodes=new JSONObject();
        for(String root:new String[]{"/proc/device-tree/","/sys/firmware/devicetree/base/"}){
            for(String name:new String[]{"model","compatible"}){
                String path=root+name;
                try(InputStream in=new FileInputStream(path)){nodes.put(path,readValue(in));}
                catch(IOException | SecurityException e){nodes.put(path,"unavailable: "+e.getMessage());}
            }
        }
        data.put("device_tree",nodes);
        JSONObject props=new JSONObject();
        for(String key:new String[]{"ro.product.vendor.device","ro.board.platform","ro.boot.hardware",
                "ro.boot.hardware.sku","ro.boot.boot_devices","ro.boot.slot_suffix","ro.boot.verifiedbootstate",
                "ro.boot.flash.locked","ro.boot.vbmeta.device_state","ro.build.ab_update","ro.vendor.build.fingerprint",
                "service.adb.tcp.port","persist.adb.tcp.port","service.adb.tls.port","persist.adb.tls_server.enable","init.svc.adbd","ro.adb.secure","sys.usb.config","sys.usb.state","sys.usb.controller","persist.sys.usb.config","vendor.usb.controller","sys.usb.configfs","sys.usb.ffs.ready"}){
            try {
                java.lang.Process process=new ProcessBuilder("/system/bin/getprop",key).start();
                try(InputStream in=process.getInputStream()){props.put(key,readValue(in));}
                finally{process.destroy();}
            } catch(IOException | SecurityException e){props.put(key,"unavailable");}
        }
        data.put("properties",props);
        data.put("usb_diagnosis",usbDiagnosis());
        return data;
    }
    String readNode(File file) {
        try(InputStream in=new FileInputStream(file)){return readValue(in);}
        catch(IOException | SecurityException e){return "unavailable: "+e.getMessage();}
    }
    void inspectDt(File dir,String prefix,int depth,JSONObject result) throws Exception {
        File[] children=dir.listFiles();
        if(children==null)return;
        for(File node:children){
            String n=node.getName();
            if(!node.isDirectory() || !(n.startsWith("usb") || n.startsWith("dwc3") || n.startsWith("soc@") || n.equals("soc")))continue;
            String path=prefix+"/"+n;
            if(new File(node,"dr_mode").exists() || n.startsWith("usb@") || n.startsWith("dwc3@")){
                JSONObject values=new JSONObject();
                for(String prop:new String[]{"compatible","status","dr_mode"}){
                    File f=new File(node,prop);
                    if(f.exists())values.put(prop,readNode(f));
                }
                if(result.length()<32)result.put(path,values);
            }
            if(depth>0)inspectDt(node,path,depth-1,result);
        }
    }
    JSONObject usbDiagnosis() throws Exception {
        JSONObject result=new JSONObject();
        for(String[] spec:new String[][]{{"udc","state"},{"usb_role","role"},{"typec","data_role"},{"extcon","state"}}){
            File[] entries=new File("/sys/class/"+spec[0]).listFiles();
            JSONObject readings=new JSONObject();
            if(entries==null)readings.put("access","unavailable");
            else for(File entry:entries){
                if(readings.length()>=12)break;
                readings.put(entry.getName(),readNode(new File(entry,spec[1])));
            }
            result.put(spec[0],readings);
        }
        result.put("gadget_binding",readNode(new File("/config/usb_gadget/g1/UDC")));
        JSONObject modes=new JSONObject();
        File[] platform=new File("/sys/devices/platform").listFiles();
        if(platform==null)modes.put("access","unavailable");
        else for(File dev:platform){
            File node=new File(dev,"otg_mode");
            if(node.exists())modes.put(node.getPath(),readNode(node));
        }
        for(String path:new String[]{"/sys/devices/platform/fe8a0000.usb2-phy/otg_mode","/sys/devices/platform/usbdrd/fcc00000.dwc3/otg_mode"})
            if(!modes.has(path))modes.put(path,readNode(new File(path)));
        result.put("otg_mode",modes);
        JSONArray pages=new JSONArray();
        try{
            for(ActivityInfo a:getPackageManager().getPackageInfo("com.android.settings",PackageManager.GET_ACTIVITIES).activities)
                if(a.name.toLowerCase().contains("usb") && pages.length()<24)pages.put(a.name.replace("com.android.settings.","")+(a.exported?"":" (未导出)"));
        }catch(Exception e){pages.put("unavailable: "+e);}
        result.put("settings_usb_pages",pages);
        JSONObject dt=new JSONObject();
        inspectDt(new File("/proc/device-tree"),"",2,dt);
        result.put("device_tree_controllers",dt);
        return result;
    }
    String readValue(InputStream input) throws IOException {
        byte[] buffer=new byte[1024];
        int count;
        ByteArrayOutputStream out=new ByteArrayOutputStream();
        while(out.size()<1024 && (count=input.read(buffer,0,1024-out.size()))>0)out.write(buffer,0,count);
        return out.toString("UTF-8").replace((char)0,' ').trim();
    }
    void install(String line,int session) throws Exception {
        String[] words=line.split(" ");
        if(words.length!=4 || !words[2].matches("[0-9a-f]{64}"))throw new IOException("安装请求格式错误");
        java.security.Signature verifier=java.security.Signature.getInstance("SHA256withRSA");
        byte[] key=android.util.Base64.decode("MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAqggtXHbevwwjzsuRYP+w1nvt3dPmRIOpR8hrq1v1MR/kaZeG/kB8vDkn1vemDa+YI4E03pNXH2j7GGvG5UNW1orcYdDtpYQDcSN0y4SjtxN31x7zmEVPZ71dT3PM9tdy//htLiWphqku5SEs8Z8LO2rKRVO/EXIeRgHYwjnUZNLulMRFLpNFk4v5kNPYO2KS0G9wc2wJIs2MsP0xpoBIuxlvNRifa9NXFja2fslcz+lnASW+sKaLMJ5K6xYeFZnMkNxvDROXAWND9MTdlxRhpbSiGECEyvqN0U26TSLQZ6aT76UW9r8xs1M4z8MY5wFnWEvVHZuDdcWAV9NF7b3slwIDAQAB",0);
        verifier.initVerify(java.security.KeyFactory.getInstance("RSA").generatePublic(new java.security.spec.X509EncodedKeySpec(key)));
        verifier.update(line.substring(0,line.lastIndexOf(' ')).getBytes("UTF-8"));
        if(!verifier.verify(android.util.Base64.decode(words[3],0)))throw new IOException("Mac 签名无效");
        if(line.equals(getPreferences(0).getString("delivered","")))return;
        if(!getPackageManager().canRequestPackageInstalls()){
            previous=line;
            runOnUiThread(()->{if(session==generation)startActivity(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,android.net.Uri.parse("package:"+getPackageName())));});
            return;
        }
        String name=words[2]+".apk";
        File tmp=File.createTempFile("pkg",".tmp",getCacheDir());
        HttpURLConnection c=connect(name);
        try{
            java.security.MessageDigest sha=java.security.MessageDigest.getInstance("SHA-256");
            try(InputStream in=c.getInputStream();OutputStream out=new FileOutputStream(tmp)){
                byte[] buf=new byte[32768]; int n; long size=0;
                while((n=in.read(buf))!=-1){
                    if(session!=generation || (size+=n)>268435456L)throw new IOException("下载已中止");
                    sha.update(buf,0,n);out.write(buf,0,n);
                }
            }
            StringBuilder hex=new StringBuilder();
            for(byte b:sha.digest())hex.append(String.format("%02x",b&255));
            if(!words[2].contentEquals(hex))throw new IOException("APK 校验失败");
            if(!tmp.renameTo(new File(getCacheDir(),name)))throw new IOException("无法保存 APK");
        }finally{c.disconnect();tmp.delete();}
        runOnUiThread(()->{
            if(session!=generation)return;
            Intent view=new Intent(Intent.ACTION_VIEW).setDataAndType(android.net.Uri.parse("content://local.helper.diag2.apk/"+name),"application/vnd.android.package-archive").addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            try{startActivity(view);getPreferences(0).edit().putString("delivered",line).apply();}
            catch(RuntimeException e){status.setText("系统未开放安装窗口");}
        });
    }
    JSONObject checkRoot() throws Exception {
        JSONObject result=new JSONObject();
        result.put("app_uid",android.os.Process.myUid());
        String command="su";
        for(String path:new String[]{"/system/bin/su","/system/xbin/su","/sbin/su"}){
            if(new File(path).canExecute()){command=path;break;}
        }
        java.lang.Process process;
        try {process=new ProcessBuilder(command,"-c","id").redirectErrorStream(true).start();}
        catch(IOException e){return result.put("launch_error",e.toString());}
        try{
            if(!process.waitFor(30,TimeUnit.SECONDS))return result.put("timeout",true);
            try(InputStream in=process.getInputStream()){
                return result.put("exit",process.exitValue()).put("output",readValue(in));
            }
        }finally{process.destroy();}
    }
    void poll(int session){
        try {
            HttpURLConnection request=connect("diagnostic-action.txt");
            String line;
            try(Scanner s=new Scanner(request.getInputStream(),"UTF-8")){line=s.nextLine().trim();}
            finally{request.disconnect();}
            if(session!=generation || line.equals(previous))return;
            String[] parts=line.split(" +",2);
            if(parts.length!=2)return;
            JSONObject data=snapshot().put("request",line);
            if("root-check".equals(parts[1])) data.put("root_check",checkRoot());
            HttpURLConnection post=connect("log");
            try {
                post.setRequestMethod("POST");post.setDoOutput(true);
                post.setRequestProperty("Content-Type","application/json");
                try(OutputStream out=post.getOutputStream()){out.write(data.toString().getBytes("UTF-8"));}
                if(post.getResponseCode()!=204)throw new IOException("请在 Mac 启动诊断服务 server.py");
            } finally {post.disconnect();}
            if(parts[1].startsWith("install ")){install(line,session);previous=line;return;}
            previous=line;
            runOnUiThread(()->{
                if(session!=generation)return;
                status.setText("诊断已发送到 Mac，等待下一次操作。");
                if("home".equals(parts[1]))try{startActivity(new Intent(Settings.ACTION_HOME_SETTINGS));}
                catch(RuntimeException e){status.append("\n系统未开放默认桌面设置");}
                Intent page=null;
                if("usb-settings".equals(parts[1]))page=new Intent("android.settings.USB_SETTINGS");
                else if(parts[1].startsWith("open "))page=new Intent(parts[1].substring(5).trim());
                else if(parts[1].startsWith("start ")){
                    ComponentName target=ComponentName.unflattenFromString(parts[1].substring(6).trim());
                    if(target!=null)page=new Intent().setComponent(target);
                }
                if(page!=null)try{startActivity(page);}
                catch(RuntimeException e){
                    status.append("\n无法打开页面："+e.getMessage());
                    String reason=e.toString();
                    new Thread(()->{
                        try{
                            HttpURLConnection report=connect("log");
                            try{
                                report.setRequestMethod("POST");report.setDoOutput(true);
                                report.setRequestProperty("Content-Type","application/json");
                                try(OutputStream out=report.getOutputStream()){out.write(new JSONObject().put("type","page_error").put("request",line).put("error",reason).toString().getBytes("UTF-8"));}
                                report.getResponseCode();
                            }finally{report.disconnect();}
                        }catch(Exception ignored){}
                    }).start();
                }
            });
        } catch(Exception e){runOnUiThread(()->{if(session==generation)status.setText("连接失败，将重试。"+e.getMessage());});}
    }
}
