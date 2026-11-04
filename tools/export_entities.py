#!/usr/bin/env python3
"""匯出解包後 S.EXE 的實體、移動腳本、武器、子彈與特效資料，供 remake 規格引用。

用法：export_entities.py <S.unpacked.bin> <輸出 json>
只用標準函式庫。任何結構、雜湊或位元組簽章不符就中止（fail-closed）。

容器內執行（輸入唯讀，只開放輸出目錄寫入）：
  docker run --rm --name rs2-ent-export --network none -u "$(id -u):$(id -g)" --memory 1g --cpus 2 \
    --pids-limit 128 --log-opt max-size=10m --log-opt max-file=3 \
    -v "$PWD/workspace/unpacked:/in:ro" -v "$PWD/tools:/tools:ro" -v "$PWD/workspace/extracted:/out" \
    python:3.12-slim timeout 120 python -I /tools/export_entities.py /in/S.unpacked.bin /out/entities.json

位址慣例：
  線性位址 L：IDA 映像位址，解包映像位移 = L - 0x10000。
  dseg 偏移 X：線性位址 = 0x1CBC0 + X，解包映像位移 = 0xCBC0 + X。
  JSON 內位址欄位以 "ds:XXXX" 表示 dseg 偏移，"L:XXXXX" 表示線性位址。

資料格式與語意見 docs/re/11-entities-combat.md，每項的位址證據也在該文件。
語意表（欄位名稱、派發表）由人讀反組譯後寫在本檔，並在下列兩處對位元組驗證：
  1. SIGNATURES：每個依賴的函式開頭位元組必須在映像中恰好出現一次，且位於預期位址。
  2. cross_check_*：派發表裡的樣板位址必須和函式機器碼內的立即值序列一致。
"""
import hashlib
import json
import re
import struct
import sys

EXPECTED_SHA256 = "8475b41b45507ab10f0430238e5d20a11563b7d1b202997680f73a7cab34de72"
DS = 0xCBC0          # dseg:0 在解包映像內的位移
CODE = 0x10000       # 線性位址 - CODE = 解包映像位移

SIGNATURES = [
    ("spawn_template_15D41", 0x15D41, "bb0000bab422b90f008b"),
    ("entity_copy_in_15E1A", 0x15E1A, "a3b4258bf0b8bc0c8ed8"),
    ("entity_update_15E68", 0x15E68, "e82201833eea21597427"),
    ("script_ctl_15ECD", 0x15ECD, "3cf074053ce07413c346"),
    ("frame_select_15F4C", 0x15F4C, "8b36b8252bdb8a1e081b"),
    ("damage_resolve_15F8D", 0x15F8D, "c706f61a4e00c706ea21"),
    ("move_dispatch_15FD9", 0x15FD9, "c706ea214e00e888003c"),
    ("enemy_fire_1617A", 0x1617A, "a1e225a32a04e821af8b"),
    ("pbullet_spawn_16578", 0x16578, "bb0000ba2f26b914008b"),
    ("weapon_pattern_165CF", 0x165CF, "a1fc25833efe253e7404"),
    ("weapon_pattern_left_1669F", 0x1669F, "3d4b0074123d4c00741a"),
    ("pbullet_damage_16946", 0x16946, "a000263cff7429fec82a"),
    ("pbullet_vs_grid_16B49", 0x16B49, "c606002600908b36cd28"),
    ("ebullet_dispatch_16D3B", 0x16D3B, "3d4600741c3d6c007421"),
    ("aim_dir_16F9F", 0x16F9F, "e83c03a1af322d03003d"),
    ("effect_dispatch_17A3A", 0x17A3A, "833eb83f53742d833eb8"),
    ("player_update_14501", 0x14501, "f6061d04017501c3c706"),
    ("pickup_apply_14D04", 0x14D04, "a0e80e3c4e741e803ecd"),
    ("pickup_classify_14CD4", 0x14CD4, "8ae080e4f080fc80741b80fc40741680fcc0741180fc20740c80fc407407c606000f"),
    ("player_vs_ebullet_14C85", 0x14C85, "b8bc0c8ed88ec0c606e8"),
    ("rand_110A4", 0x110A4, "a12a04403d4b007203b8"),
    ("entity_death_15DBE", 0x15DBE, "833eea21597538833ec4"),
    ("target_select_172DE", 0x172DE, "f6061d04017425f6061d"),
    ("ebullet_move_17631", 0x17631, "e88100a2122cc706b132"),
    ("ground_follow_1606A", 0x1606A, "3c0d7401c3a1c8250503"),
    ("track_row_1609C", 0x1609C, "833eca2518770da1ca25"),
    ("effects_tick_17C9A", 0x17C9A, "b91e00bb000051538b87"),
    ("ebullets_tick_17361", 0x17361, "bb0000b91400c6061b2c"),
    ("pbullets_tick_1673E", 0x1673E, "bb0000b91400518b8707"),
    ("laser_right_16A4E", 0x16A4E, "b8bc0c8ec08ed8c6060026ff908b1ec528e86402"),
    ("seek_dir_16BA8", 0x16BA8, "b8bc0c8ec08ed88b1ec528e8"),
    ("ebullet_grid_write_17855", 0x17855, "8b365135ada3be9cad8bc8b8"),
    ("entity_grid_write_16233", 0x16233, "b8bc0c8ec08ed88b36d225ada3be9cad8bc88b1eca25e844"),
    ("pbullet_dispatch_167A3", 0x167A3, "833eb1284c7423833eb1"),
    ("homing_turn_173FE", 0x173FE, "3d01007504e84400c33d"),
    ("player_vs_entity_14B1E", 0x14B1E, "be0d0fada3be9cad8bc8b8bc"),
    ("pod_vs_entity_14B63", 0x14B63, "c606ca0e0090be160fad"),
    ("pod_vs_ebullet_14BAE", 0x14BAE, "b8bc0c8ed88ec0be0d0f"),
    ("pod_damage_14C18", 0x14C18, "a0ca0e3cff741d3c0074"),
    ("timeline_15D1C", 0x15D1C, "8b043d0000741d3b0622"),
    ("death_step_14679", 0x14679, "fe0e030f803e030f0074"),
    ("player_move_up_146AC", 0x146AC, "833e090f007e0fff0e09"),
    ("player_move_right_146DA", 0x146DA, "833e0b0f21731083060b"),
    ("pod_fire_1480A", 0x1480A, "803ecd0e597401c3a1d8"),
    ("ebullet_spawn_16CEA", 0x16CEA, "bb0000badb32b914008b"),
    ("hit_counter_16C7D", 0x16C7D, "535157833ebf28317424"),
]

