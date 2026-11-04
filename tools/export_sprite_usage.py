#!/usr/bin/env python3
"""輸出 workspace/extracted/sprite-usage.json：15 個圖庫、1,337 張圖各自的實際用途。

用法（全部在容器內跑，工作區唯讀，只寫輸出檔）：
  docker run --rm --network none -u $(id -u):$(id -g) --memory 1g --cpus 2 --pids-limit 128 \
    --log-opt max-size=10m --log-opt max-file=3 \
    -v $PWD/workspace:/ws:ro -v $PWD/tools:/tools:ro -v $PWD/workspace/extracted:/out \
    python:3.12-slim timeout 300 python -I /tools/export_sprite_usage.py /ws /out/sprite-usage.json

輸入：
  <ws>/ida/s_disasm.txt              IDA 匯出的反組譯（映像基底 10000h，dseg:X = 1CBC0h + X）
  <ws>/unpacked/S.unpacked.bin       解包映像（dseg:X = 位移 0CBC0h + X）
  <ws>/extracted/manifest.json       圖庫尺寸、遮罩、調色盤
  <ws>/extracted/stage-tables.json   時間表與實體樣板
  <ws>/extracted/maps/SHT.SCN.bin    第三關地圖

方法：
  1. 從 start 與一個自修改跳躍（11619h 被補成 NOP，進入 1161Bh）做指令層級的可達性分析，
     不依賴 IDA 的 proc 邊界，死碼因此不會被算成用途。
  2. 對每個 blit 呼叫點往回找 x、y、索引、背景暫存區偏移的最近一次 mov（只吃立即值）。
  3. 表驅動的用途（實體樣板、敵彈與道具樣板、玩家子彈、特效腳本、第三關地圖）直接讀映像內的資料表。
  4. 人工判讀只放在 OWNER_INFO 等表，每項都有位址證據，執行時用步驟 2 的掃描值對照，不符就中止。
輸出欄位定義見 docs/re/12-stages-presentation.md。
"""
import collections
import hashlib
import json
import os
import re
import struct
import sys

ws, out_path = sys.argv[1], sys.argv[2]
DS = 0x1CBC0          # dseg:X = 線性 1CBC0h + X
IMG_DS = 0xCBC0       # 解包映像內 dseg 起點
FAIL = []


def fail(msg):
    FAIL.append(msg)


# ------------------------------------------------------------------ 反組譯與可達性
ADDR_RE = re.compile(r"^([0-9A-F]{5})\s+(.*)$")


def clean(x):
    return re.sub(r"\s{2,}", " ", x.split(";")[0].rstrip())


class Disasm:
    def __init__(self, path):
        self.ins, self.order, self.label, self.owner = {}, [], {}, {}
        cur = None
        for l in open(path, errors="replace").read().split("\n"):
            m = ADDR_RE.match(l)
            if not m:
                continue
            a = int(m.group(1), 16)
            body = clean(m.group(2))
            if not body:
                continue
            if body.endswith(" proc"):
                self.label[body.split()[0]] = a
                cur = body.split()[0]
                continue
            if body.endswith(":"):
                self.label[body[:-1]] = a
                continue
            p = body.split(None, 1)
            self.ins[a] = (p[0], p[1] if len(p) > 1 else "")
            self.order.append(a)
            self.owner[a] = cur
        self.pos = {a: i for i, a in enumerate(self.order)}
        self.nxt = {a: self.order[i + 1] for i, a in enumerate(self.order[:-1])}
        self.by_addr = {}
        for n, a in self.label.items():
            self.by_addr.setdefault(a, n)

    def resolve(self, name):
        if name in self.label:
            return self.label[name]
        m = re.search(r"_([0-9A-F]{5})$", name)
        return int(m.group(1), 16) if m else None

    def target(self, ops):
        m = re.search(r"\b((?:sub|loc|locret|byte|word|dword|nullsub)_?[0-9A-Za-z]+)\b", ops)
        return self.resolve(m.group(1)) if m else None

    def reachable(self, seeds, extra):
        seen, stack = set(), list(seeds)
        while stack:
            a = stack.pop()
            while a is not None and a not in seen and a in self.ins:
                seen.add(a)
                mn, ops = self.ins[a]
                nx = self.nxt.get(a)
                if a in extra:
                    stack.extend(extra[a])
                if mn in ("retn", "retf", "iret"):
                    break
                if mn == "jmp":
                    if "$+" in ops:
                        a = nx
                        continue
                    t = self.target(ops)
                    if t is not None:
                        stack.append(t)
                    break
                if mn.startswith("j") or mn.startswith("loop"):
                    t = None if "$+" in ops else self.target(ops)
                    if t is not None:
                        stack.append(t)
                    a = nx
                    continue
                if mn == "call":
                    t = self.target(ops)
                    if t is not None:
                        stack.append(t)
                a = nx
        return seen


GLOBALS = {0x26880: "x", 0x26882: "y", 0x26888: "idx", 0x2685C: "bgoff", 0x26864: "sc", 0x26866: "sr",
           0x26868: "sx"}


def norm_mem(op):
    op = op.strip()
    m = re.match(r"(?:word|byte) ptr ds:([0-9A-F]+)h?$", op)
    if m:
        return DS + int(m.group(1), 16)
    m = re.match(r"(?:word|byte)_([0-9A-F]{5})$", op)
    return int(m.group(1), 16) if m else None


def imm(v):
    v = v.strip()
    m = re.match(r"^([0-9A-F]+)h$", v)
    if m:
        return int(m.group(1), 16)
    return int(v) if re.match(r"^\d+$", v) else None


def scan_sites(d, wrappers, window=70):
    inv = {ad: n for n, ad in d.label.items() if n in wrappers}
    res = []
    for a in d.order:
        mn, ops = d.ins[a]
        if mn != "call" or d.target(ops) not in inv:
            continue
        vals, mods = {}, set()
        i, steps = d.pos[a] - 1, 0
        while i >= 0 and steps < window:
            b = d.order[i]
            m2, o2 = d.ins[b]
            if d.owner.get(b) != d.owner.get(a) or m2 in ("retn", "retf", "iret"):
                break
            if m2 == "mov":
                p = o2.split(",", 1)
                n = norm_mem(p[0]) if len(p) == 2 else None
                if n in GLOBALS and GLOBALS[n] not in vals:
                    k = imm(p[1])
                    if k is None:
                        src = p[1].strip()
                        k = src
                        if i > 0:
                            pm, po = d.ins[d.order[i - 1]]
                            if pm == "mov" and po.split(",", 1)[0].strip() == src:
                                k2 = imm(po.split(",", 1)[1])
                                k = k2 if k2 is not None else po.split(",", 1)[1].strip()
                    vals[GLOBALS[n]] = k
            elif m2 in ("inc", "dec", "add", "sub"):
                n = norm_mem(o2.split(",", 1)[0])
                if n in GLOBALS and GLOBALS[n] not in vals:
                    mods.add(GLOBALS[n])
            i -= 1
            steps += 1
        res.append({"addr": a, "wrapper": inv[d.target(ops)], "vals": vals, "mods": sorted(mods)})
    return res


def imm_assigns(d, reach, varname):
    """所有可達的 `mov <varname>, 立即值`。"""
    out = set()
    for a in d.order:
        if a not in reach:
            continue
        mn, ops = d.ins[a]
        if mn == "mov":
            p = ops.split(",", 1)
            if len(p) == 2 and p[0].strip() == varname and imm(p[1]) is not None:
                out.add(imm(p[1]))
    return out


def owner_assigns(d, reach, owner, varname):
    out = set()
    for a in d.order:
        if a in reach and d.owner.get(a) == owner:
            mn, ops = d.ins[a]
            if mn == "mov":
                p = ops.split(",", 1)
                if len(p) == 2 and norm_mem(p[0]) == varname and imm(p[1]) is not None:
                    out.add(imm(p[1]))
    return out


# ------------------------------------------------------------------ 資料
dis = Disasm(os.path.join(ws, "ida", "s_disasm.txt"))
REACH = dis.reachable([0x10000], {0x11619: [0x1161B]})
IMG = open(os.path.join(ws, "unpacked", "S.unpacked.bin"), "rb").read()
MAN = json.load(open(os.path.join(ws, "extracted", "manifest.json")))
TBL = json.load(open(os.path.join(ws, "extracted", "stage-tables.json")))
SCN = open(os.path.join(ws, "extracted", "maps", "SHT.SCN.bin"), "rb").read()


def W(o):
    return struct.unpack_from("<H", IMG, IMG_DS + o)[0]


def SW(o):
    return struct.unpack_from("<h", IMG, IMG_DS + o)[0]


def B(o):
    return IMG[IMG_DS + o]


def chv(v):
    return chr(v) if 32 <= v < 127 else v


def hx(v):
    return f"{v:04X}"


LIBS = ["MY", "SHT1", "SHT2", "SHT3", "SHT4", "SHT5", "BH1", "BH2", "BH4", "J1", "J2", "J3", "J4", "MAP", "BEG1"]
BANK = {}
for lib in LIBS:
    b = MAN["banks"][lib]
    BANK[lib] = {
        "count": b["count"],
        "sprites": {s["index"]: s for s in b["sprites"]},
        "masks": {s["index"]: s for s in b["masks"]},
        "palette_default": b["palette"],
        "mask_source": b.get("mask_source"),
    }


def spr_size(lib, i):
    s = BANK[lib]["sprites"].get(i)
    return None if s is None else [s["width"], s["height"]]


def mask_info(lib, i):
    m = BANK[lib]["masks"].get(i)
    if m is None:
        return None
    return {"index": i, "size": [m["width"], m["height"]]}


def is_placeholder(lib, i):
    s = BANK[lib]["sprites"].get(i)
    return s is not None and s["height"] == 1 and s["width"] <= 16


