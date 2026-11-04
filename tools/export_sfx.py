#!/usr/bin/env python3
"""匯出 S.EXE 內層的 PC 喇叭音效規則：JSON 與示意用 WAV。只用標準函式庫。

用法：export_sfx.py <S.unpacked.bin> <原版目錄> <輸出目錄>
位址：IDA 線性位址（基底 10000h），資料段偏移 X = 1CBC0h + X；byte_2684F = ds:9C8Fh，依此類推。

內容分兩條路徑，工具各自從機器碼取證：
  1. 參數式引擎：sub_18672 依 byte_2684F 的位元選 sub_186BA / sub_18717 / sub_186F8 / sub_186D9 寫 5 個字組，
     sub_18736 依這些字組逐 tick 開關喇叭閘門。工具解出參數、模擬 sub_18736，並用位元組掃描檢查可達性。
  2. 繪圖附帶音：sub_192EA 在特效畫面逐列 OR 寫入時，用 PIT 通道 2 與喇叭閘門發聲。工具依 MY.GO 的像素資料
     算出每列的 (PIT 除數, 閘門) 序列。

WAV 是示意：實機時序取決於 CPU 速度，靜態資料無法決定，所以 WAV 用標明的假設時間軸合成方波，
不是實錄，也不能當成原版聲音。規則見 docs/re/13-audio-and-misc.md。
"""
import json
import os
import struct
import sys
import wave

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rs2_common import go_sections, parse_bank, unrle  # noqa: E402

BASE = 0x10000
DS = 0x1CBC0
PIT_HZ = 1193182
FIELD = {0x9C90: "duration", 0x9C92: "period", 0x9C94: "threshold", 0x9C96: "pitch_divisor", 0x9C98: "counter"}
FIELD_IDA = {0x9C90: "word_26850", 0x9C92: "word_26852", 0x9C94: "word_26854", 0x9C96: "word_26856", 0x9C98: "word_26858"}
EFFECT_FRAMES = {   # docs/re/11-entities-combat.md 8.1：特效動畫表使用的 MY.GO 圖索引
    "B 大爆炸": [27, 28, 29],
    "G 玩家死亡爆炸": [24, 23, 22, 21],
    "S 命中火花": [2, 56, 57],
}


def u16(img, lin):
    return struct.unpack_from("<H", img, lin - BASE)[0]


def decode_setter(img, addr):
    """sub_186BA 等：一串 mov word [ds:imm16], imm16（C7 06 lo hi lo hi），以 retn 結尾。"""
    out = {}
    p = addr
    while img[p - BASE] != 0xC3:
        if img[p - BASE] != 0xC7 or img[p - BASE + 1] != 0x06:
            raise ValueError("%05X 不是 mov word [m],imm" % p)
        a, v = u16(img, p + 2), u16(img, p + 4)
        out[FIELD[a]] = v
        p += 6
    return out


def decode_dispatch(img):
    """sub_18672：test byte [9C8F], mask / nop / jnz rel8，之後各分支 call 目標。"""
    p = 0x18672
    res = []
    for _ in range(4):
        if img[p - BASE:p - BASE + 4] != bytes.fromhex("f6068f9c"):
            raise ValueError("%05X 不是 test byte [9C8F]" % p)
        mask = img[p - BASE + 4]
        assert img[p - BASE + 5] == 0x90 and img[p - BASE + 6] == 0x75
        tgt = p + 8 + struct.unpack_from("<b", img, p - BASE + 7)[0]
        assert img[tgt - BASE] == 0xE8
        call = (tgt + 3 + struct.unpack_from("<h", img, tgt + 1 - BASE)[0]) & 0xFFFF
        res.append({"mask": mask, "handler": "%05X" % (BASE + call)})
        p += 8
    return res


def scan_operand(img, lo16):
    """回傳程式碼區內 16 位元運算元 == lo16 的位置與前一個位元組（粗掃，供人工核對）。"""
    pat = struct.pack("<H", lo16)
    out = []
    for i in range(0, DS - BASE - 1):
        if img[i:i + 2] == pat:
            out.append((BASE + i, img[i - 2], img[i - 1]))
    return out


def scan_calls(img, target):
    """掃 E8 rel16 剛好指到 target 的位置。"""
    out = []
    for i in range(0, DS - BASE - 3):
        if img[i] == 0xE8:
            rel = struct.unpack_from("<h", img, i + 1)[0]
            if BASE + ((i + 3 + rel) & 0xFFFF) == target:
                out.append(BASE + i)
    return out