TIMELINES = {  # 與 tools/export_tables.py 相同的起點；格式 (u16 幀, u16 樣板位移)，幀 0 結束
    "stage1": (0x41BE, "sub_17D82", "SHT1"),
    "stage2": (0x54FE, "sub_17D8E", "SHT2"),
    "stage3": (0x68E0, "sub_17D9A", "SHT3"),
    "stage4_a": (0x7922, "sub_17DA6", "SHT4"),
    "stage4_b": (0x8D1C, "sub_17DB2", "SHT4"),
}

ENTITY_WORDS = [  # (字組索引, 名稱, 說明, 等級)。偏移 = 索引 * 2；工作區 = ds:25B6 + 偏移
    (0, "hp", "耐久門檻。累計傷害 > hp + 難度修正 才死亡；999 為不可摧毀", "靜態已證實"),
    (1, "frame_tbl", "9 字組圖框表指標，以移動碼 0..8 索引；kind=K 時指向 (圖,x,y) 三字組", "靜態已證實"),
    (2, "x_px", "繪製 x（位元組欄）。生成時 = x，恆等於 x", "靜態已證實"),
    (3, "y_px", "繪製 y 像素。生成時 = y 列 * 10", "靜態已證實"),
    (4, "dx_px", "水平單步（繪製座標）", "靜態已證實"),
    (5, "dy_px", "垂直單步（繪製座標，像素）", "靜態已證實"),
    (6, "fire_thr", "開火門檻；0x63 永不開火，否則 亂數+難度 > 門檻 即開火", "靜態已證實"),
    (7, "bullet", "開火時交給 sub_16D3B 的子彈種類字母；'E' 另表示過關標記", "靜態已證實"),
    (8, "mode", "'N' 腳本；'Y' 腳本外另隨捲動左移 1 欄；'A' 追蹤玩家（w13 改作朝向）", "靜態已證實"),
    (9, "x", "邏輯 x（欄，0..39，可為負）", "靜態已證實"),
    (10, "y", "邏輯 y（列，0..19，可為負）", "靜態已證實"),
    (11, "dx", "水平單步（邏輯座標，欄）", "靜態已證實"),
    (12, "dy", "垂直單步（邏輯座標，列）", "靜態已證實"),
    (13, "script", "移動腳本指標（執行中遞增）；mode='A' 時為目前朝向 1..8", "靜態已證實"),
    (14, "shape", "碰撞形狀指標：u16 寬, u16 高, 寬*高位元組", "靜態已證實"),
    (15, "unused", "全部樣板皆為 1，程式碼不讀", "靜態已證實（無讀取端）"),
    (16, "damage", "低位元組=累計傷害，高位元組=1 表本幀被命中", "靜態已證實"),
    (17, "explosion", "死亡爆炸特效字母（sub_17A3A）", "靜態已證實"),
    (18, "kind", "'O' 一般；'N' 不畫；'K' 嵌在背景的目標；'S' 畫在玩家上層；'P' 不在此欄", "靜態已證實"),
    (19, "carrier", "'P' 死亡時掉出 w7 字母的道具；其他值無作用", "靜態已證實"),
    (20, "limit_tbl", "9 字組動畫上限表指標，0 表示無動畫", "靜態已證實"),
    (21, "anim", "動畫計數，執行時寫入，樣板恆為 0", "靜態已證實"),
    (22, "rand_idx", "此實體的亂數表索引，樣板恆為 0", "靜態已證實"),
    (23, "spill", "第 24 個字組。樣板只有 23 字組（46 位元組），複製 24 字組會多帶下一個樣板的 w0；程式碼不讀", "靜態已證實（無讀取端）"),
]