# ------------------------------------------------------------------ 場景與調色盤
# 每個場景的有效調色盤與切換證據。設定函式與位址見 docs/re/12-stages-presentation.md 第 2、6 節。
SCENES = {
    "boot-loader": {"palettes": ["loader"], "note": "sub_11112 @10C96，MY.GO 載入後的第一畫面",
                    "evidence": ["10C96"]},
    "title-beg1-a": {"palettes": ["beg1_a"], "note": "sub_129E5 @11910 與 @11C0F；BEG1 第 7～31 項動畫",
                     "evidence": ["11910", "11C0F"]},
    "title-beg1-flash": {"palettes": ["beg1_b"], "note": "sub_12A13 @11C03，BEG1 第 31 項後的閃白，之後回 beg1_a",
                         "evidence": ["11C03"]},
    "title-d": {"palettes": ["title_d"], "note": "sub_1295B：BEG1 第 1 項（@115A1 前）、BEG1 第 0 項與製作名單（@115C1 前）",
                "evidence": ["11591", "115C1"]},
    "title-e": {"palettes": ["title_e"], "note": "sub_12989 @115AE：BEG1 第 2～6、14～17 項",
                "evidence": ["115AE"]},
    "menu": {"palettes": ["menu"], "note": "sub_129B7 @12588：難度與控制方式選單", "evidence": ["12588"]},
    "pic-stage1": {"palettes": ["j1"], "note": "sub_12A41 @11C6B 與 @1286A；只在一般模式（byte_1CFDC=N）顯示 J1 圖，等任意鍵",
                   "evidence": ["11C6B", "1286A", "11C4A"]},
    "pic-stage2": {"palettes": ["j2"], "note": "sub_12A6F @11E46 與 @12895；只在一般模式顯示",
                   "evidence": ["11E46", "12895", "11E25"]},
    "pic-stage3": {"palettes": ["j3"], "note": "sub_12A9D @127DF；只在一般模式顯示", "evidence": ["127DF", "121BC"]},
    "pic-stage4": {"palettes": ["j4"], "note": "sub_12ACB @128C0；只在一般模式顯示", "evidence": ["128C0", "12358"]},
    "card-stage1": {"palettes": ["j1", "title_d"], "note": "ACT 1 字樣。一般模式沿用 j1（@11C6B），示範模式（byte_1CFDC=Y）改 title_d（@11C46）",
                    "evidence": ["11C6B", "11C46"]},
    "card-stage2": {"palettes": ["j2", "title_d"], "note": "ACT 2。一般 j2（@11E46），示範 title_d（@11E21）",
                    "evidence": ["11E46", "11E21"]},
    "card-stage3": {"palettes": ["j3", "title_d"], "note": "ACT 3。一般 j3（@127DF 於 sub_127CD 內設定），示範 title_d（@121B8）",
                    "evidence": ["127DF", "121B8"]},
    "card-stage4": {"palettes": ["j4", "title_d"], "note": "ACT 4。一般 j4（@128C0），示範 title_d（@12354）",
                    "evidence": ["128C0", "12354"]},
    "stage1-main": {"palettes": ["stage1"], "note": "sub_110E3 @10F09，幀計數 1；幀 0 仍是 loader 調色盤（@10D05）",
                    "frames": [1, 619], "evidence": ["10F09", "10D05"]},
    "stage1-boss": {"palettes": ["stage1_boss"], "note": "sub_11141 @10F14，幀計數 26Ch（620）",
                    "frames": [620, None], "evidence": ["10F14"]},
    "stage2-main": {"palettes": ["stage2", "stage2_d"],
                    "note": "sub_137F8 @13229 與幀 1（@1343F）設定 stage2；幀 161～260 每個奇數頁由 sub_13894 把"
                            " stage2_d 工作表（CBFh 起 21 位元組，DAC 第 8～14 色）每次減 1",
                    "frames": [0, 261], "evidence": ["13229", "1343F", "13431"]},
    "stage2-mid": {"palettes": ["stage2_b"], "note": "sub_13827 @13419，幀計數 106h（262），之前 sub_132AB 重置 stage2_d 表",
                   "frames": [262, 629], "evidence": ["13419", "13416"]},
    "stage2-boss": {"palettes": ["stage2_c"], "note": "sub_13856 @1341D，幀計數 276h（630）",
                    "frames": [630, None], "evidence": ["1341D"]},
    "stage3": {"palettes": ["stage3"], "note": "sub_13D32 @13A72，全關不再切換", "evidence": ["13A72"]},
    "stage4": {"palettes": ["stage4"], "note": "sub_14063 @13DD2，全關不再切換", "evidence": ["13DD2"]},
    "ending-1": {"palettes": ["ending_a"], "note": "sub_143EA @1420E，SHT5 第 3 項地景", "evidence": ["1420E"]},
    "ending-2": {"palettes": ["ending_b"], "note": "sub_14419 @14240，按鍵後 SHT5 第 4 項與 END 字樣",
                 "evidence": ["14240"]},
    "any-gameplay": {"palettes": ["stage1", "stage1_boss", "stage2", "stage2_b", "stage2_c", "stage3", "stage4"],
                     "note": "MY.GO 的玩家、子彈、爆炸在四關都用，色號 1～4 與 8～14 隨關卡調色盤變色",
                     "evidence": ["10F09", "13229", "13A72", "13DD2"]},
    "inherit": {"palettes": [], "note": "不設定調色盤，沿用當下畫面", "evidence": []},
}
GAMEPLAY = "any-gameplay"


def stage_scene(stage, frame):
    if stage == "stage1":
        return "stage1-main" if frame < 620 else "stage1-boss"
    if stage == "stage2":
        return "stage2-main" if frame < 262 else ("stage2-mid" if frame < 630 else "stage2-boss")
    if stage == "stage3":
        return "stage3"
    if stage == "stage4_a":
        return "stage4"
    return "ending-2"


STAGE_LIB = {"stage1": "SHT1", "stage2": "SHT2", "stage3": "SHT3", "stage4_a": "SHT4", "stage4_b": "SHT5"}
STAGE_LABEL = {"stage1": "第一關", "stage2": "第二關", "stage3": "第三關", "stage4_a": "第四關", "stage4_b": "片尾"}

# ------------------------------------------------------------------ 輸出容器
ENT = {}     # (lib, idx) -> entry


def entry(lib, i):
    k = (lib, i)
    if k not in ENT:
        ENT[k] = {"lib": lib, "index": i, "size": spr_size(lib, i), "uses": [], "roles": [], "scenes": set(),
                  "draw_modes": set(), "mask_dy": set(), "mask_used": False, "families": []}
    return ENT[k]


FAMILIES = []     # (lib, [indices], label, kind, note)


def family(lib, idxs, label, kind, note=None):
    idxs = sorted(set(idxs))
    for i in idxs:
        if i not in BANK[lib]["sprites"]:
            fail(f"family {lib}:{label} 含不存在的索引 {i}")
    FAMILIES.append({"lib": lib, "members": idxs, "label": label, "kind": kind, "note": note})


def use(lib, i, role, scene, draw, site=None, sub=None, mask=False, mask_dy=None, **kw):
    """記錄一次用途。site 是位址字串或 None，reach 以 site 判定。"""
    if i not in BANK[lib]["sprites"]:
        fail(f"{lib} 索引 {i} 不存在（{kw}）")
        return
    e = entry(lib, i)
    u = {"role": role, "scene": scene, "draw": draw}
    if sub:
        u["subrole"] = sub
    if site is not None:
        u["site"] = site
        a = int(site.split("+")[0], 16)
        u["reachable"] = a in REACH
        if not u["reachable"]:
            u["role_if_reached"] = role
            u["role"] = "dead-code"
    else:
        u["reachable"] = True
    u.update(kw)
    e["uses"].append(u)
    if u["reachable"]:
        e["roles"].append(role)
        e["scenes"].add(scene)
        e["draw_modes"].add(draw)
        if mask:
            e["mask_used"] = True
            e["mask_dy"].add(mask_dy)


# ------------------------------------------------------------------ 1. 固定索引的 blit 呼叫點
WRAP_MY = {"sub_18C0A", "sub_18C6E", "sub_18CE2", "sub_18D46", "sub_18D89", "sub_18DCC", "sub_18E30", "sub_18E73",
           "sub_18EB6", "sub_18B96", "sub_18BFA"}
WRAP_SHT = {"sub_18F1A", "sub_18F5D", "sub_18FC1", "sub_19025", "sub_19ADC"}
WRAP_BJ = {"sub_19068", "sub_190CC", "sub_19B40"}
WRAP_MAP = {"sub_18ACE", "sub_18B32"}
SITES = scan_sites(dis, WRAP_MY | WRAP_SHT | WRAP_BJ | WRAP_MAP)
SITE_BY_ADDR = {s["addr"]: s for s in SITES}

