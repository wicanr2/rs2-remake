#!/usr/bin/env python3
"""在 rs2-hdtools:r1 內產生私人 repo 的手冊插圖。

python3 tools/manual_images.py workspace/manual docs/images/manual
只讀已核對的八張 JPEG；第 5 張只擷取左頁，密碼表永遠不輸出。
售後服務另以文字整理於 docs/manual.md。
"""
import hashlib
import json
import os
import sys
from pathlib import Path
from PIL import Image

NAMES = ('packaging', 'cover-spread', 'contents-story', 'setup-controls',
         'player2-weapons', 'disk-a', 'disk-b', 'disk-c')


def main():
    source, dest = map(Path, sys.argv[1:])
    assert dest.parent.is_dir() and dest.parent.stat().st_uid == os.getuid()
    dest.mkdir(exist_ok=True)
    manifest = json.loads((source / 'manifest.json').read_text())
    assert manifest['sha256'] == 'a28beac84835d62d7d1f1161bfa4d1d13fc6386e66c481a1ddc6ef30fdb84c15'
    rows = [r for r in manifest['entries'] if r['source'].endswith('.jpg')]
    assert len(rows) == 8
    for i, (row, name) in enumerate(zip(rows, NAMES), 1):
        p = source / Path(row['source']).name
        assert hashlib.sha256(p.read_bytes()).hexdigest() == row['sha256']
        im = Image.open(p).convert('RGB')
        if i == 5:
            # 中縫 x 約 1068；留白內縮至 x=1050，右頁任何像素都不保留。
            assert im.size == (2137, 1581)
            im = im.crop((0, 0, 1050, 1581))
        im.thumbnail((1600, 1800), Image.Resampling.LANCZOS)
        out = dest / f'{name}.png'
        assert not out.is_symlink()
        if out.exists():
            assert out.stat().st_uid == os.getuid()
        im.save(out, optimize=True)
        print(i, out.name, im.size, hashlib.sha256(out.read_bytes()).hexdigest())


if __name__ == '__main__':
    main()