MOVE_CODES = [  # 對照 sub_15FD9 / sub_1606A / sub_1609C / sub_16132 / 15E68
    (0x00, "停留", "不移動，圖框取表項 0"),
    (0x01, "N", "上：y_px -= dy_px; y -= dy"),
    (0x02, "NE", "上 + 右"),
    (0x03, "E", "右：x_px += dx_px; x += dx"),
    (0x04, "SE", "下 + 右"),
    (0x05, "S", "下：y_px += dy_px; y += dy"),
    (0x06, "SW", "下 + 左"),
    (0x07, "W", "左：x_px -= dx_px; x -= dx"),
    (0x08, "NW", "左 + 上"),
    (0x09, "追列", "y<=24 且 y>=玩家1列 → 上，否則下（只看玩家 1）"),
    (0x0A, "右跳 6", "x_px += 6; x += 6"),
    (0x0B, "發射 B", "在 (x, y+2) 以字母 'B' 呼叫 sub_16D3B"),
    (0x0C, "無作用", "不在派發表內"),
    (0x0D, "追玩家列", "把 (x+3, y+4) 當發射點求朝向；朝向在 4..6（玩家在下）→ 下，否則 → 上"),
    (0x0E, "無作用", "不在派發表內"),
    (0x0F, "死亡", "word_1EDAA='Y'，不播爆炸"),
    (0xE0, "變身", "E0 + u16 樣板位址：載入新樣板，保留目前 x、y、w17"),
    (0xF0, "跳轉", "F0 + u16 位址：腳本指標改到該位址"),
]

# 敵方子彈派發 sub_16D3B：字母 -> (需要的瞄準函式, 展開的樣板位址列表, x 修正, y 修正)
EBULLET_DISPATCH = {
    "F": {"aim": "sub_1701E", "family": [0x3557, 0x3575, 0x3593, 0x35B1, 0x35CF, 0x35ED, 0x360B, 0x3629], "dx": 0, "dy": 0, "note": "八向瞄準基本彈"},
    "l": {"aim": "sub_170B0", "family": [0x3737, 0x3755, 0x3773, 0x3791, 0x37AF, 0x37CD, 0x37EB, 0x3809], "dx": 0, "dy": 0, "note": "L 道具球（八向）"},
    "b": {"aim": "sub_17117", "family": [0x3827, 0x3845, 0x3863, 0x3881, 0x389F, 0x38BD, 0x38DB, 0x38F9], "dx": 0, "dy": 0, "note": "B 道具球（八向）"},
    "k": {"aim": "sub_1717E", "family": [0x3917, 0x3935, 0x3953, 0x3971, 0x398F, 0x39AD, 0x39CB, 0x39E9], "dx": 0, "dy": 0, "note": "K 道具球（八向）"},
    "s": {"aim": "sub_171E5", "family": [0x3A07, 0x3A25, 0x3A43, 0x3A61, 0x3A7F, 0x3A9D, 0x3ABB, 0x3AD9], "dx": 0, "dy": 0, "note": "S 道具球（八向）"},
    "A": {"aim": None, "templates": [0x3AF7], "dx": 0, "dy": 0, "note": "無樣板引用；腳本指標 0007 指向資料段開頭，視為未使用"},
    "B": {"aim": "sub_17085", "family": {8: 0x3B15, 7: 0x3B33, 6: 0x3B51}, "default": 0x3B33, "dx": 0, "dy": 1, "note": "高速向左彈，帶 E 尾跡；只分三種朝向"},
    "L": {"aim": None, "templates": [0x3B6F, 0x3B8D, 0x3BAB, 0x3BC9, 0x3BE7], "dx": 8, "dy": 0, "note": "五發齊射（W、WNW、SSW、N、S 腳本）"},
    "V": {"aim": None, "templates": [0x3C05], "dx": -12, "dy": 0, "note": "單幀寬 12 列 2 的爆風區"},
    "1": {"aim": None, "templates": [0x3C23], "dx": 4, "dy": -2, "note": "單幀爆風區（右側）"},
    "2": {"aim": None, "templates": [0x3C23], "dx": 3, "dy": -2, "note": "單幀爆風區（左側）"},
    "X": {"aim": None, "templates": [0x3C41, 0x3CF5, 0x3D13], "dx": 0, "dy": 0, "note": "三向：N、W、WNW"},
    "C": {"aim": None, "templates": [0x3C5F, 0x3C7D, 0x3C9B], "dx": 0, "dy": 0, "note": "三向：ENE、E、ESE"},
    "<": {"aim": None, "templates": [0x3C41, 0x3C5F, 0x3C7D, 0x3C9B, 0x3CB9, 0x3CD7, 0x3CF5, 0x3D13], "dx": 0, "dy": 2, "note": "八向放射"},
    "?": {"aim": None, "templates": [0x3C41, 0x3C5F, 0x3C7D, 0x3C9B, 0x3CB9, 0x3CD7, 0x3CF5, 0x3D13], "dx": 0, "dy": 6, "note": "八向放射（先 y+4 再落入 '<' 的 y+2）"},
    "Q": {"aim": None, "templates": [0x3D8B, 0x3DA9], "dx": 4, "dy": 0, "note": "兩顆邊界反彈球；第二顆再 x+4"},
    "U": {"aim": None, "templates": [0x3D13, 0x3C5F], "dx": 0, "dy": 0, "note": "WNW + ENE"},
    "D": {"aim": None, "templates": [0x3C9B, 0x3CD7], "dx": 0, "dy": 0, "note": "ESE + WSW"},
    "E": {"aim": "sub_1701E", "templates": [0x3D6D, 0x3D4F, 0x3D31], "dx": 0, "dy": 0, "note": "三顆慢彈（WNW、W、WSW，每步 2 欄），再接六發瞄準彈：(x,y)、(x+4,y-8)、之後每發 y+3，共 5 發在 x+4"},
}
EBULLET_DEFAULT = 0x3557

