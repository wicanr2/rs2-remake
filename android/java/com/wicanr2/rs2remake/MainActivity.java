package com.wicanr2.rs2remake;

import org.libsdl.app.SDLActivity;

/** 遊戲本體都在 libmain.so（crates/rs2-android），這裡只指定要載入的函式庫。 */
public class MainActivity extends SDLActivity {
    @Override
    protected String[] getLibraries() {
        return new String[] {"SDL2", "main"};
    }
}
