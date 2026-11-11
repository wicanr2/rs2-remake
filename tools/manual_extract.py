#!/usr/bin/env python3
"""在有 libarchive.so.13 的 Docker 內解開已釘雜湊的手冊至既有 workspace。

python3 tools/manual_extract.py 大宇資訊-RS2.rar workspace/manual
預設不覆寫；相同內容可重跑。原始 RAR 以唯讀掛載。
"""
import ctypes as c
import hashlib
import json
import os
import sys
from pathlib import Path

EXPECTED = 'a28beac84835d62d7d1f1161bfa4d1d13fc6386e66c481a1ddc6ef30fdb84c15'


def main():
    source, dest = map(Path, sys.argv[1:])
    assert hashlib.sha256(source.read_bytes()).hexdigest() == EXPECTED, 'RAR 雜湊不符'
    assert dest.parent.is_dir(), '先確認既有輸出根目錄'
    assert dest.parent.stat().st_uid == os.getuid(), '輸出根目錄擁有者不符'
    dest.mkdir(exist_ok=True)
    lib = c.CDLL('libarchive.so.13')
    signatures = [
        ('archive_read_new', c.c_void_p, []),
        ('archive_read_support_format_all', c.c_int, [c.c_void_p]),
        ('archive_read_support_filter_all', c.c_int, [c.c_void_p]),
        ('archive_read_open_filename', c.c_int, [c.c_void_p, c.c_char_p, c.c_size_t]),
        ('archive_read_next_header', c.c_int, [c.c_void_p, c.POINTER(c.c_void_p)]),
        ('archive_entry_pathname', c.c_char_p, [c.c_void_p]),
        ('archive_entry_size', c.c_longlong, [c.c_void_p]),
        ('archive_read_data', c.c_longlong, [c.c_void_p, c.c_void_p, c.c_size_t]),
        ('archive_read_free', c.c_int, [c.c_void_p]),
    ]
    for name, rest, args in signatures:
        f = getattr(lib, name)
        f.restype, f.argtypes = rest, args
    a = lib.archive_read_new()
    rows = []
    seen = set()
    try:
        lib.archive_read_support_format_all(a)
        lib.archive_read_support_filter_all(a)
        assert lib.archive_read_open_filename(a, os.fsencode(source), 65536) == 0
        entry = c.c_void_p()
        while True:
            rc = lib.archive_read_next_header(a, c.byref(entry))
            if rc == 1:
                break
            assert rc == 0, f'解壓錯誤 {rc}'
            name = lib.archive_entry_pathname(entry).decode('utf-8')
            size = lib.archive_entry_size(entry)
            if not size:
                continue
            assert 0 < size < 20_000_000
            assert '..' not in Path(name).parts and not Path(name).is_absolute()
            leaf = Path(name).name
            assert leaf not in seen
            seen.add(leaf)
            data = bytearray()
            buf = c.create_string_buffer(65536)
            while True:
                n = lib.archive_read_data(a, buf, len(buf))
                assert n >= 0, '讀取解壓資料失敗'
                if n == 0:
                    break
                data.extend(buf.raw[:n])
                assert len(data) <= size
            assert len(data) == size
            out = dest / leaf
            assert not out.is_symlink()
            if out.exists():
                assert out.read_bytes() == data, f'拒絕覆寫不同內容：{out}'
            else:
                out.write_bytes(data)
            rows.append(dict(source=name, sha256=hashlib.sha256(data).hexdigest(), size=size))
    finally:
        lib.archive_read_free(a)
    assert len(rows) == 10 and len([r for r in rows if r['source'].endswith('.jpg')]) == 8
    manifest = dict(archive=source.name, sha256=EXPECTED, entries=rows)
    (dest / 'manifest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n')
    print(f'已驗證 {len(rows)} 個檔案，8 張掃描圖：{dest}')


if __name__ == '__main__':
    main()