# 玩家武器派發 sub_165CF（朝右）與 sub_1669F（朝左）
WEAPON_TABLE = {
    "none": {"code": 0, "right": [0x28D5], "left": [0x2A15], "dy": 1},
    "K": {"code": 0x4B, "right": [0x2935], "left": [0x2A75], "dy": 0},
    "S": {"code": 0x53, "right": [0x2955, 0x2975], "left": [0x2A95, 0x2AB5], "dy": 1},
    "B": {"code": 0x42, "right": [0x2995, 0x29B5], "left": [0x2AD5, 0x2AF5], "dy": 1, "cap": "word_1F1A8 <= 5 才發，發後 +2"},
    "L": {"code": 0x4C, "right": [0x29D5], "left": [0x29F5], "dy": 1, "dx": "右 +3、左 -2"},
    "F": {"code": 0x46, "right": [0x2B35] * 6, "left": [0x2B35] * 6, "dy": "y 由 1 起，每發 +3"},
}

EFFECT_LISTS = {  # sub_17A3A 字母 -> 動畫表（每筆 8 位元組：圖, dx, dy, 形狀指標；圖 0x0F 結束）
    "S": [0x4094], "B": [0x40D0], "V": [0x40EE, 0x40EE], "E": [0x413C], "A": [0x4162], "G": [0x4188],
    ">": [0x4094], "<": [0x40B2],
}
MASK_NAMES = {0x4094: "S/>", 0x40B2: "<", 0x40D0: "B", 0x40EE: "V", 0x413C: "E", 0x4162: "A", 0x4188: "G"}


def die(msg):
    sys.exit("export_entities: " + msg)


class Img:
    def __init__(self, data):
        self.b = data

    def u8(self, ds):
        return self.b[DS + ds]

    def u16(self, ds):
        return struct.unpack_from("<H", self.b, DS + ds)[0]

    def i16(self, ds):
        return struct.unpack_from("<h", self.b, DS + ds)[0]

    def code(self, lin, n):
        return self.b[lin - CODE:lin - CODE + n]


def hx(v):
    return f"ds:{v:04X}"


def chr_or_none(v):
    return chr(v) if 32 <= v < 127 else None


def verify_signatures(im):
    out = []
    for name, addr, hexsig in SIGNATURES:
        sig = bytes.fromhex(hexsig)
        n = im.b.count(sig)
        if n != 1:
            die(f"簽章 {name} 在映像中出現 {n} 次，應為 1")
        pos = im.b.find(sig)
        if pos != addr - CODE:
            die(f"簽章 {name} 位於 {pos + CODE:X}，預期 {addr:X}")
        out.append({"name": name, "linear": f"L:{addr:X}", "bytes": hexsig})
    return out


def imm_scan(im, start, end, prefix):
    """掃描 [start,end) 內 prefix + u16 立即值 的所有出現，回傳立即值（依位址順序）。"""
    seg = im.code(start, end - start)
    res = []
    i = 0
    while True:
        j = seg.find(prefix, i)
        if j < 0:
            break
        res.append(struct.unpack_from("<H", seg, j + len(prefix))[0])
        i = j + 1
    return res


def read_shape(im, p):
    w, h = im.u16(p), im.u16(p + 2)
    if not (1 <= w <= 80 and 1 <= h <= 24):
        die(f"形狀 {p:04X} 尺寸異常 {w}x{h}")
    cells = bytes(im.u8(p + 4 + i) for i in range(w * h))
    return {"ptr": hx(p), "w": w, "h": h, "cells_hex": cells.hex(), "solid": cells.count(1), "armor_FE": cells.count(0xFE)}


def read_words(im, p, n):
    return [im.u16(p + 2 * i) for i in range(n)]


def disasm_entity_script(im, p, limit=4000):
    """線性列出實體腳本。F0（跳轉）、E0（變身）、0F（死亡）為終點。"""
    ops, i = [], 0
    while i < limit:
        b = im.u8(p + i)
        if b == 0xF0:
            ops.append(["J", hx(im.u16(p + i + 1))])
            i += 3
            break
        if b == 0xE0:
            ops.append(["M", hx(im.u16(p + i + 1))])
            i += 3
            break
        if b == 0x0F:
            ops.append(["END"])
            i += 1
            break
        if b > 0x0F:
            # 0x10..0xDF、0xE1..0xEF、0xF1..0xFF 不在派發表內，執行時等同不移動；列表到此為止
            ops.append(["STOP_UNDEFINED", f"{b:02X}"])
            break
        ops.append(b)
        i += 1
    else:
        die(f"實體腳本 {p:04X} 超過 {limit} 位元組仍未結束")
    return ops, i


def rle(ops):
    out, last, cnt = [], None, 0
    for o in ops:
        if isinstance(o, int):
            if o == last:
                cnt += 1
            else:
                if last is not None:
                    out.append([last, cnt])
                last, cnt = o, 1
        else:
            if last is not None:
                out.append([last, cnt])
                last = None
            out.append(o)
    if last is not None:
        out.append([last, cnt])
    return out


