#!/usr/bin/env python3
"""驗證 rs2dyn 的完整逐幀收據（trace-NAME.jsonl）。

  python -I check_trace.py FILE [--expect-rows N] [--expect-first-frame F]
                                [--expect-last-frame F] [--json] [--diff OTHER] [--lck DMn.LCK]

檢查項目（任何一項失敗就以非 0 結束）：
  1. 每行是合法 JSON，頂層鍵與型別與 docs/re/10-dynamic-receipts.md 的欄位表一致，數值在 u16 範圍。
  2. 陣列長度：ent_t 15、pb_t 20、eb_t 20、fx_* 30、g_* 10、bg 4；
     記錄字組數：ent 24、pb 16、eb 15。
  3. 指標與記錄一一對應：非 0 指標的槽恰有一筆記錄，指標等於 池起點 + 槽號 x 記錄大小。
  4. n 從 0 連續遞增；emu.incs 與 n 的差在整個檔案內是常數（幀不跳號、不重複）。
  5. 幀計數連續：每筆比前一筆 +1。容許的例外只有三種，各自要在欄位上有根據：
       第一關頭目段回捲（650 之後接 649，pt 10fe3）、
       展示模式整關重置（650 之後接 1，misc.demo = 89）、
       關卡切換（misc.stage 與前一筆不同）、第三關頭目段幀計數釘在 475h（13BBBh 取樣，stage 63h）。
  6. pt（取樣點）與關卡（misc.stage）相符，第一關的 10f98 只在幀 <= 621，10fe3 只在幀 >= 622。

--lck FILE：展示模式用，驗 in1 等於 FILE[frame-1] and 3Fh。
--diff OTHER：逐行比對兩份檔案，忽略 emu（模擬器專屬欄位），回報第一個不同的筆數與欄位路徑。

只用標準函式庫。
"""
import hashlib
import json
import sys
from collections import Counter, OrderedDict

POOLS = {
    # 名稱: (指標表鍵, 記錄鍵, 槽數, 字組數, 池起點, 記錄大小)
    "ent": ("ent_t", "ent", 15, 24, 0x22B4, 0x30),
    "pb": ("pb_t", "pb", 20, 16, 0x262F, 0x20),
    "eb": ("eb_t", "eb", 20, 15, 0x32DB, 0x1E),
}
PLAYER_KEYS = ["col", "row", "x", "y", "img", "hit", "inv", "die", "hp", "wpn", "pod1", "pod2", "face", "chg"]
MISC_KEYS = ["seek", "dhp", "dfire", "boom", "demo", "stage", "nodmg", "fx0", "fx1", "fx2"]
EMU_KEYS = ["step", "ticks", "incs"]
TOP_KEYS = ["n", "frame", "pt", "page", "in1", "in2", "sc", "alive", "clear", "p1", "p2", "bg",
            "ent_t", "ent", "pb_t", "pb", "eb_t", "eb", "fx_x", "fx_y", "fx_a",
            "g_st", "g_x", "g_y", "g_dx", "misc", "emu"]
ARR_LEN = {"bg": 4, "ent_t": 15, "pb_t": 20, "eb_t": 20, "fx_x": 30, "fx_y": 30, "fx_a": 30,
           "g_st": 10, "g_x": 10, "g_y": 10, "g_dx": 10}
# misc.stage 是 byte_1CFDB：'A'..'D' = 41h..44h，第三關頭目段改成 63h
STAGE_PTS = {0x41: {"10f98", "10fe3"}, 0x42: {"1333c", "13390"}, 0x43: {"13b5f", "13bbb"}, 0x44: {"13ef4", "13f42"},
             0x63: {"13b5f", "13bbb"}}  # 63h：第三關頭目段（13BD3h 把 byte_1CFDB 改成 'c'）
DEMO_Y = 0x59  # 'Y'


def is_u(v, hi=0xFFFF):
    return isinstance(v, int) and not isinstance(v, bool) and 0 <= v <= hi