# 每個 owner（IDA proc 名）的語意。lib=None 表示用途由資料表決定，下面另有專門的段落處理。
# scene：SCENES 的鍵。role：player enemy boss_part bullet explosion item background ui text cutscene effect
OWNER_INFO = {
    # 啟動與標題
    "sub_10C86": dict(lib="MY", role="text", scene="boot-loader", draw="plane-replace",
                      note="載入 MY.GO 後的第一畫面，第 25 項 8x3 小條在 (5,80)，第 26 項字樣在 (10,170)"),
    "sub_10F41": dict(lib="MY", role="text", scene="stage1-main", draw="plane-replace",
                      note="第一關幀計數 0～620 每幀畫在 (27,172)，boss 段 sub_10F9F 不再畫（除錯用幀數字樣不算）"),
    "sub_116F3": dict(lib=None, note="BEG1 第 0 項經 BG 暫存區 B 往上捲，結尾疊 MY 第 47 項"),
    "sub_1175B": dict(lib="BEG1", role="cutscene", scene="title-d", draw="bg-store-b-replace+scroll"),
    "sub_117BF": dict(lib="MY", role="text", scene="title-d", draw="plane-replace"),
    "sub_11806": dict(lib="BEG1", role="cutscene", scene="title-e", draw="plane-replace"),
    "sub_118A2": dict(lib="BEG1", role="cutscene", scene="title-e", draw="plane-replace"),
    "sub_11910": dict(lib="BEG1", role="cutscene", scene="title-beg1-a", draw="bg-store-b-replace+scroll"),
    "sub_1199F": dict(lib="BEG1", role="cutscene", scene="title-beg1-a", draw="plane-replace"),
    # 關間卡
    "sub_127CD": dict(lib="J3", role="cutscene", scene="pic-stage3", draw="bg-store-b-replace+scroll"),
    "sub_12858": dict(lib="J1", role="cutscene", scene="pic-stage1", draw="bg-store-b-replace"),
    "sub_12883": dict(lib="J2", role="cutscene", scene="pic-stage2", draw="bg-store-b-replace"),
    "sub_128AE": dict(lib="J4", role="cutscene", scene="pic-stage4", draw="bg-store-b-replace"),
    # 選單
    "sub_126B8": dict(lib="MY", role="text", scene="menu", draw="plane-replace"),
    "sub_12438": dict(lib="BEG1", role="cutscene", scene="menu", draw="plane-replace", note="1243Ch 之後無入口的舊選單畫面"),
    # 背景暫存區（階段開頭或頭目登場時寫入）
    "sub_10B65": dict(lib="BH1", role="background", scene="stage1-main", draw="bg-store-a-replace"),
    "sub_10C3D": dict(lib="SHT1", role="boss_part", scene="stage1-boss", draw="bg-store-b-replace"),
    "sub_12FB6": dict(lib="BH2", role="background", scene="stage2-main", draw="bg-store-a/b-replace"),
    "sub_1316C": dict(lib="SHT2", role="boss_part", scene="stage2-mid", draw="bg-store-b-replace"),
    "sub_131D1": dict(lib="SHT2", role="boss_part", scene="stage2-boss", draw="bg-store-b-replace"),
    "sub_13344": dict(lib="SHT2", role="boss_part", scene="stage2-boss", draw="bg-store-b-replace"),
    "sub_13A32": dict(lib="SHT3", role="boss_part", scene="stage3", draw="bg-store-a-replace"),
    "sub_13D70": dict(lib="BH4", role="background", scene="stage4", draw="bg-store-a-replace"),
    "sub_13E74": dict(lib="SHT4", role="placeholder", scene="stage4", draw="bg-store-a-replace",
                      note="幀 1002 清暫存區後畫 16x1 佔位圖，無可見內容"),
    "sub_141FC": dict(lib="SHT5", role="cutscene", scene="ending-1", draw="bg-store-b-replace",
                      scene_by_idx={3: "ending-1", 4: "ending-2"}),
}
# 這些 owner 的索引不是固定值，由資料表或變數決定，後面逐段處理
DYNAMIC_OWNERS = {"sub_14704", "sub_1478B", "sub_147BC", "sub_1480A", "sub_15232", "sub_152B9", "sub_152EA",
                  "sub_15338", "sub_15CF0", "sub_15DBE", "sub_164F3", "sub_16541", "sub_1691A", "sub_16A99",
                  "sub_16B18", "sub_17768", "sub_17CB6", "sub_17DF9", "sub_182CD", "sub_185AA", "sub_18613",
                  "sub_1812E", "sub_181E1", "sub_1840E", "sub_1852B"}
ACT_OWNERS = {"sub_11CFC": "card-stage1", "sub_11D58": "card-stage1", "sub_11DAC": "card-stage1",
              "sub_11F58": "card-stage2", "sub_11FB3": "card-stage2", "sub_1200A": "card-stage2",
              "sub_12062": "card-stage2", "sub_120B6": "card-stage2", "sub_12132": "card-stage2",
              "sub_1221E": "card-stage3", "sub_123BD": "card-stage4"}

# 迴圈內索引遞增的呼叫點：位址 -> (起始索引, 次數)。掃描值必須等於起始索引。
LOOPS = {0x10BDD: (1, 10, "const"), 0x13082: (4, 20, "const"), 0x130A3: (5, 8, "inc"), 0x130CF: (13, 4, "inc"),
         0x130F4: (17, 2, "inc"), 0x1311F: (19, 4, "inc"), 0x13144: (23, 4, "inc")}

MY_ROLE_FIXED = {  # 第 11、12 節以外，MY 固定索引的語意（owner 為 DYNAMIC 者另有處理）
}

fixed_seen = collections.Counter()
for s in SITES:
    a, w, v, o = s["addr"], s["wrapper"], s["vals"], dis.owner[s["addr"]]
    if o in DYNAMIC_OWNERS:
        continue
    if o in ACT_OWNERS:
        continue
    if o in ("nullsub_4", "nullsub_5"):
        info = dict(lib="MY", role="unused", scene="inherit", draw="plane-replace",
                    note="進度條函式 nullsub_4 被改成 retn，原函式本體留在其後")
    elif o == "sub_1C9BB":
        info = dict(lib="MY", role="text", scene="inherit", draw="plane-replace",
                    note="磁碟片提示，只有檔案讀取失敗且 word_1CFA8 非 0（軟碟安裝）時顯示")
    elif o == "sub_116F3":
        info = dict(lib="BEG1" if w == "sub_19B40" else "MY", role="cutscene" if w == "sub_19B40" else "text",
                    scene="title-d", draw="bg-store-b-replace+scroll" if w == "sub_19B40" else "plane-replace")
    else:
        info = OWNER_INFO.get(o)
    if info is None or info.get("lib") is None:
        fail(f"owner {o} @{a:X} 沒有語意（{v}）")
        continue
    idx = v.get("idx")
    if not isinstance(idx, int):
        # 由 owner 的程式決定的索引，另處理
        continue
    lib, role = info["lib"], info["role"]
    if o == "sub_10C86" and idx == 25:
        role = "ui"
    sub = None
    # 單一 owner 內角色依索引分流
    if o == "sub_1199F":
        sub = "ship" if idx in (8, 9, 10, 11, 12, 13, 18, 19, 20) else ("exhaust" if 21 <= idx <= 28 else "planet-flash")
    if o == "sub_11806":
        sub = "ship-front-zoom"
    if o == "sub_118A2":
        sub = "title-logo"
    if o == "sub_117BF":
        sub = "credits" if idx == 48 else "logo"
    if o == "sub_116F3" and w != "sub_19B40":
        sub = "logo"
    if o == "sub_10C86":
        sub = "boot"
    if o == "sub_10F41":
        sub = "banner"
    if o == "sub_126B8":
        sub = "portrait" if 73 <= idx <= 76 else "label"
    if o == "sub_1C9BB":
        sub = "disk-prompt"
    if o == "sub_13E74":
        sub = "placeholder"
    count = 1
    if a in LOOPS:
        i0, count, kind = LOOPS[a]
        if idx != i0:
            fail(f"迴圈位址 {a:X} 掃描索引 {idx} 不等於預期 {i0}")
        idxs = [i0 + k for k in range(count)] if kind == "inc" else [i0]
    else:
        idxs = [idx]
    mask_wrap = w in ("sub_18D46", "sub_18E30", "sub_18E73", "sub_18F1A")
    for j in idxs:
        use(lib, j, role, info.get("scene_by_idx", {}).get(j, info["scene"]), "mask-and" if mask_wrap else f"{w}", site=f"{a:X}", sub=sub,
            func=o, x=v.get("x"), y=v.get("y"), bgoff=v.get("bgoff"), mask=False,
            **({"loop_count": count} if a in LOOPS else {}))
        fixed_seen[(lib, j)] += 1
        if o == "sub_1199F" and j >= 29:
            use(lib, j, role, "title-beg1-flash", "plane-replace", site=f"{a:X}", sub=sub, func=o,
                note="第 31 項之後 sub_12A13（@11C03）把調色盤換成 beg1_b 閃白")

# 磁碟片提示：第 70～72 項依 byte_274EC（磁碟 1～3）擇一，掃描只取到最後一次指派，改讀 owner 內全部立即值
disk_idx = owner_assigns(dis, REACH, "sub_1C9BB", 0x26888)
if disk_idx != {68, 69, 70, 71, 72}:
    fail(f"磁碟片提示索引預期 68～72，實際 {sorted(disk_idx)}")
for i in (70, 71, 72):
    if (("MY", i) not in fixed_seen):
        use("MY", i, "text", "inherit", "my-plane-replace", site="1CA17", sub="disk-prompt", func="sub_1C9BB",
            note=f"磁碟代號字母，byte_274EC={'123'[i - 70]} 時顯示（1C9F0、1C9F9、1CA02）")

# 規格化 draw 名稱：把 wrapper 名換成可讀說明
DRAW_NAME = {
    "sub_18C0A": "my-plane-replace", "sub_18C6E": "my-plane-inverted", "sub_18CE2": "my-plane-or",
    "sub_18D46": "my-mask-and", "sub_18D89": "my-mask-inverted-or", "sub_18DCC": "my-plane-or+sound",
    "sub_18E30": "my-mask-and", "sub_18E73": "my-mask-and", "sub_18EB6": "my-plane-or",
    "sub_19068": "bj-plane-replace", "sub_190CC": "bj-plane-inverted", "sub_19B40": "bj-plane-to-bg-store",
    "sub_19ADC": "bg-store-plane-replace", "sub_18B96": "my-plane-replace-fullheight",
    "sub_18BFA": "my-clear",
}
for e in ENT.values():
    e["draw_modes"] = {DRAW_NAME.get(m, m) for m in e["draw_modes"]}
    for u in e["uses"]:
        u["draw"] = DRAW_NAME.get(u["draw"], u["draw"])

# ACT 卡：x、y 來自變數，索引固定
ACT_SIDX = collections.OrderedDict()
for s in SITES:
    o = dis.owner[s["addr"]]
    if o in ACT_OWNERS and isinstance(s["vals"].get("idx"), int):
        ACT_SIDX.setdefault((o, s["vals"]["idx"]), []).append(s)