def decode_templates(im):
    """收集全部時間表樣板，並跟著 E0 變身目標遞迴收集。"""
    timelines, todo = {}, set()
    for name, (start, reader, gal) in TIMELINES.items():
        rows, p = [], start
        while True:
            f = im.u16(p)
            if f == 0:
                break
            t = im.u16(p + 2)
            rows.append([f, t])
            todo.add(t)
            p += 4
            if len(rows) > 2000:
                die(f"{name} 時間表過長")
        timelines[name] = {"ds_offset": hx(start), "reader": reader, "gallery": gal, "rows": rows, "end": hx(p)}
    tmpl, morph_only = {}, set()
    pending = sorted(todo)
    while pending:
        t = pending.pop()
        if t in tmpl:
            continue
        w = read_words(im, t, 23)
        d = {"addr": hx(t), "words": w, "spawned_by_timeline": t in todo}
        mode = chr(w[8]) if w[8] < 128 else "?"
        d["fields"] = {n: w[i] for i, n, _, _ in ENTITY_WORDS if i < 23}
        d["letters"] = {k: chr_or_none(w[i]) for k, i in (("bullet", 7), ("mode", 8), ("explosion", 17), ("kind", 18), ("carrier", 19))}
        # 不變量
        d["spawn_pos"] = {"x": struct.unpack("<h", struct.pack("<H", w[9]))[0], "y": struct.unpack("<h", struct.pack("<H", w[10]))[0]}
        d["step_mismatch"] = bool(w[4] != w[11] or w[5] != 10 * w[12])
        if w[15] != 1 or w[21] != 0 or w[22] != 0 or w[2] != 0 or w[3] != 0 or w[16] != 0:
            die(f"樣板 {t:04X} 的保留欄位非預期值")
        # 圖框表
        kind = chr(w[18])
        if kind == "K":
            d["frames_k_record"] = {"image": im.u16(w[1]), "x": im.u16(w[1] + 2), "y": im.u16(w[1] + 4), "ptr": hx(w[1])}
        else:
            d["frames"] = {"ptr": hx(w[1]), "table": read_words(im, w[1], 9)}
        if w[20]:
            d["limits"] = {"ptr": hx(w[20]), "table": read_words(im, w[20], 9)}
        d["shape"] = read_shape(im, w[14])
        if mode == "A":
            d["script"] = {"kind": "heading", "initial_heading": w[13]}
        else:
            ops, n = disasm_entity_script(im, w[13])
            d["script"] = {"kind": "bytes", "ptr": hx(w[13]), "length": n, "ops_rle": rle(ops)}
            for o in ops:
                if isinstance(o, list) and o[0] == "M":
                    tgt = int(o[1][3:], 16)
                    if tgt not in tmpl:
                        pending.append(tgt)
                        morph_only.add(tgt)
        tmpl[t] = d
    for t in tmpl:
        tmpl[t]["morph_target_only"] = t in morph_only and not tmpl[t]["spawned_by_timeline"]
    return timelines, tmpl


def rand_stats(im):
    idx_addr, tbl = 0x42A, 0x42C
    vals = [im.u16(tbl + 2 * i) for i in range(0x4B)]
    if max(vals) > 15:
        die("亂數表出現大於 15 的值")
    cnt = [vals.count(v) for v in range(16)]
    return {"index_ds": hx(idx_addr), "table_ds": hx(tbl), "length": 0x4B, "values": vals, "count_by_value": cnt,
            "step": "idx = (idx + 1) mod 75 後取 table[idx]；每個實體自己帶 w22 當 idx（交換進出全域 ds:42A）"}


def fire_prob(vals, thr, d):
    if thr == 0x63:
        return 0.0
    return sum(1 for v in vals if v + d > thr) / len(vals)


def ebullet_templates(im):
    """從派發表收集全部敵方子彈樣板並解碼。"""
    addrs = set()
    for k, v in EBULLET_DISPATCH.items():
        for key in ("family", "templates"):
            if key in v:
                addrs.update(v[key].values() if isinstance(v[key], dict) else v[key])
        if "default" in v:
            addrs.add(v["default"])
    addrs.add(EBULLET_DEFAULT)
    out = {}
    for a in sorted(addrs):
        w = read_words(im, a, 15)
        d = {"addr": hx(a), "words": w}
        d["heading"] = w[0]
        d["frames"] = {"ptr": hx(w[1]), "table": read_words(im, w[1], 9)}
        d["dx_px"], d["dy_px"], d["dx"], d["dy"] = w[4], w[5], w[11], w[12]
        d["trail_E"] = w[7] == 0x59
        d["mask_gallery"] = "MY" if w[8] == 0x4D else "stage"
        if w[8] not in (0, 0x4D, 0x41):
            die(f"敵方子彈 {a:04X} 的 w8 非預期 {w[8]:X}")
        d["shape"] = read_shape(im, w[14])
        d["script_ptr"] = hx(w[13])
        out[a] = d
    return out


def disasm_bullet_script(im, p, ebullet, limit=64):
    """子彈腳本：低半位元組 = 方向/指令，高半位元組 F = 向回跳（低半位元組 = 位元組數）。"""
    ops, i = [], 0
    while i < limit:
        b = im.u8(p + i)
        hi, lo = b >> 4, b & 15
        if hi == 0xF:
            ops.append(["LOOP_BACK", lo])
            i += 1
            return rle(ops), i
        if hi != 0:
            die(f"子彈腳本 {p:04X}+{i} 出現未定義位元組 {b:02X}")
        if ebullet and lo == 0:
            ops.append(["END"])
            i += 1
            return rle(ops), i
        ops.append(lo)
        i += 1
    die(f"子彈腳本 {p:04X} 過長")


