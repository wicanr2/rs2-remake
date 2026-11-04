#!/usr/bin/env python3
"""解析 A.MIX / B.MIX：音色表 + Ad Lib MUS 風格事件串流，並輸出 JSON 與預覽用 .mid。

用法：parse_mix.py <原版目錄> <輸出目錄>
格式：u16 音色數 n；u16 音色表位元組數 (= n x 56)；n x 28 個 u16 音色參數；8 位元組曲目表頭；
接著是事件串流：delta 位元組 (F8 = 溢位 240 tick)、MIDI 風格狀態位元組 (可沿用 running status)，FC 結束。
曲目表頭 8 位元組（定義見 docs/re/13-audio-and-misc.md，對照 ASMDRV 的 sub_1008A）：
  u8 每拍 tick 數 (tickBeat, 240)、u8 每小節拍數 (4)、u16 事件串流長度、u8 節奏模式 (1)、
  u8 滑音範圍半音數 (1)、u16 基本速度 BPM (A=100, B=120)。
事件資料長度依驅動：80h/90h/B0h/E0h 兩個資料位元組，A0h/C0h/D0h 一個。
tick 換算：tick 頻率 = tickBeat x BPM / 60，所以預覽 .mid 的 division 用 tickBeat，速度用 BPM。
預覽 .mid 只給人聽音高與節奏，音色不對，不是忠實音源。
"""
import json
import os
import struct
import sys


def parse(buf):
    n, tab = struct.unpack_from("<HH", buf, 0)
    if tab != n * 56:
        raise ValueError(f"音色表長度不符：{tab} != {n}*56")
    inst = [list(struct.unpack_from("<28H", buf, 4 + i * 56)) for i in range(n)]
    p = 4 + tab
    # 8 位元組曲目表頭：u16 未知 (兩首都是 04F0h)、u16 事件串流長度、u8 x2 未知 (兩首都是 1)、u8 速度、u8 未知
    unk0, slen, unk1, unk2, tempo, unk3 = struct.unpack_from("<HHBBBB", buf, p)
    # 欄位意義：unk0 = tickBeat | beatMeasure << 8，unk1 = 節奏模式，unk2 = 滑音範圍，tempo | unk3 << 8 = 基本速度
    p += 8
    if p + slen != len(buf):
        raise ValueError(f"串流長度不符：{p}+{slen} != {len(buf)}")
    tick = 0
    status = None
    ev = []
    end = None
    while p < len(buf):
        d = buf[p]
        p += 1
        if d == 0xF8:
            tick += 240
            continue
        if d == 0xFC:
            end = p
            break
        tick += d
        b = buf[p]
        if b == 0xFC:
            end = p + 1
            break
        if b >= 0x80:
            status = b
            p += 1
        hi = status & 0xF0
        ch = status & 0x0F
        if hi in (0x80, 0x90, 0xB0, 0xE0):
            a, c = buf[p], buf[p + 1]
            p += 2
            ev.append((tick, hi, ch, a, c))
        elif hi in (0xA0, 0xC0, 0xD0):
            a = buf[p]
            p += 1
            ev.append((tick, hi, ch, a, 0))
        elif status == 0xF0:
            q = p
            while buf[p] != 0xF7:
                p += 1
            ev.append((tick, 0xF0, 0, q, p))
            p += 1
        else:
            raise ValueError(f"未知狀態 {status:#x} @ {p:#x}")
    if end is None:
        raise ValueError("沒有 FC 結束碼")
    return {"instrument_count": n, "instruments": inst, "stream_offset": 4 + tab + 8, "stream_end": end,
            "track_header": {"unk0": unk0, "stream_length": slen, "unk1": unk1, "unk2": unk2, "tempo": tempo, "unk3": unk3,
                             "tick_beat": unk0 & 0xFF, "beat_measure": unk0 >> 8, "sound_mode": unk1,
                             "pitch_bend_range": unk2, "basic_tempo": tempo | (unk3 << 8)},
            "trailing_bytes": len(buf) - end, "ticks": tick, "events": ev}


def vlq(v):
    out = [v & 0x7F]
    v >>= 7
    while v:
        out.insert(0, (v & 0x7F) | 0x80)
        v >>= 7
    return bytes(out)


def to_midi(r):
    tr = bytearray()
    last = 0
    h = r["track_header"]
    us = 60000000 // h["basic_tempo"]
    tr += b"\0\xff\x51\x03" + us.to_bytes(3, "big")
    for tick, hi, ch, a, c in r["events"]:
        if hi in (0xF0, 0xA0, 0xB0, 0xD0):   # 驅動不處理 A0h/B0h/D0h；F0h 7F 00 是速度倍率事件，A/B 都是 1.0
            continue
        tr += vlq(tick - last)
        last = tick
        if hi in (0xC0, 0xD0):
            tr += bytes([hi | ch, a])
        else:
            tr += bytes([hi | ch, a, c])
    tr += b"\0\xff\x2f\0"
    return b"MThd" + struct.pack(">IHHH", 6, 0, 1, h["tick_beat"]) + b"MTrk" + struct.pack(">I", len(tr)) + bytes(tr)


src, dst = sys.argv[1], sys.argv[2]
os.makedirs(dst, exist_ok=True)
for n in ("A.MIX", "B.MIX"):
    buf = open(os.path.join(src, n), "rb").read()
    r = parse(buf)
    kinds = {}
    chans = set()
    notes = 0
    for t, hi, ch, a, c in r["events"]:
        kinds[hex(hi)] = kinds.get(hex(hi), 0) + 1
        chans.add(ch)
        if hi == 0x90 and c:
            notes += 1
    with open(os.path.join(dst, n + ".mid"), "wb") as fh:
        fh.write(to_midi(r))
    with open(os.path.join(dst, n + ".json"), "w") as fh:
        json.dump(r, fh)
    print(n, "instruments", r["instrument_count"], "stream", hex(r["stream_offset"]), "end", hex(r["stream_end"]),
          "trailing", r["trailing_bytes"], "ticks", r["ticks"], "events", len(r["events"]), "notes", notes,
          "channels", sorted(chans), "header", r["track_header"], kinds)