for (o, idx), ss in ACT_SIDX.items():
    ra = [s for s in ss if s["addr"] in REACH]
    if not ra:
        site = f"{ss[0]['addr']:X}"
    else:
        site = f"{ra[0]['addr']:X}"
    sub = "letter" if idx in (61, 62, 63) else "stage-number"
    use("MY", idx, "text", ACT_OWNERS[o], "my-mask-and+plane-or", site=site, sub=sub, func=o,
        mask=True, mask_dy=0, y=ss[0]["vals"].get("y"))

# ------------------------------------------------------------------ 2. 選單、玩家與其他由變數決定的 MY 索引
# 選單：77+0、82+0 的標籤，78+難度、83+控制、73+難度的字樣與頭像。座標在 sub_126B8。
for i in (78, 79, 80, 81):
    use("MY", i, "text", "menu", "my-plane-replace", site="12711", sub="label", func="sub_126B8",
        note="索引 = 4Eh + word_1D142（難度 0～3）")
for i in (83, 84, 85):
    use("MY", i, "text", "menu", "my-plane-replace", site="12729", sub="label", func="sub_126B8",
        note="索引 = 53h + word_1D144（控制 0～2）")
for i in (73, 74, 75, 76):
    use("MY", i, "ui", "menu", "my-plane-replace", site="12741", sub="portrait", func="sub_126B8",
        note="索引 = 49h + word_1D142，頭像表情隨難度變，放在 (3,30)")

# 玩家機體：word_1DABD / word_1DB17 的立即值指派，必須落在可達碼
p1 = imm_assigns(dis, REACH, "word_1DABD")
p2 = imm_assigns(dis, REACH, "word_1DB17")
if p1 != {7, 9, 11}:
    fail(f"P1 機體索引集合預期 {{7,9,11}}，實際 {p1}")
if p2 != {14, 16, 18}:
    fail(f"P2 機體索引集合預期 {{14,16,18}}，實際 {p2}")
P1_ROLE = {9: "中立", 7: "向下（146C3 設 7）", 11: "向上（146AC 設 0Bh）"}
P2_ROLE = {16: "中立（15037）", 14: "向上（151DA 設 0Eh）", 18: "向下（151F1 設 12h）"}
for i in sorted(p1):
    use("MY", i, "player", GAMEPLAY, "my-mask-and(y)+plane-or(y+1)", site="14726", sub="ship-p1", mask=True,
        mask_dy=-1, func="sub_14704", pose=P1_ROLE[i])
for i in sorted(p2):
    use("MY", i, "player", GAMEPLAY, "my-mask-and(y)+plane-or(y+1)", site="15254", sub="ship-p2", mask=True,
        mask_dy=-1, func="sub_15232", pose=P2_ROLE[i])
# 受傷閃白：同一組遮罩改畫反相 OR（sub_18D89）
for i in sorted(p1):
    use("MY", i, "player", GAMEPLAY, "my-mask-inverted-or (hit flash, odd page)", site="14753", sub="ship-p1",
        func="sub_14704", note="byte_1DAC1>0 時只畫遮罩，奇數頁再把遮罩反相 OR 成白色剪影，持續 6 幀")
for i in sorted(p2):
    use("MY", i, "player", GAMEPLAY, "my-mask-inverted-or (hit flash, odd page)", site="15281", sub="ship-p2",
        func="sub_15232", note="同 P1")

# 機體附屬：噴焰、槍口、閃電、砲座
for i, pg in ((13, "偶數頁"), (12, "奇數頁")):
    use("MY", i, "player", GAMEPLAY, "my-mask-and+plane-or", site="14757" if i == 13 else "14769", sub="exhaust",
        mask=True, mask_dy=0, func="sub_14704", note=f"byte_1DAAB=Y（向右移動）時畫在機體左後方，{pg}")
use("MY", 17, "player", GAMEPLAY, "my-mask-and+plane-or", site="1479C", sub="muzzle", mask=True, mask_dy=0,
    func="sub_1478B", note="byte_1DAA5=Y（開火中）畫在 (x+3,y+3)，P1 專用")
use("MY", 10, "player", GAMEPLAY, "my-mask-and+plane-or", site="148C0", sub="muzzle", mask=True, mask_dy=0,
    func="sub_1480A", note="每個附屬砲座旁的 8x7 亮點；P2 在 152CA")
use("MY", 10, "player", GAMEPLAY, "my-mask-and+plane-or", site="152CA", sub="muzzle", mask=True, mask_dy=0,
    func="sub_152B9", note="P2 版")
use("MY", 8, "effect", GAMEPLAY, "my-mask-and+plane-or", site="147D4", sub="pickup-lightning", mask=True, mask_dy=0,
    func="sub_147BC", note="撿到武器字母後 word_1DAA6=FF42h，之後每幀 +28h 往下落，到 >0 為止，約 6 幀；P2 在 15302")
# 砲座：索引 = 基底 + 動畫計數，基底與計數上限由 sub_14908、sub_1496D、sub_149D2 決定
POD = {"sub_14908": (0x1E, 0x24, 5, "<6 級"), "sub_1496D": (0x31, 0x31, 3, "6～13 級"),
       "sub_149D2": (0x34, 0x34, 3, "≥14 級")}
for fn, (br, bl, n, lvl) in POD.items():
    a0 = dis.label[fn]
    adds, cmps = set(), set()
    for a in dis.order:
        if dis.owner.get(a) == fn and a in REACH:
            mn, ops = dis.ins[a]
            if mn == "add" and ops.startswith("ax, "):
                adds.add(imm(ops.split(",")[1]))
            if mn == "cmp" and ops.startswith("word_1DA96, "):
                cmps.add(imm(ops.split(",")[1]))
    if not ({br, bl} <= adds and n in cmps):
        fail(f"{fn} 砲座基底預期 {br:X}/{bl:X}、計數上限 {n}，實際 add={adds} cmp={cmps}")
    sub = "pod" if fn == "sub_14908" else "charge-orb"
    for k in range(n):
        use("MY", br + k, "player", GAMEPLAY, "my-mask-and+plane-or", site="14834", sub=sub, mask=True, mask_dy=0,
            func="sub_1480A", facing="右（byte word_1DA8F='>'）" if bl != br else "左右同圖", level=lvl)
        if bl != br:
            use("MY", bl + k, "player", GAMEPLAY, "my-mask-and+plane-or", site="14834", sub=sub, mask=True,
                mask_dy=0, func="sub_1480A", facing="左", level=lvl)

# ------------------------------------------------------------------ 3. 玩家子彈樣板（MY）
# sub_16578 把 ds:word_1F46F 指到的樣板複製進 20 槽的子彈池，樣板第 1 字組是 MY 索引。
bt = sorted(imm_assigns(dis, REACH, "word_1F46F"))
BUL_T = {}
for t in bt:
    BUL_T[t] = {"type": chv(W(t)), "idx": W(t + 2)}
b_sites = collections.defaultdict(list)
for a in dis.order:
    if a in REACH:
        mn, ops = dis.ins[a]
        if mn == "mov" and ops.startswith("word_1F46F, "):
            b_sites[imm(ops.split(",")[1])].append(a)
WEAPON_OF = {  # sub_165CF 與 sub_1669F 的分派：樣板位址 -> (武器字母, 朝向)
    0x28D5: ("預設", ">"), 0x28F5: ("預設", ">"), 0x2915: ("預設", ">"), 0x2935: ("K", ">"),
    0x2955: ("S", ">"), 0x2975: ("S", ">"), 0x2995: ("B", ">"), 0x29B5: ("B", ">"), 0x29D5: ("L", ">"),
    0x29F5: ("L", "<"), 0x2A15: ("預設", "<"), 0x2A35: ("預設", "<"), 0x2A55: ("預設", "<"), 0x2A75: ("K", "<"),
    0x2A95: ("S", "<"), 0x2AB5: ("S", "<"), 0x2AD5: ("B", "<"), 0x2AF5: ("B", "<"), 0x2B15: ("L", "<"),
    0x2B35: ("F 蓄力", "兩向"),
}
for t, info in BUL_T.items():
    wp = WEAPON_OF.get(t, ("?", "?"))
    if t not in WEAPON_OF:
        fail(f"玩家子彈樣板 {t:X} 沒有武器對照")
    use("MY", info["idx"], "bullet", GAMEPLAY, "my-mask-and+plane-or(same y)", site=f"{b_sites[t][0]:X}",
        sub="player-shot", mask=True, mask_dy=0, func="sub_1691A", template=f"{t:04X}", weapon=wp[0], facing=wp[1],
        type=info["type"])
# 雷射 L：用 MY 第 4、25 項的 8x3 小段一路畫到命中點（sub_16A99、sub_16B18）
use("MY", 4, "bullet", GAMEPLAY, "my-plane-replace", site="16AAA", sub="laser-segment", func="sub_16A99",
    note="偶數頁用第 4 項、奇數頁用第 25 項，逐段往右（16A99）或往左（16B18）疊出雷射光束")
use("MY", 25, "bullet", GAMEPLAY, "my-plane-replace", site="16AB7", sub="laser-segment", func="sub_16A99",
    note="見第 4 項")

# ------------------------------------------------------------------ 4. 特效腳本（MY 爆炸）
# sub_17A3A 依代碼把腳本位址（4094h 等）交給 sub_17C6C，腳本每步 (MY 索引, dx, dy, 形狀指標)，索引 0Fh 結束。
EFFECT_CODE = {0x4094: "S 與 > 與 <（預設）", 0x40B2: "<", 0x40D0: "B", 0x40EE: "V", 0x413C: "E", 0x4162: "A", 0x4188: "G"}
eff_scripts = collections.OrderedDict()
for a in dis.order:
    if a in REACH and dis.ins[a][0] == "mov" and dis.ins[a][1].startswith("ax, "):
        v = imm(dis.ins[a][1].split(",")[1])
        n2 = dis.nxt.get(a)
        if v is not None and n2 and dis.ins[n2] == ("call", "sub_17C6C"):
            eff_scripts.setdefault(v, []).append(a)