def simulate_engine(params, ticks):
    """sub_18736 逐 tick 的狀態機。回傳每 tick 的 (閘門事件, 狀態)。閘門事件 on/off/None。"""
    s = dict(params)
    ev = []
    for t in range(ticks):
        gate = None
        if s["duration"] != 0:
            s["duration"] -= 1
            if s["duration"] == 0:       # 18749 jnz：減到 0 時換成預設參數
                s["period"], s["threshold"], s["pitch_divisor"] = 0x32, 5, 0x1F40
        ax, bx = s["period"], s["threshold"]
        s["counter"] = (s["counter"] - 1) & 0xFFFF
        c = s["counter"]
        sc = c - 0x10000 if c >= 0x8000 else c           # 18768 之後用有號比較
        sb = bx - 0x10000 if bx >= 0x8000 else bx
        if sc == sb:
            if s["pitch_divisor"] <= 0x3E8:
                s["pitch_divisor"] += 0x64
            if s["pitch_divisor"] >= 0x3E8:
                s["pitch_divisor"] += 0x32
            gate = "on"                  # 1879A：sub_187B3，port 61h |= 3
        elif sc < sb and sc == 0:
            s["counter"] = ax
            gate = "off"                 # 1877A：sub_187BB，port 61h &= FCh
        ev.append((t, gate, dict(s)))
    return ev


def my_planes(orig):
    b = open(os.path.join(orig, "MY.GO"), "rb").read()
    starts = go_sections(b)
    planes = []
    for k in range(4):
        raw, _, _ = unrle(b, starts[k])
        parse_bank(raw)                  # 驗證圖庫結構
        planes.append(raw)
    return planes


def frame_events(raw_plane_banks, idx, hi):
    """sub_192EA 對一張圖、四個平面（bl=1,2,4,8 依序 word_1CF52..58）逐列產生的喇叭設定。

    每列：列尾 OR 寫入完成後取下一列第一個位元組 al；al & 2Dh 非零時以 ax = (hi<<8)|al 設 PIT 並開閘門，
    否則關閘門；列尾再以 ax = (2BCh << cl) 設 PIT，cl 為剩餘列數（5 位元遮罩，286 以上行為）。
    hi 是列尾位址 (列起點 + 27h) 的高位元組，隨顯示頁與 y 變動，這裡由呼叫者指定。
    """
    seq = []
    for k, raw in enumerate(raw_plane_banks):
        items = parse_bank(raw)
        it = items[idx]
        h, w, off = it["height"], it["stride"], it["offset"] + 4
        for r in range(h):
            nxt = raw[off + (r + 1) * w] if off + (r + 1) * w < len(raw) else 0
            cl = h - r
            ev = {"plane": k, "row": r, "remaining": cl}
            if nxt & 0x2D:
                ev.update(gate="on", divisor=(hi << 8) | nxt)
            else:
                ev.update(gate="off")
            ev["tail_divisor"] = (0x2BC << (cl & 0x1F)) & 0xFFFF
            seq.append(ev)
    return seq


def square(freq, dur, rate, amp=9000):
    n = int(dur * rate)
    out = bytearray()
    if freq <= 0:
        return bytes(2 * n)
    for i in range(n):
        v = amp if int(i * 2 * freq / rate) % 2 == 0 else -amp
        out += struct.pack("<h", v)
    return bytes(out)


def write_wav(path, data, rate=44100):
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(data)