def check_row(o, errs, ln):
    def e(msg):
        errs.append("第 %d 行：%s" % (ln, msg))

    if not isinstance(o, dict):
        e("頂層不是物件")
        return False
    if list(o.keys()) != TOP_KEYS:
        e("頂層鍵不符：缺 %s，多 %s，或順序不同" % (sorted(set(TOP_KEYS) - set(o)), sorted(set(o) - set(TOP_KEYS))))
        return False
    for k in ("n", "frame", "page", "in1", "in2", "alive", "clear"):
        if not is_u(o[k], 0xFFFFFFFF if k == "n" else 0xFFFF):
            e("%s 不是 u16 整數：%r" % (k, o[k]))
    if not isinstance(o["pt"], str) or o["pt"] not in {p for s in STAGE_PTS.values() for p in s}:
        e("pt 不是已知取樣點：%r" % (o["pt"],))
    if not isinstance(o["sc"], list) or not all(is_u(v, 0xFF) for v in o["sc"]):
        e("sc 不是 u8 陣列")
    for pk in ("p1", "p2"):
        p = o[pk]
        if not isinstance(p, dict) or list(p.keys()) != PLAYER_KEYS or not all(is_u(v) for v in p.values()):
            e("%s 欄位不符" % pk)
    for k, n in ARR_LEN.items():
        a = o[k]
        if not isinstance(a, list) or len(a) != n or not all(is_u(v) for v in a):
            e("%s 長度或型別不符（預期 %d 個 u16）" % (k, n))
    for name, (tk, rk, slots, words, base, size) in POOLS.items():
        t, recs = o[tk], o[rk]
        if not isinstance(recs, list):
            e("%s 不是陣列" % rk)
            continue
        live = [i for i, p in enumerate(t) if isinstance(p, int) and p != 0] if isinstance(t, list) else []
        got = []
        for r in recs:
            if not isinstance(r, dict) or list(r.keys()) != ["s", "w"]:
                e("%s 記錄的鍵不是 [s, w]" % rk)
                continue
            if not (isinstance(r["w"], list) and len(r["w"]) == words and all(is_u(v) for v in r["w"])):
                e("%s 槽 %r 的字組數不是 %d" % (rk, r["s"], words))
            got.append(r["s"])
        if got != live:
            e("%s 記錄的槽 %s 與指標表非 0 的槽 %s 不一致" % (rk, got, live))
        for i in live:
            if t[i] != base + i * size:
                e("%s 槽 %d 的指標 %04X 不等於 %04X" % (tk, i, t[i], base + i * size))
    m = o["misc"]
    if not isinstance(m, dict) or list(m.keys()) != MISC_KEYS or not all(is_u(v) for v in m.values()):
        e("misc 欄位不符")
    em = o["emu"]
    if not isinstance(em, dict) or list(em.keys()) != EMU_KEYS or not all(is_u(v, 1 << 63) for v in em.values()):
        e("emu 欄位不符")
    return not any(x.startswith("第 %d 行" % ln) for x in errs)


def load(path):
    rows, errs = [], []
    with open(path, "r", encoding="utf-8") as f:
        for ln, line in enumerate(f, 1):
            if not line.endswith("\n"):
                errs.append("第 %d 行：沒有換行結尾（檔案可能被截斷）" % ln)
            try:
                o = json.loads(line, object_pairs_hook=OrderedDict, parse_constant=lambda c: (_ for _ in ()).throw(ValueError(c)))
            except ValueError as ex:
                errs.append("第 %d 行：JSON 解析失敗：%s" % (ln, ex))
                continue
            rows.append((ln, o))
    return rows, errs


def core_hash(rows):
    h = hashlib.sha256()
    for _, o in rows:
        c = OrderedDict((k, v) for k, v in o.items() if k != "emu")
        h.update(json.dumps(c, separators=(",", ":")).encode() + b"\n")
    return h.hexdigest()


def first_diff(a, b, path=""):
    if type(a) != type(b):
        return path or "/"
    if isinstance(a, dict):
        for k in a:
            if k not in b:
                return path + "/" + k
            d = first_diff(a[k], b[k], path + "/" + k)
            if d:
                return d
        for k in b:
            if k not in a:
                return path + "/" + k
        return None
    if isinstance(a, list):
        for i in range(min(len(a), len(b))):
            d = first_diff(a[i], b[i], "%s[%d]" % (path, i))
            if d:
                return d
        return None if len(a) == len(b) else path + "[len]"
    return None if a == b else path


