#!/usr/bin/env python3
"""匯出解包後 S.EXE 內的關卡時間表與實體樣板，供 remake 規格引用。

用法：export_tables.py <S.unpacked.bin> <輸出 json>
時間表格式對照 sub_15D1C（15D1Ch）：(u16 幀, u16 樣板位移) 重複，幀為 0 結束。
樣板長度對照 sub_15D41（15D41h）：rep movsw cx=18h，共 48 位元組。
"""
import hashlib
import json
import struct
import sys

img = open(sys.argv[1], "rb").read()
DS = 0xCBC0
TABLES = {"stage1": (0x41BE, "sub_17D82"), "stage2": (0x54FE, "sub_17D8E"), "stage3": (0x68E0, "sub_17D9A"),
          "stage4_a": (0x7922, "sub_17DA6"), "stage4_b": (0x8D1C, "sub_17DB2")}


def u16(o):
    return struct.unpack_from("<H", img, DS + o)[0]


out = {"input_sha256": hashlib.sha256(img).hexdigest(), "timelines": {}, "templates": {}, "stage1_group_table": []}
seen = set()
for name, (start, func) in TABLES.items():
    rows = []
    p = start
    while True:
        f = u16(p)
        if f == 0:
            break
        t = u16(p + 2)
        rows.append([f, t])
        seen.add(t)
        p += 4
        if len(rows) > 2000:
            raise SystemExit(f"{name} 超過 2000 列仍未結束")
    out["timelines"][name] = {"ds_offset": hex(start), "reader": func, "entries": rows, "end_ds_offset": hex(p)}
for t in sorted(seen):
    out["templates"][hex(t)] = list(struct.unpack_from("<24H", img, DS + t))
p = 0x19DC
while True:
    f = u16(p)
    if f == 0:
        break
    out["stage1_group_table"].append([f, u16(p + 2)])
    p += 4
json.dump(out, open(sys.argv[2], "w"), indent=0)
for k, v in out["timelines"].items():
    fr = [e[0] for e in v["entries"]]
    print(k, v["ds_offset"], "列數", len(fr), "幀範圍", min(fr), max(fr), "樣板種類", len({e[1] for e in v["entries"]}))
print("templates", len(out["templates"]), "group", len(out["stage1_group_table"]))
