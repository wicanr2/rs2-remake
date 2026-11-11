#!/usr/bin/env python3
"""在 rs2-hdtools:r1 內重建定案海報及版面參考圖。

python3 tools/poster_assets.py workspace/hd docs/images/game tools/hd-styles [workspace/rs2/poster-verification]
原稿只讀；輸出為 HD 根目錄的 assets/posters/、定案預覽及提示詞。
原始來源、權利與驗收見 docs/spec/007-manual-posters.md。
"""
import hashlib
import io
import json
import os
import sys
from pathlib import Path
from PIL import Image

MASTERS = {
    'classic': 'e9ddd7dd7e4a957c9beac27e12ee76138366b40d805c6c1713884e2fdcdf020c',
    'modern': '760f123f40a39be29823f1a2d31684ecd9d9412de3c7002e9eba0c348f96e77b',
}
PANELS = {
    'classic-left': '05239e1b53226d7d7f3d681840fac07daab332081887945cb326e3ccc896bf91',
    'classic-right': 'e15a016a891f4f5c59af1a636e20564b12aada5d60744cc2709d4bf49cfa9699',
    'modern-left': '363e3f8f6daab22833c18cfd534eb0b46686df952f23af6fad1f6758f7746482',
    'modern-right': '163add451f5779ea468681492d1dace5001a3da4a42c41c1b31781848052a2b4',
}


def write(path, data, replace=False):
    assert path.parent.is_dir() and path.parent.stat().st_uid == os.getuid()
    assert not path.is_symlink()
    if path.exists():
        assert path.stat().st_uid == os.getuid()
        assert replace or path.read_bytes() == data, f'拒絕改變定案檔案：{path}'
        if replace:
            path.write_bytes(data)
    else:
        path.write_bytes(data)


def main():
    hd, previews, prompts = map(Path, sys.argv[1:4])
    out = hd / 'assets' / 'posters'
    assert out.parent.is_dir() and out.parent.stat().st_uid == os.getuid()
    out.mkdir(exist_ok=True)
    for style, digest in MASTERS.items():
        batch = hd / f'poster-{style}-r1'
        master = (batch / 'master.png').read_bytes()
        assert hashlib.sha256(master).hexdigest() == digest
        im = Image.open(io.BytesIO(master)).convert('RGBA')
        assert im.size == (1024, 1536)
        for side, x in [('left', 0), ('right', 512)]:
            panel = im.crop((x, 0, x + 512, 1536))
            assert panel.getchannel('A').getextrema() == (255, 255)
            buf = io.BytesIO()
            panel.save(buf, format='PNG')
            data = buf.getvalue()
            key = f'{style}-{side}'
            assert hashlib.sha256(data).hexdigest() == PANELS[key]
            write(out / f'{key}.png', data)
        write(previews / f'poster-approved-{style}.png', (batch / 'game-preview.png').read_bytes())
        write(prompts / f'poster-{style}.txt', (batch / 'prompt.txt').read_bytes())
    print('PASS: 四張定案海報、兩張原始版面預覽及兩份提示詞')
    if len(sys.argv) == 5:
        verification = Path(sys.argv[4])
        receipt = json.loads((verification / 'receipt.json').read_text())
        for name, target in [('classic-game', 'poster-classic-window'), ('modern-game', 'poster-modern-window'), ('modern-settings', 'poster-settings')]:
            p = verification / f'{name}.png'
            assert hashlib.sha256(p.read_bytes()).hexdigest() == receipt['artifacts'][p.name]
            im = Image.open(p).convert('RGB')
            buf = io.BytesIO()
            im.save(buf, format='PNG', optimize=True)
            data = buf.getvalue()
            assert Image.open(io.BytesIO(data)).tobytes() == im.tobytes()
            write(previews / f'{target}.png', data, replace=True)
            print(target, im.size, hashlib.sha256(data).hexdigest())


if __name__ == '__main__':
    main()
