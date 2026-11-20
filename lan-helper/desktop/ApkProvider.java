package local.cloud.launcher;
import android.content.*;
import android.database.*;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import java.io.*;
public class ApkProvider extends ContentProvider {
    public boolean onCreate(){return true;}
    private File file(Uri uri){
        String name=uri.getLastPathSegment();
        if(name==null || !name.matches("[0-9a-f]{64}\\.apk"))throw new IllegalArgumentException();
        return new File(getContext().getCacheDir(),name);
    }
    public String getType(Uri u){return "application/vnd.android.package-archive";}
    public ParcelFileDescriptor openFile(Uri u,String mode)throws FileNotFoundException{
        if(!"r".equals(mode))throw new FileNotFoundException();
        return ParcelFileDescriptor.open(file(u),ParcelFileDescriptor.MODE_READ_ONLY);
    }
    public Cursor query(Uri u,String[] cols,String s,String[] args,String sort){
        if(cols==null)cols=new String[]{"_display_name","_size"};
        MatrixCursor cursor=new MatrixCursor(cols);
        MatrixCursor.RowBuilder row=cursor.newRow();
        for(String col:cols)row.add("_size".equals(col)?(Object)file(u).length():"_display_name".equals(col)?"package.apk":null);
        return cursor;
    }
    public Uri insert(Uri u,ContentValues v){throw new UnsupportedOperationException();}
    public int update(Uri u,ContentValues v,String s,String[] a){throw new UnsupportedOperationException();}
    public int delete(Uri u,String s,String[] a){throw new UnsupportedOperationException();}
}