for sa, sites_ in eff_scripts.items():
    steps, p = [], sa
    while W(p) != 0xF and len(steps) < 64:
        steps.append((W(p), SW(p + 2), SW(p + 4)))
        p += 8
    for k, (i, dx, dy) in enumerate(steps):
        use("MY", i, "explosion", GAMEPLAY, "my-mask-and+plane-or(same y)+sound", site=f"{sites_[0]:X}",
            sub="effect-script", mask=True, mask_dy=0, func="sub_17CB6", script=f"{sa:04X}", step=k,
            code=EFFECT_CODE.get(sa, "?"))
    family("MY", [s[0] for s in steps], f"effect-{sa:04X}", "explosion", f"腳本 {sa:04X}")
SCRIPTS = {f"{k:04X}": [s for s in eff_scripts] for k in []}

# ------------------------------------------------------------------ 5. 敵彈、碎片與道具池（dseg:32B3h，20 槽）
# sub_16D3B(代碼) 把樣板位址放進 word_200F3 再 sub_16CEA 複製入池。樣板第 1 字組是影格表指標（9 字組，依方向），
# 第 8 字組（word_20105）為 4Dh（M）時畫 MY 圖庫，否則畫目前關卡的 SHTn 圖庫（sub_17768）。
pool_sites = collections.defaultdict(list)
for a in dis.order:
    if a in REACH:
        mn, ops = dis.ins[a]
        if mn == "mov" and ops.startswith("word_200F3, ") and imm(ops.split(",")[1]) is not None:
            pool_sites[imm(ops.split(",")[1])].append(a)


def pool_tpl(t):
    tv = [W(t + 2 * i) for i in range(15)]
    fr = [W(tv[1] + 2 * i) for i in range(9)]
    return {"frames": fr, "flag": chv(tv[8]), "k0": tv[0]}


# 代碼 -> 該代碼會放進池的樣板位址（依 sub_16D3B、sub_1701E 等分派，證據是各位址的 mov word_200F3 呼叫點）
CODE_TPL = {
    "F": [0x3557, 0x3575, 0x3593, 0x35B1, 0x35CF, 0x35ED, 0x360B, 0x3629],
    "B": [0x3B15, 0x3B33, 0x3B51],
    "L": [0x3B6F, 0x3B8D, 0x3BAB, 0x3BC9, 0x3BE7],
    "V": [0x3C05], "1": [0x3C23], "2": [0x3C23],
    "X": [0x3C41, 0x3CF5, 0x3D13], "C": [0x3C5F, 0x3C7D, 0x3C9B],
    "U": [0x3D13, 0x3C5F], "D": [0x3C9B, 0x3CD7],
    "Q": [0x3D8B, 0x3DA9],
    "E": [0x3D6D, 0x3D4F, 0x3D31, 0x3557, 0x3575, 0x3593, 0x35B1, 0x35CF, 0x35ED, 0x360B, 0x3629],
    "l": [0x3737, 0x3755, 0x3773, 0x3791, 0x37AF, 0x37CD, 0x37EB, 0x3809],
    "b": [0x3827, 0x3845, 0x3863, 0x3881, 0x389F, 0x38BD, 0x38DB, 0x38F9],
    "k": [0x3917, 0x3935, 0x3953, 0x3971, 0x398F, 0x39AD, 0x39CB, 0x39E9],
    "s": [0x3A07, 0x3A25, 0x3A43, 0x3A61, 0x3A7F, 0x3A9D, 0x3ABB, 0x3AD9],
}
RING = [0x3C41, 0x3C5F, 0x3C7D, 0x3C9B, 0x3CB9, 0x3CD7, 0x3CF5, 0x3D13]
CODE_TPL["<"] = RING
CODE_TPL["?"] = RING
CODE_TPL["_default"] = [0x3557]
for c, ts in CODE_TPL.items():
    for t in ts:
        if t not in pool_sites:
            fail(f"敵彈樣板 {t:X}（代碼 {c}）在可達碼內沒有 mov word_200F3 呼叫點")

# 腳本位元組 0Bh 會直接發 B 型彈（sub_1606A 的 case 0Bh，@16054）：各樣板是否含 0Bh 在實體節處理。

# ------------------------------------------------------------------ 6. 關卡實體樣板
TPL = collections.OrderedDict()   # stage -> list of dict


def walk_script(p, limit=6000):
    """實體移動腳本（sub_15E68、sub_15ECD、sub_15FD9）：每幀讀一個位元組。
    F0 xx xx 跳到 xxxx 繼續；E0 xx xx 把實體換成位址 xxxx 的另一份樣板；0F 讓實體消失（走到終點）；
    0B 在該幀發 B 型彈（sub_15FD9 的 case 0Bh，@16054）。回傳終結種類、E0 目標與 0B 旗標。"""
    seen, stack = set(), [p]
    e0, term, has0b, steps = [], set(), False, 0
    while stack:
        a = stack.pop()
        while True:
            if a in seen:
                term.add("loop")
                break
            seen.add(a)
            steps += 1
            if steps > limit:
                term.add("overrun")
                break
            v = B(a)
            if v == 0xF0:
                seen.update((a + 1, a + 2))
                stack.append(W(a + 1))
                break
            if v == 0xE0:
                e0.append(W(a + 1))
                term.add("E0")
                break
            if v == 0x0F:
                term.add("0F")
                break
            if v == 0x0B:
                has0b = True
            a += 1
    return {"terminals": sorted(term), "e0": e0, "has_0B": has0b, "steps": steps}


BOSS_TPL = {
    "stage1": {0x43E4: "控制器", 0x45DE: "損壞貼片", 0x460C: "損壞貼片", 0x463A: "損壞貼片", 0x4668: "損壞貼片",
               0x4696: "損壞貼片", 0x46C4: "損壞貼片", 0x47AA: "核心"},
    "stage2": {0x59D4: "中頭目控制器", 0x5A5E: "中頭目損壞貼片", 0x5A8C: "中頭目損壞貼片", 0x5B16: "中頭目損壞貼片",
               0x5BA0: "中頭目砲口", 0x5A02: "中頭目控制器", 0x5ABA: "中頭目損壞貼片", 0x5AE8: "中頭目損壞貼片",
               0x5B44: "中頭目損壞貼片", 0x5BCE: "中頭目砲口", 0x5A30: "頭目控制器", 0x5B72: "頭目損壞貼片",
               0x5BFC: "頭目核心"},
    "stage3": {0x7168: "頭目砲塔", 0x7196: "頭目砲塔", 0x71C4: "頭目砲塔", 0x71F2: "頭目砲塔", 0x7220: "頭目砲塔",
               0x724E: "頭目核心"},
    "stage4_a": {0x8441: "頭目", 0x83E5: "中頭目"},
}
def read_tpl(tp, frames, via):
    tv = [W(tp + 2 * i) for i in range(24)]
    t1 = [W(tv[1] + 2 * i) for i in range(9)]
    t20 = [W(tv[20] + 2 * i) for i in range(9)] if tv[20] else None
    sc = walk_script(tv[13]) if tv[13] >= 0x100 else {"terminals": ["data"], "e0": [], "has_0B": False, "steps": 0}
    return {"addr": tp, "frames": frames, "via": via, "hp": tv[0], "fire_interval": tv[6], "code": chv(tv[7]),
            "tracking": chv(tv[8]), "death_effect": chv(tv[17]), "flag": chv(tv[18]), "drop": chv(tv[19]),
            "x0": tv[9] - (65536 if tv[9] >= 0x8000 else 0), "y0": tv[10] - (65536 if tv[10] >= 0x8000 else 0),
            "script": tv[13], "script_terminals": sc["terminals"], "script_steps": sc["steps"],
            "e0_targets": sc["e0"], "has_0B": sc["has_0B"], "frame_table": tv[1], "limit_table": tv[20] or None,
            "dir_table": t1, "limit": t20, "hit_shape": tv[14]}


for st, tl in TBL["timelines"].items():
    lib = STAGE_LIB[st]
    seen = collections.OrderedDict()
    for f, tp in tl["entries"]:
        seen.setdefault(tp, []).append(f)
    rd = tl["reader"]
    if dis.label[rd] not in REACH:
        fail(f"{st} 的時間表讀取端 {rd} 不可達")
    rows, queue, done = [], [(tp, fr, None) for tp, fr in seen.items()], set()
    while queue:
        tp, frames, via = queue.pop(0)
        if tp in done:
            continue
        done.add(tp)
        r = read_tpl(tp, frames, via)
        # 驗證：影格表的索引不得超出該關圖庫
        for v_ in r["dir_table"] + (r["limit"] or []):
            if v_ >= BANK[lib]["count"] and r["flag"] not in ("K", "N"):
                fail(f"{st} 樣板 {tp:X} 影格索引 {v_} 超出 {lib}")
        rows.append(r)
        for q in r["e0_targets"]:
            if q not in done:
                queue.append((q, frames, tp))
    TPL[st] = rows