def player_bullets(im):
    out = {}
    for a in range(0x28D5, 0x2B55, 0x20):
        w = read_words(im, a, 16)
        d = {"addr": hx(a), "words": w}
        d["type"] = chr_or_none(w[0]) if w[0] else None
        d["image_MY"] = w[1]
        d["dx_px"], d["dy_px"], d["dx"], d["dy"] = w[4], w[5], w[11], w[12]
        d["initial_heading_w6"] = w[6]
        d["hit_effect"] = chr_or_none(w[15])
        d["shape"] = read_shape(im, w[14])
        ops, n = disasm_bullet_script(im, w[13], False)
        d["script"] = {"ptr": hx(w[13]), "length": n, "ops_rle": ops}
        out[a] = d
    return out


def effects(im):
    out = {}
    for ptr in sorted(MASK_NAMES):
        recs, p = [], ptr
        for _ in range(64):
            img, dx, dy, shp = im.u16(p), im.i16(p + 2), im.i16(p + 4), im.u16(p + 6)
            if img == 0x0F:
                recs.append({"end": True})
                break
            recs.append({"image_MY": img, "dx": dx, "dy": dy, "shape_ptr": hx(shp)})
            p += 8
        else:
            die(f"特效表 {ptr:04X} 未結束")
        out[hx(ptr)] = {"letters": MASK_NAMES[ptr], "records": recs}
    shapes = {}
    for s in (0x41AE, 0x41B3, 0x41B8):
        shapes[hx(s)] = read_shape(im, s)
    # 'e' 群爆：從機器碼掃描 (dx, dy, 動畫表) 序列
    big = []
    for rng_name, (a, b) in (("e_reachable", (0x17B99, 0x17C6C)), ("E_dead_duplicate", (0x17AA3, 0x17B76))):
        seg = im.code(a, b - a)
        pat = re.compile(rb"\xA1\xBA\x3F\x05(..)\xA3\xBC\x3F\xC7\x06\xBE\x3F(..)\xB8(..)\xE8..", re.S)
        rows = []
        for m in pat.finditer(seg):
            rows.append({"dx": struct.unpack("<H", m.group(1))[0], "dy": struct.unpack("<H", m.group(2))[0],
                         "list": hx(struct.unpack("<H", m.group(3))[0])})
        big.append({"range": rng_name, "entries": rows})
    return {"lists": out, "mask_shapes": shapes, "big_explosion": big,
            "slots": {"count": 30, "table_ds": hx(0x403A), "x_ds": hx(0x3FC2), "y_ds": hx(0x3FFE)}}


def cross_check_dispatch(im):
    """派發表的樣板位址必須等於函式機器碼內 mov word_200F3 / word_1F46F 立即值序列。"""
    pre_e = b"\xC7\x06\x33\x35"      # mov word_200F3, imm16
    pre_p = b"\xC7\x06\xAF\x28"      # mov word_1F46F, imm16
    got = {}
    for name, (a, b) in {"sub_1701E": (0x1701E, 0x17085), "sub_17085": (0x17085, 0x170B0), "sub_170B0": (0x170B0, 0x17117),
                         "sub_17117": (0x17117, 0x1717E), "sub_1717E": (0x1717E, 0x171E5), "sub_171E5": (0x171E5, 0x1724C)}.items():
        got[name] = imm_scan(im, a, b, pre_e)
    exp = {
        "sub_1701E": EBULLET_DISPATCH["F"]["family"] + [EBULLET_DEFAULT],
        "sub_170B0": EBULLET_DISPATCH["l"]["family"] + [0x3737],
        "sub_17117": EBULLET_DISPATCH["b"]["family"] + [0x3827],
        "sub_1717E": EBULLET_DISPATCH["k"]["family"] + [0x3917],
        "sub_171E5": EBULLET_DISPATCH["s"]["family"] + [0x3A07],
        "sub_17085": [0x3B15, 0x3B33, 0x3B51, 0x3B33],
    }
    for k, v in exp.items():
        if got[k] != v:
            die(f"{k} 立即值序列 {[hex(x) for x in got[k]]} 與派發表 {[hex(x) for x in v]} 不符")
    # sub_16D3B 全函式的直接立即值（不含呼叫子函式內的）
    direct = imm_scan(im, 0x16D3B, 0x16F9F, pre_e)
    want = [0x3AF7, 0x3B6F, 0x3B8D, 0x3BAB, 0x3BC9, 0x3BE7, 0x3C05, 0x3C23, 0x3C23, 0x3C41, 0x3CF5, 0x3D13,
            0x3C5F, 0x3C7D, 0x3C9B, 0x3D8B, 0x3DA9, 0x3557,
            0x3C41, 0x3C5F, 0x3C7D, 0x3C9B, 0x3CB9, 0x3CD7, 0x3CF5, 0x3D13, 0x3D13, 0x3C5F, 0x3C9B, 0x3CD7,
            0x3D6D, 0x3D4F, 0x3D31]
    if sorted(direct) != sorted(want):
        die("sub_16D3B 直接立即值與 EBULLET_DISPATCH 不符：" + str([hex(x) for x in direct]))
    # 玩家武器
    right = imm_scan(im, 0x165CF, 0x1669F, pre_p)
    left = imm_scan(im, 0x1669F, 0x1673E, pre_p)
    wr = [0x2935, 0x28F5, 0x2915, 0x2955, 0x2975, 0x2995, 0x29B5, 0x29D5, 0x28D5, 0x2B35]
    wl = [0x2A75, 0x29F5, 0x2A95, 0x2AB5, 0x2AD5, 0x2AF5, 0x2A15, 0x2B35]
    # 朝右的 28F5/2915 在 165FC 的不可達區塊內
    if sorted(right) != sorted(wr) or sorted(left) != sorted(wl):
        die("武器派發立即值與 WEAPON_TABLE 不符：" + str([hex(x) for x in right]) + str([hex(x) for x in left]))
    # 特效派發 sub_17A3A 的 mov ax, imm16 在 sub_17C6C 呼叫前
    seg = im.code(0x17A3A, 0x17C6C - 0x17A3A)
    mov = [struct.unpack_from("<H", m.group(0), 1)[0] for m in re.finditer(rb"\xB8..\xE8..", seg, re.S)]
    if not set([0x4094, 0x40D0, 0x40EE, 0x413C, 0x4162, 0x4188, 0x40B2]) <= set(mov):
        die("sub_17A3A 缺少預期的動畫表位址")
    return {"enemy_family_imms": {k: [hx(x) for x in v] for k, v in got.items()},
            "sub_16D3B_direct_imms": [hx(x) for x in direct],
            "weapon_right_imms": [hx(x) for x in right], "weapon_left_imms": [hx(x) for x in left]}


