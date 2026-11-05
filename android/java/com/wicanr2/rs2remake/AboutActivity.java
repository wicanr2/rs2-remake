package com.wicanr2.rs2remake;

import android.app.Activity;
import android.os.Bundle;
import android.widget.ScrollView;
import android.widget.TextView;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;

/** 顯示發行包內的授權與第三方聲明全文（assets/LICENSE、assets/THIRD-PARTY-NOTICES.txt、assets/SDL2-LICENSE.txt）。 */
public class AboutActivity extends Activity {
    private String read(String name) {
        try (InputStream in = getAssets().open(name)) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] buf = new byte[65536];
            int n;
            while ((n = in.read(buf)) > 0) {
                out.write(buf, 0, n);
            }
            return out.toString("UTF-8");
        } catch (IOException e) {
            return "（讀不到 " + name + "：" + e.getMessage() + "）";
        }
    }

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        TextView text = new TextView(this);
        int pad = (int) (16 * getResources().getDisplayMetrics().density);
        text.setPadding(pad, pad, pad, pad);
        text.setTextIsSelectable(true);
        text.setText(
            "RS-2 重製\n"
                + "原作：大宇資訊（Softstar），1990 年。原版的程式、美術與音樂屬於原權利人，不在本授權範圍內。\n\n"
                + read("LICENSE")
                + "\n\n----- 第三方元件 -----\n\n"
                + read("THIRD-PARTY-NOTICES.txt")
                + "\n\n----- SDL2 -----\n\n"
                + read("SDL2-LICENSE.txt"));
        ScrollView scroll = new ScrollView(this);
        scroll.addView(text);
        setContentView(scroll);
    }
}
