#!/usr/bin/env python3
"""由 tools/rust.sh play-poster-check 在 Xvfb 容器內執行，驗規格 007。

從預設標題開始，用真實視窗按鍵切換海報，驗設定寫入、重開與固定輸入的模擬一致性。
截圖來自實際 SDL 視窗，沒有合成海報或修改遊戲狀態。
"""
import hashlib
import json
import os
import subprocess as sp
import sys
import time
from pathlib import Path


def run(args, **kw):
    return sp.run(args, check=True, timeout=kw.pop('timeout', 25), **kw)


def wait_for(test, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if test():
            return
        time.sleep(.1)
    raise AssertionError('等待 GUI 條件逾時')


def keys(win, *seq):
    run(['xdotool', 'key', '--window', win, '--clearmodifiers', '--delay', '100', *seq])
    time.sleep(.25)


def geometry(win):
    raw = sp.check_output(['xdotool', 'getwindowgeometry', '--shell', win], text=True)
    return {k: int(v) for k, v in (line.split('=', 1) for line in raw.splitlines())}


def capture(win, out):
    g = geometry(win)
    run(['ffmpeg', '-nostdin', '-hide_banner', '-loglevel', 'error', '-y', '-f', 'x11grab',
         '-draw_mouse', '0', '-video_size', f"{g['WIDTH']}x{g['HEIGHT']}",
         '-i', f"{os.environ['DISPLAY']}+{g['X']},{g['Y']}", '-frames:v', '1', '-threads', '1', str(out)])
    return g


def main():
    binary, out = Path(sys.argv[1]), Path(sys.argv[2])
    assert out.parent.is_dir() and out.parent.stat().st_uid == os.getuid()
    out.mkdir(exist_ok=True)
    cfg = out / 'config'
    assert not cfg.exists(), '使用新輸出目錄，避免重用上次設定與收據'
    base = [str(binary), '--orig', '/orig', '--hd', '/hd', '--config', str(cfg), '--no-audio']
    receipts = {}
    record = out / 'normal-input.jsonl'

    def gui(name, secs, callback, extra=()):
        logfile = out / f'{name}.log'
        with logfile.open('w') as log:
            proc = sp.Popen([*base, '--max-secs', str(secs), *extra], stdout=log, stderr=log)
            try:
                wait_for(lambda: '[hd] 預載' in logfile.read_text(), 20)
                win = sp.check_output(['xdotool', 'search', '--onlyvisible', '--name', '^RS-2 remake$'], text=True).splitlines()[-1]
                run(['xdotool', 'windowfocus', '--sync', win])
                time.sleep(.4)
                callback(win)
                assert proc.wait(timeout=secs + 5) == 0, logfile.read_text()[-2000:]
            finally:
                if proc.poll() is None:
                    proc.terminate()
                    try:
                        proc.wait(timeout=3)
                    except sp.TimeoutExpired:
                        proc.kill()
                        proc.wait()

    def first(win):
        assert geometry(win)['WIDTH'] == 960, '首次啟動預設關閉'
        keys(win, 'F10', 'F8')
        keys(win, *(['Tab'] * 5), 'Return')
        settings = cfg / 'settings.txt'
        wait_for(lambda: settings.exists() and 'poster=classic\n' in settings.read_text())
        receipts['classic-settings'] = capture(win, out / 'classic-settings.png')
        keys(win, 'Escape')
        # 開場初始化會清掉早到的按鍵；用一般玩家的重按行為跨過各等待點。
        for _ in range(8):
            keys(win, 'a')
            time.sleep(.5)
        time.sleep(4)
        receipts['classic-game'] = capture(win, out / 'classic-game.png')
        keys(win, 'F10')
        keys(win, *(['Tab'] * 5), 'Right')
        wait_for(lambda: 'poster=modern\n' in settings.read_text())
        receipts['modern-settings'] = capture(win, out / 'modern-settings.png')
        keys(win, 'Escape')
        time.sleep(5)
        receipts['modern-game'] = capture(win, out / 'modern-game.png')

    gui('normal', 30, first, ('--record', str(record)))
    assert 'poster=modern\n' in (cfg / 'settings.txt').read_text()

    def reopen(win):
        g = geometry(win)
        assert g['WIDTH'] == 1536, '重開須從保存的海報設定建立視窗'
        receipts['reopened-modern'] = capture(win, out / 'reopened-modern.png')

    gui('reopened', 4, reopen)
    rows = record.read_text().splitlines()
    assert rows and any(json.loads(r).get('sc') for r in rows), '必須記錄到正常操作的按鍵'
    hashes = {}
    for mode, display in [('off', 'vga'), ('classic', 'vga'), ('modern', 'vga'), ('modern', 'cga'), ('modern', 'mga')]:
        name = f'{mode}-{display}'
        h = out / f'{name}.hashes'
        env = dict(os.environ, SDL_VIDEODRIVER='dummy', SDL_RENDER_DRIVER='software')
        with (out / f'{name}.log').open('w') as log:
            run([*base, '--poster', mode, '--mode', display, '--replay', str(record),
                 '--headless-steps', str(len(rows)), '--hash-out', str(h),
                 '--shot-at', f'{len(rows)}:{out / (name + ".png")}'], env=env, stdout=log, stderr=log, timeout=60)
        hashes[name] = h.read_bytes()
        assert hashes[name] == hashes['off-vga'], f'{name} 改變模擬'

    receipts['summary'] = {'steps': len(rows), 'normal_input_sha256': hashlib.sha256(record.read_bytes()).hexdigest(),
                           'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                           'same_simulation': list(hashes), 'first_launch': 'off', 'saved_and_reopened': 'modern'}
    receipts['artifacts'] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(out.glob('*.png'))}
    (out / 'receipt.json').write_text(json.dumps(receipts, ensure_ascii=False, indent=2) + '\n')
    print('PASS: 正常 GUI 切換、兩組截圖、重開保存、五條顯示路徑同輸入模擬一致')


if __name__ == '__main__':
    main()