def homing_turn(im):
    """sub_173FE 的八個子函式各寫 8 個字組（ds:2C02..2C10），再交給 sub_175EA 轉向。
    回傳 table[目標朝向 1..8][目前朝向 0..8] = 下一幀朝向。"""
    ranges = [(0x1744A, 0x1747E), (0x1747E, 0x174B2), (0x174B2, 0x174E6), (0x174E6, 0x1751A),
              (0x1751A, 0x1754E), (0x1754E, 0x17582), (0x17582, 0x175B6), (0x175B6, 0x175EA)]
    slots = [0x2C02 + 2 * i for i in range(8)]
    table, lists = {}, {}
    for tgt, (a, b) in enumerate(ranges, start=1):
        seg = im.code(a, b - a)
        got = {}
        for m in re.finditer(rb"\xC7\x06(..)(..)", seg, re.S):
            off, val = struct.unpack("<H", m.group(1))[0], struct.unpack("<H", m.group(2))[0]
            if off in slots:
                got[off] = val
        if sorted(got) != slots:
            die(f"sub_173FE 目標 {tgt} 的子函式缺少 ds:2C02..2C10 的指派")
        lst = [got[o] for o in slots]
        lists[tgt] = lst
        row = {}
        for h in range(0, 9):
            if h == tgt:
                row[h] = tgt
            elif h in lst[0:4]:
                row[h] = h - 1
            elif h in lst[4:6]:
                row[h] = h + 1
            elif h == lst[6]:
                row[h] = lst[7]
            else:
                row[h] = 1
        table[tgt] = row
    return {"lists_ds_2C02_2C10": lists, "next_heading": table,
            "rule": "同向不變；h 在前 4 項 → h-1；在第 5、6 項 → h+1；等於第 7 項 → 第 8 項；其餘 → 1。目標不在 1..8 時朝向改為 7"}


def groups(im):
    rows, p = [], 0x19DC
    while True:
        f = im.u16(p)
        if f == 0:
            break
        ptr = im.u16(p + 2)
        rows.append({"frame": f, "record": hx(ptr), "state_image": im.u16(ptr), "x": im.i16(ptr + 2), "y": im.i16(ptr + 4), "dx": im.i16(ptr + 6)})
        p += 4
        if len(rows) > 500:
            die("群組表過長")
    return {"table_ds": hx(0x19DC), "slots_ds": {"state": hx(0x19C8), "x": hx(0x19A0), "y": hx(0x19B4), "dx": hx(0x198C)},
            "slot_count": 10, "rows": rows,
            "draw_order": "state 0x10 先畫、0x0F、0x0E 最後（sub_15C9C、sub_15CB8、sub_15CD4），圖庫為關卡圖庫"}


def player_block(im):
    shapes = {n: read_shape(im, a) for n, a in (("body", 0x0F0D), ("pod", 0x0F16), ("dirty3x3", 0x0F1E))}
    shapes["p2_body"] = read_shape(im, 0x0F7B)
    shapes["p2_pod"] = read_shape(im, 0x0F85)
    shapes["p2_dirty3x3"] = read_shape(im, 0x0F8D)
    return {
        "fields_p1": {"col": hx(0xF0B), "row": hx(0xF09), "x_px": hx(0xF07), "y_px": hx(0xF05), "sprite": hx(0xEFD),
                      "hit_flag": hx(0xF00), "invuln": hx(0xF01), "death_ctr": hx(0xF03), "weapon": hx(0xEE9),
                      "pod1": hx(0xECD), "pod2": hx(0xECE), "facing": hx(0xECF), "charge": hx(0xECB), "hp": hx(0x041E)},
        "fields_p2": {"col": hx(0xF79), "row": hx(0xF77), "x_px": hx(0xF61), "y_px": hx(0xF5F), "sprite": hx(0xF57),
                      "hit_flag": hx(0xF5A), "invuln": hx(0xF5B), "death_ctr": hx(0xF5D), "weapon": hx(0xF4B),
                      "pod1": hx(0xF2F), "pod2": hx(0xF30), "facing": hx(0xF31), "charge": hx(0xF2D), "hp": hx(0x0420)},
        "constants": {
            "start_col": 5, "start_row": 10, "col_odd_range": [1, 33], "col_step": 2, "row_range": [0, 16], "row_step": 1,
            "hp_init": 255, "hp_on_hit": "hp >>= 1 (cheat word_273A5='Y' 時不減)", "hp_zero_death_frames": 15,
            "invuln_frames": 6, "stage_reset_only_stage1_or_demo": True, "facing_init": "<",
            "main_shot_facing": ">", "charge_release_events": 14, "p1_pod_col_off": 3, "p1_pod_row_off": -3, "p1_pod2_row_gap": 5,
            "p2_pod_col_off": 3, "p2_pod_row_off": -1, "p2_pod2_row_gap": 2, "left_facing_pod_col_adj": -5,
        },
        "shapes": shapes,
    }