DEATH_SCRIPT = {"B": 0x40D0, "S": 0x4094, "V": 0x40EE}   # 死亡代碼 -> 特效腳本（sub_17A3A 分派）
tpl_groups = []     # (lib, members, label, kind)
for st, rows in TPL.items():
    lib = STAGE_LIB[st]
    for r in rows:
        r["boss"] = BOSS_TPL.get(st, {}).get(r["addr"])
        r["sprites"] = []
        sc = stage_scene(st, r["frames"][0])
        scenes = sorted({stage_scene(st, f) for f in r["frames"]})
        sp_site = {"O": "1652B", "S": "16573"}.get(r["flag"])
        if r["flag"] in ("O", "S"):
            idxs = set()
            for d_ in range(9):
                b0 = r["dir_table"][d_]
                if r["limit"]:
                    lm = r["limit"][d_]
                    if lm < b0:
                        fail(f"{st} 樣板 {r['addr']:X} 方向 {d_} 動畫上限 {lm} < 基底 {b0}")
                        lm = b0
                    idxs.update(range(b0, lm + 1))
                else:
                    idxs.add(b0)
            r["sprites"] = sorted(idxs)
            role = "boss_part" if r["boss"] else ("text" if st == "stage4_b" else ("item" if r["drop"] == "P" else "enemy"))
            mdraw = "my-n/a"
            for i in r["sprites"]:
                for scn in scenes:
                    if r["flag"] == "S":
                        use(lib, i, role, scn, "sht-plane-replace (after players, opaque)", site="16573",
                            sub="opaque-foreground", func="sub_16541", stage=st, template=f"{r['addr']:04X}",
                            spawn_frames=r["frames"], hp=r["hp"], boss=r["boss"])
                    else:
                        use(lib, i, role, scn, "sht-mask-and(y)+plane-or(y+1), hit flash=mask-inverted-or",
                            site="1652B", sub=None, mask=True, mask_dy=-1, func="sub_164F3", stage=st,
                            template=f"{r['addr']:04X}", spawn_frames=r["frames"], hp=r["hp"], boss=r["boss"])
            tpl_groups.append((lib, r["sprites"], "boss" if r["boss"] else "enemy", st, r))
        elif r["flag"] == "K":
            # 不畫；死亡時 sub_15DBE 把 (索引, x, y) 畫進背景暫存區 B
            ov = r["dir_table"][:3]
            r["sprites"] = [ov[0]]
            use(lib, ov[0], "boss_part", stage_scene(st, r["frames"][0]), "bg-store-b-replace (on destroy)",
                site="15E15", sub="damage-overlay", func="sub_15DBE", stage=st, template=f"{r['addr']:04X}",
                overlay_x=ov[1], overlay_y=ov[2], hp=r["hp"], boss=r["boss"])
        else:   # N：不畫的控制器
            r["sprites"] = []
    # 死亡特效：每個樣板的 death_effect 決定 MY 腳本，MY 的 use 已由特效腳本節記錄，這裡只在樣板列留欄位
for st, rows in TPL.items():
    for r in rows:
        r["death_script"] = f"{DEATH_SCRIPT[r['death_effect']]:04X}" if r["death_effect"] in DEATH_SCRIPT else None

# 發彈：fire_interval != 99 的樣板，用 code 決定樣板池；腳本含 0Bh 者另發 B 型
STAGE_FIRE = {}
for st, rows in TPL.items():
    codes = collections.OrderedDict()
    for r in rows:
        if r["flag"] == "N":
            continue
        fs = r["frames"]
        if r["fire_interval"] != 99:
            codes.setdefault(r["code"], []).append((r["addr"], fs))
        if r["has_0B"]:
            codes.setdefault("B", []).append((r["addr"], fs))
    STAGE_FIRE[st] = codes
for st, codes in STAGE_FIRE.items():
    lib = STAGE_LIB[st]
    for c, tpls in codes.items():
        ts = CODE_TPL.get(c, CODE_TPL["_default"])
        scs = sorted({stage_scene(st, f) for _, fs in tpls for f in fs})
        for t in ts:
            pt = pool_tpl(t)
            frames = sorted(set(pt["frames"]))
            mylib = pt["flag"] == "M"
            for i in frames:
                for scn in scs:
                    use("MY" if mylib else lib, i, "item" if mylib else "bullet", scn,
                        "my-mask-and(y)+plane-or(y+1)" if mylib else "sht-mask-and+plane-or(same y)",
                        site=f"{pool_sites[t][0]:X}", sub="enemy-shot", mask=True, mask_dy=-1 if mylib else 0,
                        func="sub_17768", stage=st, code=c, pool_template=f"{t:04X}",
                        from_templates=sorted({f"{x:04X}" for x, _ in tpls}))

# 道具：載體（SHTn 第 8 項，f19=P）死亡時 sub_16D3B 依代碼放出 MY 的字母圓球
ITEM_CODES = {"l": 42, "b": 43, "k": 44, "s": 45}
for st, rows in TPL.items():
    for r in rows:
        if r["drop"] == "P" and r["code"] in ITEM_CODES:
            t = CODE_TPL[r["code"]][0]
            use("MY", ITEM_CODES[r["code"]], "item", GAMEPLAY, "my-mask-and(y)+plane-or(y+1)",
                site=f"{pool_sites[t][0]:X}", sub="item-ball", mask=True, mask_dy=-1, func="sub_17768", stage=st,
                code=r["code"], pool_template=f"{t:04X}", from_templates=[f"{r['addr']:04X}"],
                note="撿取判定：2C1Ch 格狀緩衝高半位元組 80h=L、40h=B、C0h=K、20h=S（14CD4、14D04）")

# 敵彈池在 sub_16D3B 內另有 A 型（3AF7h，影格表 [9]）。實體樣板沒有任何一個用 A 代碼，所以不計用途。

# ------------------------------------------------------------------ 7. 第一關岩塊（群組表 dseg:19DCh，sub_15C92）
grp_rows = TBL["stage1_group_table"]
rock_idx = set()
for f, p in grp_rows:
    rock_idx.add(W(p))
    spd = W(p + 6)
if dis.label["sub_15C92"] not in REACH:
    fail("sub_15C92 不可達")
for i in sorted(rock_idx):
    rows = [(f, W(p + 2), W(p + 4), W(p + 6)) for f, p in grp_rows if W(p) == i]
    use("SHT1", i, "background", "stage1-main", "sht-mask-and+plane-or(same y)", site="15D15", sub="drifting-rock",
        mask=True, mask_dy=0, func="sub_15CF0", spawn_frames=[r[0] for r in rows],
        speed_bytes_per_frame=rows[0][3], y_values=sorted({r[2] for r in rows}),
        note="群組表 19DCh 的 35 列，(幀, 指標)，指標指向 [索引, x, y, 速度]；繪製順序 16 → 15 → 14（遠到近）")
family("SHT1", sorted(rock_idx), "rocks", "background", "第一關漂浮岩塊三種大小，速度 4、2、1 位元組每幀")

# ------------------------------------------------------------------ 8. 背景暫存區的滾動層
# 每層的來源列、滾動量、顯示列與寫入函式見 docs/re/12-stages-presentation.md 第 3 節。
LAYER = [
    # (lib, idx, role, scene, sub, note)
    ("BH1", 1, "sky-gradient", "stage1-main", "固定不捲：來源列 6Eh、高 89，貼在 y=word_1E6BA；幀 >250 才隨 word_1E6BA 下移"),
    ("BH1", 0, "wave-layer", "stage1-main", "來源列 0、高 41；word_1E6BC 每幀 +2 位元組（16 像素）循環 40；貼在 y=word_1E6B8+95h"),
    ("BH1", 2, "rock-ceiling", "stage1-main", "來源列 28h、高 69；word_1E6BE 每幀 +1 位元組（8 像素）循環 40；貼在 y=word_1E6B8+52h"),
    ("BH1", 3, "mountain-art", "stage1-main", "來源列 C8h、高 43；以 BH1 第 4 項為遮罩 AND 後 OR；同 word_1E6BE"),
    ("BH1", 4, "mountain-mask", "stage1-main", "遮罩，來源列 F3h、高 43，用 GC 803h AND 清出山形；它是第 3 項的遮罩，不是獨立圖"),
]
for lib, i, sub, scn, note in LAYER:
    wr = {"BH1": {1: "10BDD", 0: "10B85", 2: "10BAF", 3: "10BFA", 4: "10C24"}}[lib][i]
    use(lib, i, "background", scn, "bg-store-a-replace -> latch-copy to screen (sub_16302)", site=wr, sub=sub,
        func="sub_16302", note=note)
use("BH1", 4, "background", "stage1-main", "bg-store-a AND-mask (GC 803h) via sub_1A09F", site="163AA",
    sub="mountain-mask", func="sub_16302", note="見上")
family("BH1", [3, 4], "mountain-masked", "background", "第 4 項是第 3 項的遮罩（黑白剪影），一起重繪")
BH2_NOTE = {
    4: ("sky-tile", "16x139 的磚，橫排 20 塊成 320 像素，來源列 70h；sub_1355B 靜態貼在 y=1Ah 高 140，不捲"),
    **{i: ("shift-phase-a", "16x9，同一圖樣向左位移 0～7 像素的 8 個相位；word_1D7C4 逐幀 7→0 選相位，sub_13580 貼 5 份在 y=66h、69h、4Dh，每 8 幀整體左移 1 位元組，共 1 像素每幀")
           for i in range(5, 13)},
    **{i: ("shift-phase-b", "16x10 的 4 個相位；sub_135EB，每份間隔 0Ah 位元組，2 像素每幀，貼在 y=72h、44h") for i in range(13, 17)},
    **{i: ("shift-phase-c", "16x14 的 2 個相位；sub_1365F，每份間隔 14h 位元組，4 像素每幀，貼在 y=7Ch、36h") for i in (17, 18)},
    **{i: ("masked-art-phase", "16x37 的 4 個相位，與 23～26 的遮罩相位配對；sub_136CA 在 y=4Dh 用 AND 遮罩再 OR 圖") for i in range(19, 23)},
    **{i: ("masked-mask-phase", "第 19～22 項的遮罩相位（sub_1A144 AND）") for i in range(23, 27)},
    27: ("pillar", "16x83 的柱，sub_13711 貼 2 份間隔 14h 位元組，8 像素每幀，來源在背景暫存區 B"),
    0: ("surface-art-a", "320x34，與第 2 項遮罩配對，偶數頁（page=8000h）；sub_13452 兩份：y=word_1D7BE 與鏡射在 y=word_1D7C0"),
    1: ("surface-art-b", "320x34，與第 3 項遮罩配對，奇數頁（page=0）"),
    2: ("surface-mask-a", "遮罩，來源列 44h，用 sub_1A09F 與 sub_1A1E6 AND"),
    3: ("surface-mask-b", "遮罩，來源列 5Ah"),
}
BH2_SITE = {0: "12FD6", 1: "13000", 2: "1302A", 3: "13054", 4: "13082"}
for i, (sub, note) in BH2_NOTE.items():
    site = BH2_SITE.get(i)
    if site is None:
        site = {5: "130A3", 13: "130CF", 17: "130F4", 19: "1311F", 23: "13144", 27: "13165"}[
            max(k for k in (5, 13, 17, 19, 23, 27) if k <= i)]
    use("BH2", i, "background", "stage2-main", "bg-store-replace -> latch-copy/OR/AND to screen", site=site, sub=sub,
        func="sub_12FB6", note=note)