def analyze(rows, errs):
    st = {"rows": len(rows)}
    if not rows:
        return st
    prev = None
    d0 = None
    exc = Counter()
    pts = Counter()
    page_breaks = 0
    max_live = {"ent": 0, "pb": 0, "eb": 0}
    max_slot = {"eb": -1, "pb": -1, "ent": -1}
    hist_eb = Counter()
    eb_ge15_rows = 0
    per_stage = {}
    for k, (ln, o) in enumerate(rows):
        if o["n"] != k:
            errs.append("第 %d 行：n = %d，預期 %d" % (ln, o["n"], k))
        d = o["emu"]["incs"] - o["n"]
        if d0 is None:
            d0 = d
            st["incs_minus_n"] = d
        elif d != d0:
            errs.append("第 %d 行：emu.incs - n = %d，與前面的 %d 不同（幀被跳過或重複）" % (ln, d, d0))
            d0 = d
        pts[o["pt"]] += 1
        stg = o["misc"]["stage"]
        if stg not in STAGE_PTS:
            errs.append("第 %d 行：misc.stage = %d 不是 41h..44h 或 63h" % (ln, stg))
        elif o["pt"] not in STAGE_PTS[stg]:
            errs.append("第 %d 行：取樣點 %s 不屬於關卡 %02Xh" % (ln, o["pt"], stg))
        if o["pt"] == "10f98" and o["frame"] > 0x26D:
            errs.append("第 %d 行：10f98 出現在幀 %d（> 621）" % (ln, o["frame"]))
        if o["pt"] == "10fe3" and o["frame"] < 0x26E:
            errs.append("第 %d 行：10fe3 出現在幀 %d（< 622）" % (ln, o["frame"]))
        if prev is not None:
            pf, cf = prev["frame"], o["frame"]
            if cf == pf + 1:
                pass
            elif prev["misc"]["stage"] != stg:
                exc["stage_change"] += 1
            elif pf == 0x28A and cf == 0x289 and o["pt"] == "10fe3":
                exc["boss_wrap_650_to_649"] += 1
            elif pf == 0x28A and cf == 1 and o["misc"]["demo"] == DEMO_Y:
                exc["demo_reset"] += 1
            elif pf == cf == 0x475 and o["pt"] == "13bbb" and stg == 0x63:
                exc["stage3_boss_clamp_475"] += 1  # 13BCDh 起：遞增後 > 474h 就釘回 474h
            else:
                errs.append("第 %d 行：幀計數 %d 接在 %d 之後（跳號或重複）" % (ln, cf, pf))
            if o["page"] == prev["page"]:
                page_breaks += 1
        prev = o
        for name, (tk, rk, slots, words, base, size) in POOLS.items():
            live = [i for i, p in enumerate(o[tk]) if p]
            max_live[name] = max(max_live[name], len(live))
            if live:
                max_slot[name] = max(max_slot[name], live[-1])
        live_eb = [i for i, p in enumerate(o["eb_t"]) if p]
        hist_eb[len(live_eb)] += 1
        if live_eb and live_eb[-1] >= 15:
            eb_ge15_rows += 1
        ps = per_stage.setdefault(stg, {"rows": 0, "max_eb_live": 0, "max_eb_slot": -1, "eb_slot_ge15_rows": 0,
                                         "max_pb_live": 0, "max_ent_live": 0, "min_frame": o["frame"], "max_frame": o["frame"]})
        ps["rows"] += 1
        ps["max_eb_live"] = max(ps["max_eb_live"], len(live_eb))
        if live_eb:
            ps["max_eb_slot"] = max(ps["max_eb_slot"], live_eb[-1])
        if live_eb and live_eb[-1] >= 15:
            ps["eb_slot_ge15_rows"] += 1
        ps["max_pb_live"] = max(ps["max_pb_live"], sum(1 for p in o["pb_t"] if p))
        ps["max_ent_live"] = max(ps["max_ent_live"], sum(1 for p in o["ent_t"] if p))
        ps["min_frame"] = min(ps["min_frame"], o["frame"])
        ps["max_frame"] = max(ps["max_frame"], o["frame"])
    # 過關事件：clear（byte_1CFAD）離開 N 的第一筆、連爆計數（byte_1CFAE）、核心（w7 = 45h 'E'）的生死
    ev = {}
    cl = next(((o["n"], o["frame"], o["clear"]) for _, o in rows if o["clear"] != 0x4E), None)
    if cl:
        ev["clear_first_row"] = {"n": cl[0], "frame": cl[1], "value": cl[2]}
    booms = {}
    for _, o in rows:
        booms.setdefault(o["misc"]["boom"], (o["n"], o["frame"]))
    if len(booms) > 1:
        ev["boom_first_rows"] = {str(k): {"n": v[0], "frame": v[1]} for k, v in sorted(booms.items()) if k in (1, 2, max(booms))}
        ev["boom_max"] = max(booms)
    core_rows = [(o["n"], o["frame"], r["w"]) for _, o in rows for r in o["ent"] if r["w"][7] == 0x45 and r["w"][18] == 0x4F]
    if core_rows:
        ev["core_e"] = {"first_row": {"n": core_rows[0][0], "frame": core_rows[0][1]},
                        "last_alive_row": {"n": core_rows[-1][0], "frame": core_rows[-1][1]},
                        "hp_field": core_rows[-1][2][0], "damage_low_byte_at_last_alive": core_rows[-1][2][16] & 0xFF,
                        "first_row_without_core": core_rows[-1][0] + 1 if core_rows[-1][0] + 1 < len(rows) else None}
    if ev:
        st["clear_events"] = ev
    st.update({
        "first_frame": rows[0][1]["frame"], "last_frame": rows[-1][1]["frame"],
        "points": dict(sorted(pts.items())), "exceptions": dict(exc), "page_not_alternating": page_breaks,
        "max_live": max_live, "max_slot_index": max_slot,
        "enemy_bullet_live_histogram": dict(sorted(hist_eb.items())),
        "enemy_bullet_rows_with_slot_ge15": eb_ge15_rows,
        "per_stage": {("%02X" % k): v for k, v in sorted(per_stage.items())},
        "scancode_events": sum(len(o["sc"]) for _, o in rows), "scancodes_seen": sorted({k for _, o in rows for k in o["sc"]}),
        "hp1_min": min(o["p1"]["hp"] for _, o in rows), "hp2_min": min(o["p2"]["hp"] for _, o in rows),
        "inputs_seen": sorted({o["in1"] for _, o in rows}),
        "clear_values": sorted({o["clear"] for _, o in rows}),
        "core_sha256": core_hash(rows),
    })
    return st