def main():
    if len(sys.argv) != 3:
        die("用法：export_entities.py <S.unpacked.bin> <輸出 json>")
    raw = open(sys.argv[1], "rb").read()
    sha = hashlib.sha256(raw).hexdigest()
    if sha != EXPECTED_SHA256:
        die(f"輸入雜湊 {sha} 不是預期的 S.unpacked.bin")
    im = Img(raw)
    sigs = verify_signatures(im)
    xchk = cross_check_dispatch(im)
    timelines, tmpl = decode_templates(im)
    rnd = rand_stats(im)
    eb = ebullet_templates(im)
    for a, d in eb.items():
        d["script"] = None
        ops, n = disasm_bullet_script(im, int(d["script_ptr"][3:], 16), True) if d["script_ptr"] != "ds:0007" else ([], 0)
        d["script"] = {"ptr": d["script_ptr"], "length": n, "ops_rle": ops}
    pb = player_bullets(im)
    diff = [{"menu": i, "name": nm, "fire_add_w1E6B4": a, "hp_add_w1E6B2": b} for i, (nm, a, b) in enumerate(
        [("EASY", 0, 0), ("NORMAL", 2, 0), ("HARD", 2, 1), ("HARDEST", 3, 1)])]
    # 以時間表實際用到的樣板計算開火機率
    probs = {}
    for t, d in tmpl.items():
        thr = d["fields"]["fire_thr"]
        probs[hx(t)] = {nm["name"]: round(fire_prob(rnd["values"], thr, nm["fire_add_w1E6B4"]), 4) for nm in diff}
    out = {
        "input_sha256": sha,
        "address_convention": "ds:X = 線性 0x1CBC0+X = 解包映像位移 0xCBC0+X；L:X = IDA 線性位址",
        "signatures": sigs,
        "cross_check": xchk,
        "entity_layout": {
            "slot_count": 15, "slot_pointer_table_ds": hx(0x2294), "slot_base_ds": hx(0x22B4), "slot_bytes": 48,
            "work_area_ds": hx(0x25B6), "work_area_linear": "L:1F176", "template_bytes": 46, "copy_words": 24,
            "per_slot_render_arrays_ds": {"image": hx(0x21EC), "x": hx(0x220C), "y": hx(0x222C), "kind": hx(0x224C), "state": hx(0x226C)},
            "words": [{"index": i, "offset": 2 * i, "name": n, "meaning": m, "level": lv, "work_linear": f"L:{0x1F176 + 2 * i:X}"} for i, n, m, lv in ENTITY_WORDS],
        },
        "move_codes": [{"code": f"{c:02X}", "name": n, "effect": e} for c, n, e in MOVE_CODES],
        "timelines": timelines,
        "templates": {hx(t): d for t, d in sorted(tmpl.items())},
        "fire_probability_per_frame": probs,
        "random": rnd,
        "homing": homing_turn(im),
        "difficulty": diff,
        "enemy_bullets": {
            "pool": {"slots": 20, "pointer_table_ds": hx(0x32B3), "slot_base_ds": hx(0x32DB), "slot_bytes": 30, "work_ds": hx(0x3535)},
            "dispatch": {k: {kk: ([hx(x) for x in vv] if isinstance(vv, list) else ({str(a): hx(b) for a, b in vv.items()} if isinstance(vv, dict) else (hx(vv) if kk == "default" else vv)))
                             for kk, vv in v.items()} for k, v in EBULLET_DISPATCH.items()},
            "default_template": hx(EBULLET_DEFAULT),
            "templates": {hx(a): d for a, d in sorted(eb.items())},
        },
        "player_bullets": {
            "pool": {"slots": 20, "pointer_table_ds": hx(0x2607), "slot_base_ds": hx(0x262F), "slot_bytes": 32, "work_ds": hx(0x28B1)},
            "templates": {hx(a): d for a, d in sorted(pb.items())},
            "weapons": {k: {kk: ([hx(x) for x in vv] if isinstance(vv, list) else vv) for kk, vv in v.items()} for k, v in WEAPON_TABLE.items()},
        },
        "effects": effects(im),
        "groups_stage1": groups(im),
        "player": player_block(im),
    }
    json.dump(out, open(sys.argv[2], "w"), ensure_ascii=False, indent=1)
    print("templates", len(tmpl), "enemy_bullet_templates", len(eb), "player_bullet_templates", len(pb), "signatures", len(sigs))


if __name__ == "__main__":
    main()