family("BH2", range(5, 13), "shift-a", "background", "同一磚的 8 個像素位移相位")
family("BH2", range(13, 17), "shift-b", "background", "同一磚的 4 個像素位移相位")
family("BH2", (17, 18), "shift-c", "background", "同一磚的 2 個像素位移相位")
family("BH2", range(19, 27), "masked-layer", "background", "19～22 為圖相位，23～26 為對應遮罩相位")
family("BH2", (0, 1, 2, 3), "surface", "background", "0、1 為兩影格的圖，2、3 為兩影格的遮罩，頂底鏡射兩份")
family("BH2", (4,), "sky-tile", "background")
family("BH2", (27,), "pillar", "background")
use("BH4", 0, "background", "stage4", "bg-store-a-replace -> latch-copy (sub_13F7A)", site="13D93", sub="tunnel-wall",
    func="sub_13D70",
    note="320x107 通道壁：下壁來源列 0 貼在 y=96h+word_1D91D，速度 1 位元組每幀；上壁同來源以垂直鏡射（sub_19C4F）貼在"
         " y=28h+word_1D91D，另有 18 位元組的相位差；word_1D91D 依 0DE4h 腳本上下抖動 ±10")
# 第三關地圖圖塊
tile_use = collections.Counter()
for k in range(24000 // 2):
    tile_use[struct.unpack_from("<H", SCN, 2 * k)[0]] += 1
first_col = {}
for k in range(12000):
    t = struct.unpack_from("<H", SCN, 2 * k)[0]
    c = k % 600
    first_col[t] = min(first_col.get(t, 10 ** 9), c)
for t, n in sorted(tile_use.items()):
    if t >= BANK["MAP"]["count"]:
        fail(f"地圖圖塊 {t} 超出 MAP 圖庫")
        continue
    use("MAP", t, "background", "stage3", "map-tile 8x10 plane-replace", site="17E4B", sub="tile", func="sub_17DF9",
        count_in_map=n, first_column=first_col[t],
        note="SHT.SCN 的 u16 圖塊索引，600 欄 x 20 列；sub_182CD 每 2 個頁面畫入新的一欄，約每幀 4 像素")
# 圖塊索引 0 的特殊性：其他 owner 的 18ACE 呼叫點都在死碼（垂直捲動）或單色（185AA、18613）。

# ------------------------------------------------------------------ 9. 其餘固定圖庫的批次語意（第 2、3 節以外）
# BEG1 與 J、SHT5：已由 owner 掃描處理。以下補表驅動的 SHT5 片尾實體。
rows = TPL["stage4_b"]
for r in rows:
    pass   # 已在實體節處理（SHT5 第 0～2 項）

# ------------------------------------------------------------------ 10. 單色模式與未使用判定
# 此處不加用途，只在輸出 meta 說明。

# ------------------------------------------------------------------ 11. 圖庫逐項收尾：狀態、遮罩、調色盤、群組
for lib in LIBS:
    for i in range(BANK[lib]["count"]):
        entry(lib, i)

# 固定的家族（連續動畫、成對或成組）
family("MY", [7, 9, 11], "ship-p1", "player", "P1 機體：中立 9、向下 7、向上 11")
family("MY", [14, 16, 18], "ship-p2", "player", "P2 機體：中立 16、向上 14、向下 18")
family("MY", [12, 13], "exhaust", "player", "噴焰兩影格，依頁交替")
family("MY", [10, 17], "muzzle", "player", "槍口與砲座亮點")
family("MY", list(range(30, 35)), "pod-right", "player", "砲座，面向右，5 影格")
family("MY", list(range(36, 41)), "pod-left", "player", "砲座，面向左，5 影格")
family("MY", [49, 50, 51], "charge-orb-mid", "player", "蓄力 6～13 級，3 影格")
family("MY", [52, 53, 54], "charge-orb-full", "player", "蓄力 ≥14 級，3 影格")
family("MY", [5, 60], "shot-crescent", "bullet", "K 武器彈，右 5、左 60")
family("MY", [4, 25], "laser-segment", "bullet", "L 武器雷射小段，兩影格")
family("MY", [42, 43, 44, 45], "item-balls", "item", "L、B、K、S 字母圓球")
family("MY", [61, 62, 63, 64, 65, 66, 67], "act-card", "text", "ACT 與關卡數字")
family("MY", [15, 47, 48], "title-credits", "text", "標題最後的 SOFTSTAR 標誌、大宇資訊、製作名單")
family("MY", [68, 69, 70, 71, 72], "disk-prompt", "text", "磁碟片提示")
family("MY", list(range(77, 86)), "options-labels", "text", "選單字樣")
family("MY", [73, 74, 75, 76], "options-portraits", "ui", "難度頭像 4 表情")
family("MY", [0, 19, 20], "loader-bar", "unused", "已停用的載入進度條")
family("BEG1", [14, 15, 16, 17], "title-logo", "cutscene", "RS-2 字樣，最後的 S 變 2 的 3 影格")
family("BEG1", [2, 3, 4, 5, 6], "ship-front-zoom", "cutscene", "太空船正面由小到大 5 級")
family("BEG1", [8, 9, 10, 11, 12, 13, 18, 19, 20], "ship-approach", "cutscene", "太空船接近 9 級")
family("BEG1", list(range(21, 29)), "ship-exhaust", "cutscene", "噴焰與光點 8 影格")
family("BEG1", [29, 30, 31], "planet-flash", "cutscene", "行星與閃光 3 級")
family("BEG1", [0], "title-eyes", "cutscene")
family("BEG1", [1], "title-hand", "cutscene")
family("BEG1", [7], "title-earth", "cutscene")
family("SHT1", [17, 18, 19, 20, 21, 22, 23, 24, 33, 34, 35], "boss", "boss_part", "第一關頭目：合成體 33～35、損壞貼片 19～24、核心 17～18")
family("SHT2", [18, 19, 20, 26, 27, 28, 32, 33], "midboss", "boss_part", "第二關中頭目：合成體 32、33；損壞貼片 26～28；砲口 18～20")
family("SHT2", [23, 24, 25, 29, 30, 34, 35], "boss", "boss_part", "第二關頭目：合成體 34、35；損壞貼片 23；疊圖動畫 24、25 與 29、30")
family("SHT3", [37, 38, 39, 40, 41, 42, 43], "boss", "boss_part", "第三關頭目：合成體 43 加六個砲塔分件 37～42")
family("SHT5", [0, 1, 2], "end-letters", "cutscene", "END 三個字母")
family("MAP", range(BANK["MAP"]["count"]), "tiles", "background", "第三關圖塊集，8x10，整圖設計後再切")
family("MY", [26], "banner", "text", "大宇資訊 1990 字樣條")
family("MY", [8], "pickup-lightning", "effect", "撿到武器字母的閃電")
family("MY", [1], "shot-normal", "bullet", "預設、S、L 的細彈")
family("MY", [55], "shot-charge", "bullet", "蓄力滿級的垂直光牆")

# 實體在腳本 E0 處換成另一份樣板（變形、放大），整條鏈的索引屬同一組
for st, rows in TPL.items():
    by_addr = {r["addr"]: r for r in rows}
    for r in rows:
        for q in r["e0_targets"]:
            if q in by_addr and r["sprites"] and by_addr[q]["sprites"]:
                family(STAGE_LIB[st], r["sprites"] + by_addr[q]["sprites"], f"chain-{r['sprites'][0]}", "enemy",
                       f"{STAGE_LABEL[st]} 樣板 {r['addr']:04X} 經腳本 E0 換成 {q:04X}")

# 把 SHT 家族套上實體樣板（同一樣板的所有索引屬同組）
for lib, members, kind, st, r in tpl_groups:
    if len(members) > 1:
        family(lib, members, f"{kind}-{members[0]}", kind, f"{STAGE_LABEL[st]} 樣板 {r['addr']:04X}")

# union-find
parent = {}


def find(x):
    parent.setdefault(x, x)
    while parent[x] != x:
        parent[x] = parent[parent[x]]
        x = parent[x]
    return x


for f in FAMILIES:
    ks = [(f["lib"], i) for i in f["members"]]
    for k in ks[1:]:
        parent[find(k)] = find(ks[0])
    find(ks[0])
comp = collections.defaultdict(list)
for k in list(parent):
    comp[find(k)].append(k)
GROUPS = collections.OrderedDict()
gid_of = {}
label_pref = {}
for f in FAMILIES:    # 以先出現的標籤為組名（人工家族在前）
    r = find((f["lib"], f["members"][0]))
    label_pref.setdefault(r, f)
for r, ks in sorted(comp.items(), key=lambda kv: (kv[0][0], min(i for _, i in kv[1]))):
    lib = r[0]
    mem = sorted(i for _, i in ks)
    f = label_pref[r]
    gid = f"{lib}:{f['label']}"
    if gid in GROUPS:
        gid = f"{gid}-{mem[0]}"
    GROUPS[gid] = {"lib": lib, "members": mem, "kind": f["kind"], "note": f["note"]}
    for i in mem:
        gid_of[(lib, i)] = gid

# 實體樣板彙總（輸出用）
TPL_OUT = {}
for st, rows in TPL.items():
    TPL_OUT[st] = [
        {"addr": f"{r['addr']:04X}", "spawn_frames": r["frames"], "hp": r["hp"], "fire_interval": r["fire_interval"],
         "code": r["code"], "death_effect": r["death_effect"], "death_script": r["death_script"], "flag": r["flag"],
         "drop": r["drop"], "x0": r["x0"], "y0": r["y0"], "script": f"{r['script']:04X}",
         "script_steps": r["script_steps"], "script_terminals": r["script_terminals"], "e0_targets": [f"{x:04X}" for x in r["e0_targets"]], "via": (f"{r['via']:04X}" if r["via"] else None), "has_0B": r["has_0B"],
         "dir_table": r["dir_table"], "limit": r["limit"], "sprites": r["sprites"], "boss": r["boss"],
         "lib": STAGE_LIB[st]} for r in rows]

# 敵彈與道具的共用槽位：樣板的影格表固定索引，各關圖庫在該索引放自己的圖（見 pool 樣板）
POOL_SLOT = {0: "P 球槽（僅死碼樣板 3647h～3719h 參照）", 1: "F 型瞄準彈槽", 7: "L、X、C、U、D、Q、E、環型彈槽",
             9: "A 型彈槽（3AF7h，沒有實體樣板發 A）", 13: "B 型彈槽", 15: "1、2 型彈槽", 29: "V 型彈槽", 8: "道具載體槽（f19=P 的樣板）"}
dead_pool = {}
for a in dis.order:
    if a not in REACH:
        mn, ops = dis.ins[a]
        if mn == "mov" and ops.startswith("word_200F3, ") and imm(ops.split(",")[1]) is not None:
            t_ = imm(ops.split(",")[1])
            if t_ in (0x3647, 0x3665, 0x3683, 0x36A1, 0x36BF, 0x36DD, 0x36FB, 0x3719):
                dead_pool.setdefault(t_, []).append(f"{a:X}")
if not dead_pool:
    fail("預期找到不可達的 3647h～3719h 呼叫點")
NOTES = {
    ("MY", 0): "原載入進度條的底圖。進度條函式 nullsub_4（1CB55）被改成 retn，本體 1CB56 起不可達（呼叫點 1CB72）",
    ("MY", 19): "同上，進度條格（1CB8F）",
    ("MY", 20): "同上，進度條格（1CB75、1CB80、1CB9E、1CBA3、1CBC4）",
    ("MY", 35): "砲座影格的鄰項。程式只用 30～34 與 36～40，動畫計數上限 5（14940、149A0）",
    ("MY", 41): "同上",
    ("MY", 46): "沒有字母的圓球，尺寸與 42～45 相同。可達碼沒有任何參照",
    ("SHT1", 32): "144x25 的白色閃電光束。可達碼沒有任何參照，不確定是否為頭目雷射的遺留",
}
for lib_ in ("SHT1", "SHT2", "SHT3"):
    NOTES.setdefault((lib_, 0), "P 球槽。只有 3647h～3719h 的敵彈樣板（影格表 [0]）參照，其呼叫點 " + "、".join(sorted({x for v in dead_pool.values() for x in v})[:3]) + " 等在 1724Ch 之後的死碼")

def infer_source(u):
    if "pool_template" in u:
        return "pool-template"
    if "script" in u and u.get("func") == "sub_17CB6":
        return "effect-script"
    if "template" in u and u.get("func") == "sub_1691A":
        return "player-bullet-template"
    if "template" in u:
        return "entity-template"
    if "count_in_map" in u:
        return "map"
    if u.get("func") == "sub_15CF0":
        return "group-table"
    if u.get("func") in DYNAMIC_OWNERS or "pose" in u or "level" in u or "note" in u and "索引 =" in u.get("note", ""):
        return "variable"
    return "fixed"


# ------------------------------------------------------------------ 12. 條目收尾
ENTRIES = []
unresolved = []
role_count = collections.Counter()
for lib in LIBS:
    for i in range(BANK[lib]["count"]):
        e = entry(lib, i)
        reach_uses = [u for u in e["uses"] if u["reachable"]]
        roles = sorted(set(e["roles"]))
        sz = e["size"]
        placeholder = is_placeholder(lib, i)
        if reach_uses:
            role = roles[0] if len(roles) == 1 else None
            status = "used"
            if role is None:
                # 多角色：取最先登場的實體角色，其餘列在 roles
                pri = ["boss_part", "enemy", "player", "bullet", "explosion", "item", "effect", "background", "cutscene",
                       "ui", "text", "placeholder", "unused"]
                role = sorted(roles, key=lambda r_: pri.index(r_) if r_ in pri else 99)[0]
            if role == "unused":
                status = "unused"
            if role == "placeholder":
                status = "placeholder"
        else:
            status = "placeholder" if placeholder else "unused"
            role = "placeholder" if placeholder else "unused"
            roles = []
        m = mask_info(lib, i)
        mk = {"present": m is not None, "index": i if m else None, "size": m["size"] if m else None,
              "used": e["mask_used"], "plane_dy_vs_mask": None}
        if e["mask_used"]:
            dys = sorted(-d_ for d_ in e["mask_dy"] if d_ is not None)
            mk["plane_y_minus_mask_y"] = dys   # 平面 y 減遮罩 y：1 表示平面畫在遮罩下一列（先遮罩、再 y+1 畫平面）
            if m is None:
                mk["warning"] = "畫法會取遮罩但該索引沒有遮罩項"
                unresolved.append({"lib": lib, "index": i, "issue": "mask-missing"})
        del mk["plane_dy_vs_mask"]
        scenes = sorted(e["scenes"])
        pals = []
        for s_ in scenes:
            for p_ in SCENES[s_]["palettes"]:
                if p_ not in pals:
                    pals.append(p_)
        if not pals and lib in ("MY",) and status == "used":
            pals = []
        gid = gid_of.get((lib, i))
        if gid is None and status in ("used",):
            gid = f"{lib}:single-{i}"
            GROUPS[gid] = {"lib": lib, "members": [i], "kind": role, "note": None}
        if gid is None and status == "placeholder":
            gid = f"{lib}:placeholder"
            GROUPS.setdefault(gid, {"lib": lib, "members": [], "kind": "placeholder", "note": "高 1 列的佔位圖"})
            GROUPS[gid]["members"].append(i)
        for u_ in e["uses"]:
            u_["source"] = infer_source(u_)
        out = {"lib": lib, "index": i, "size": sz, "status": status, "role": role, "roles": roles,
               "group": gid, "mask": mk, "scenes": scenes, "palettes": pals,
               "draw_modes": sorted(e["draw_modes"]), "uses": e["uses"]}
        if status == "unused" and placeholder:
            out["status"] = "placeholder"
            out["role"] = "placeholder"
        nt = NOTES.get((lib, i))
        if nt is None and out["status"] == "unused" and lib.startswith("SHT") and i in POOL_SLOT:
            nt = f"共用槽位「{POOL_SLOT[i]}」。本關沒有樣板會用到這個槽位的代碼"
        if nt and out["status"] != "used":
            out["note"] = nt
        ENTRIES.append(out)
        role_count[out["role"]] += 1
        if not reach_uses and status == "unused":
            unresolved.append({"lib": lib, "index": i, "issue": "no-reachable-use"})
        if status == "used" and not pals:
            unresolved.append({"lib": lib, "index": i, "issue": "palette-depends-on-caller"})

# 地圖圖塊太多，entries 內仍保留，但 uses 只留一筆摘要
for e in ENTRIES:
    if e["lib"] == "MAP" and e["uses"]:
        e["uses"] = [{k: v for k, v in e["uses"][0].items() if k in ("site", "func", "count_in_map", "first_column", "scene", "source")}]

# 欄位為 null 的統計：沒有 palette 與 group 的已用項
nulls = collections.Counter()
for e in ENTRIES:
    if e["status"] == "used":
        if not e["palettes"]:
            nulls["palettes"] += 1
        if e["group"] is None:
            nulls["group"] += 1

meta = {
    "generator": "tools/export_sprite_usage.py",
    "inputs": {
        "s_disasm.txt": hashlib.sha256(open(os.path.join(ws, "ida", "s_disasm.txt"), "rb").read()).hexdigest(),
        "S.unpacked.bin": hashlib.sha256(IMG).hexdigest(),
        "manifest.json": hashlib.sha256(open(os.path.join(ws, "extracted", "manifest.json"), "rb").read()).hexdigest(),
        "stage-tables.json": hashlib.sha256(open(os.path.join(ws, "extracted", "stage-tables.json"), "rb").read()).hexdigest(),
        "SHT.SCN.bin": hashlib.sha256(SCN).hexdigest(),
    },
    "address_convention": "位址為 IDA 線性位址十六進位；dseg:X = 1CBC0h + X；site 是 blit 呼叫點，reachable 依指令層級可達性",
    "reachability_seeds": ["10000", "1161B（11619h 的自修改跳躍被補成 NOP 後的落點）"],
    "reachable_instructions": len(REACH),
    "roles": ["player", "enemy", "boss_part", "bullet", "explosion", "item", "effect", "background", "ui", "text",
              "cutscene", "placeholder", "unused"],
    "role_counts": dict(role_count),
    "palette_note": "MY 的色號 1～4 與 8～14 隨關卡調色盤變色，palettes 欄列出該圖會遇到的所有調色盤",
}
doc = {"meta": meta, "scenes": SCENES, "libraries": {lib: {"count": BANK[lib]["count"],
                                                          "masks": len(BANK[lib]["masks"]),
                                                          "default_preview_palette": BANK[lib]["palette_default"]}
                                                      for lib in LIBS},
       "groups": GROUPS, "entity_templates": TPL_OUT, "stage_fire_codes": {k: {c: sorted({f"{x:04X}" for x, _ in v}) for c, v in d_.items()} for k, d_ in STAGE_FIRE.items()},
       "unresolved": unresolved, "entries": ENTRIES}

if FAIL:
    sys.stderr.write("\n".join(FAIL) + "\n")
    sys.exit(2)
with open(out_path, "w", encoding="utf-8") as fh:
    json.dump(doc, fh, ensure_ascii=False, indent=1, sort_keys=False)
print("entries", len(ENTRIES), "groups", len(GROUPS), "roles", dict(role_count), "unresolved", len(unresolved), "nulls", dict(nulls))