def main(argv):
    src, orig, dst = argv[1], argv[2], argv[3]
    img = open(src, "rb").read()
    if len(img) != 95908:
        raise SystemExit("S.unpacked.bin 大小不符")
    os.makedirs(dst, exist_ok=True)
    disp = decode_dispatch(img)
    patterns = {}
    for d in disp:
        patterns["bit %d" % (d["mask"].bit_length() - 1)] = {"mask": d["mask"], "setter": d["handler"],
                                                              "params": decode_setter(img, int(d["handler"], 16))}
    # 可達性證據
    req_hits = scan_operand(img, 0x9C8F)
    writers = [("%05X" % a) for a, o1, o2 in req_hits if (o1, o2) == (0xC6, 0x06)]
    testers = [("%05X" % a) for a, o1, o2 in req_hits if (o1, o2) != (0xC6, 0x06)]
    mute_hits = scan_operand(img, 0x9C8E)
    tick_calls = ["%05X" % a for a in scan_calls(img, 0x18736)]
    dead_loop_calls = ["%05X" % a for a in scan_calls(img, 0x186AF)]
    pre = img[0x186AE - BASE]
    # 引擎模擬：預設以第 3 組（byte_2684F 的唯一寫入值 4 = bit 2 -> sub_186F8）展開 60 tick
    sim = simulate_engine(patterns["bit 2"]["params"], 60)
    gate_events = [{"tick": t, "gate": g, "pitch_divisor": s["pitch_divisor"], "counter": s["counter"], "duration": s["duration"]}
                   for t, g, s in sim if g]
    sound_gate = {
        "sub_187A6": "out 43h,0B6h；out 42h,al；out 42h,ah，把 PIT 通道 2 設成方波、除數 = AX",
        "sub_187B3": "port 61h |= 3，開閘門並接上喇叭",
        "sub_187BB": "port 61h &= FCh，關閘門",
        "mute": "byte_2684E（ds:9C8E）bit 0，sub_1A891 以 xor 切換，條件為按鍵 S（掃描碼 1Fh）剛按下；"
                "sub_187C3 讀 bit 0：置位時把 sub_187B3、sub_187BB 的第一個位元組改成 C3（retn），並停止音樂；清除時改回 90",
    }
    info = {
        "input": {"file": os.path.basename(src), "size": len(img)},
        "request_byte": {"ida": "byte_2684F", "ds": "9C8F", "writers": writers, "tests": testers,
                         "note": "唯一的寫入是 sub_10F41 的 mov byte [9C8F],4（普通段每幀）；任何位置都沒有清除。"
                                 "優先權：sub_18672 依 bit0、bit1、bit2、bit3 的順序比對，先命中者勝"},
        "dispatch": disp,
        "patterns": patterns,
        "engine_sub_18736": {
            "rule": "每 tick：duration 非 0 則減 1，減到 0 時 period=32h、threshold=5、pitch_divisor=1F40h；"
                    "counter 減 1；counter == threshold 時 pitch_divisor 依 <=3E8h 加 64h、>=3E8h 加 32h，並開閘門；"
                    "counter 在 threshold 之下且為 0 時 counter=period 並關閘門",
            "pitch_divisor_use": "全檔沒有任何地方把 word_26856 傳給 sub_187A6，所以音高字組只被累加，沒有輸出到 PIT",
            "call_sites_of_sub_18736": tick_calls,
            "call_sites_of_186AF": dead_loop_calls,
            "byte_before_186AF": "%02X" % pre,
            "reachability": "唯一的 call 在 186B0，位於 sub_18672 的 retn（186AE）之後，沒有任何分支或呼叫指到 186AF；"
                            "沒有指向 18736 或 186AF 的絕對位址常數。靜態判定這條路徑不會執行",
            "grade": "強推論（靜態無入口；尚未動態量 port 61h 與 42h 的寫入）",
            "simulation_pattern_bit2": gate_events[:20],
        },
        "gate_and_pit": sound_gate,
        "mute_operand_hits": ["%05X" % a for a, o1, o2 in mute_hits],
        "blit_beeper": {
            "where": "sub_192EA（由 sub_18DCC 對 MY.GO 的 4 個平面各呼叫一次），只在 byte_1CF74 == 'E' 的 4 平面模式，來源是特效腳本 sub_17CB6",
            "rule": "開始時 sub_187A6(0Ah)；每列 OR 寫入後取下一列第一位元組 al，al & 2Dh 非零時 ax = (列尾位址高位元組 << 8 | al) 設 PIT 並開閘門，"
                    "否則關閘門；列尾再 sub_187A6(2BCh << cl)，cl 為剩餘列數；結束時關閘門",
            "tail_divisor_ladder": {str(cl): (0x2BC << cl) & 0xFFFF for cl in range(1, 16)},
            "tail_freq_hz": {str(cl): round(PIT_HZ / (((0x2BC << cl) & 0xFFFF) or 65536), 1) for cl in range(1, 16)},
            "grade": "靜態已證實（程式碼逐指令讀過）；時間軸為假設",
        },
    }
    # 特效圖的列序列（hi 取 0x80：第二顯示頁 8000h 起的位址）
    banks = my_planes(orig)
    frames = {}
    for name, idxs in EFFECT_FRAMES.items():
        frames[name] = {str(i): frame_events(banks, i, 0x80) for i in idxs}
    info["blit_beeper"]["effect_frames_hi80"] = frames
    with open(os.path.join(dst, "sfx-engine.json"), "w", encoding="utf-8") as fh:
        json.dump(info, fh, ensure_ascii=False, indent=1)

    # WAV 1：尾端階梯，每階 120 ms 示意
    data = b""
    for cl in range(1, 16):
        d = (0x2BC << cl) & 0xFFFF
        data += square(PIT_HZ / (d or 65536), 0.12, 44100)
    write_wav(os.path.join(dst, "illustration-tail-ladder.wav"), data)
    # WAV 2：特效 B 的實際資料列，慢速示意，每列 2 ms（實機每列是微秒等級）
    for name in ("B 大爆炸",):
        data = b""
        for idx in EFFECT_FRAMES[name]:
            for ev in frames[name][str(idx)]:
                if ev["gate"] == "on":
                    data += square(PIT_HZ / (ev["divisor"] or 65536), 0.002, 44100)
                else:
                    data += bytes(2 * int(0.002 * 44100))
        write_wav(os.path.join(dst, "illustration-effect-B-slow.wav"), data)
    print("patterns", {k: v["params"] for k, v in patterns.items()})
    print("request writers", writers, "tick call sites", tick_calls, "dead-loop call sites", dead_loop_calls)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