def main(argv):
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 2
    path = argv[0]
    opts = {}
    other = None
    i = 1
    while i < len(argv):
        a = argv[i]
        if a in ("--expect-rows", "--expect-first-frame", "--expect-last-frame"):
            opts[a] = int(argv[i + 1])
            i += 2
        elif a == "--diff":
            other = argv[i + 1]
            i += 2
        elif a == "--lck":
            opts[a] = argv[i + 1]
            i += 2
        elif a == "--json":
            opts[a] = True
            i += 1
        else:
            print("不認得的參數 %s" % a, file=sys.stderr)
            return 2
    rows, errs = load(path)
    for ln, o in rows:
        check_row(o, errs, ln)
    st = analyze(rows, errs) if not errs or len(errs) < 50 else {"rows": len(rows)}
    if "--expect-rows" in opts and len(rows) != opts["--expect-rows"]:
        errs.append("筆數 %d，預期 %d" % (len(rows), opts["--expect-rows"]))
    if rows and "--expect-first-frame" in opts and rows[0][1]["frame"] != opts["--expect-first-frame"]:
        errs.append("第一筆幀計數 %d，預期 %d" % (rows[0][1]["frame"], opts["--expect-first-frame"]))
    if rows and "--expect-last-frame" in opts and rows[-1][1]["frame"] != opts["--expect-last-frame"]:
        errs.append("最後一筆幀計數 %d，預期 %d" % (rows[-1][1]["frame"], opts["--expect-last-frame"]))
    with open(path, "rb") as f:
        st["file_sha256"] = hashlib.sha256(f.read()).hexdigest()
    if other:
        orows, oerrs = load(other)
        diff = None
        for (la, a), (lb, b) in zip(rows, orows):
            a2 = OrderedDict((k, v) for k, v in a.items() if k != "emu")
            b2 = OrderedDict((k, v) for k, v in b.items() if k != "emu")
            p = first_diff(a2, b2)
            if p:
                diff = "第 %d 筆（n=%s，frame=%s）欄位 %s 不同" % (la, a.get("n"), a.get("frame"), p)
                break
        if diff is None and len(rows) != len(orows):
            diff = "筆數不同：%d 對 %d" % (len(rows), len(orows))
        st["diff_vs_other"] = diff or "相同（忽略 emu）"
        if diff or oerrs:
            errs.append("與 %s 比對：%s" % (other, diff or oerrs[0]))
    if "--lck" in opts:
        with open(opts["--lck"], "rb") as f:
            lck = f.read()
        bad = [o["frame"] for _, o in rows if o["frame"] - 1 >= len(lck) or o["in1"] != (lck[o["frame"] - 1] & 0x3F)]
        st["lck_match"] = "%d/%d" % (len(rows) - len(bad), len(rows))
        if bad:
            errs.append("in1 與錄影檔不符的幀（前 10 個）：%s" % bad[:10])
    st["errors"] = len(errs)
    if "--json" in opts:
        print(json.dumps(st, ensure_ascii=False, indent=1))
    else:
        for k, v in st.items():
            print("%s: %s" % (k, json.dumps(v, ensure_ascii=False) if isinstance(v, (dict, list)) else v))
    for x in errs[:30]:
        print("錯誤：" + x, file=sys.stderr)
    if len(errs) > 30:
        print("錯誤：另有 %d 項未列出" % (len(errs) - 30), file=sys.stderr)
    return 1 if errs else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
