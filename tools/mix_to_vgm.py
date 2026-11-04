#!/usr/bin/env python3
"""A.MIX / B.MIX 轉 VGM（YM3812）。只用標準函式庫，不執行原版。

做法：把 ASMDRV.EXE（解包後）的 AdLib 驅動邏輯逐函式搬成 Python，餵進 .MIX 的事件串流，
把驅動會寫進 OPL2 的暫存器寫入序列與時間記下來，再包成 VGM。規則出處見 docs/re/13-audio-and-misc.md。
位址一律是 ASMDRV 解包映像的線性位址（IDA 基底 10000h）或資料段偏移（dseg:X，線性 = 10000h + X）。

用法：
  mix_to_vgm.py <原版目錄> <輸出目錄>            轉 A.MIX、B.MIX，輸出 .vgm、.oplwrites.tsv、.instruments.txt
  mix_to_vgm.py <原版目錄> <輸出目錄> --once      只轉一輪，不放循環標記
  mix_to_vgm.py <原版目錄> <輸出目錄> --verify-driver <ASMDRV.unpacked.exe>
                                                  另外比對本檔寫死的表和驅動映像是否一致

未對拍：沒有原版執行時的實錄 OPL 寫入可比，輸出是依規則合成的結果。
"""
import json
import os
import struct
import sys

PIT_CLOCK = 1193181.8181818  # 14.318181 MHz / 12
VGM_RATE = 44100
YM3812_CLOCK = 3579545

# ---------------------------------------------------------------- 驅動內的固定表（ASMDRV 映像 dseg 偏移）
OP_REG_OFF = [0, 1, 2, 3, 4, 5, 8, 9, 10, 11, 12, 13, 16, 17, 18, 19, 20, 21]      # dseg:1345h 運算子 -> OPL 槽位偏移
OP_IS_CARRIER = [0, 0, 0, 1, 1, 1, 0, 0, 0, 1, 1, 1, 0, 0, 0, 1, 1, 1]            # dseg:1357h
OP_CHANNEL = [0, 1, 2, 0, 1, 2, 3, 4, 5, 3, 4, 5, 6, 7, 8, 6, 7, 8]               # dseg:1369h 運算子所屬的 OPL 頻道
VOICE_OPS = [0, 3, 1, 4, 2, 5, 6, 9, 7, 10, 8, 11, 12, 15, 13, 16, 14, 17,
             12, 15, 16, 0, 14, 0, 17, 0, 13, 0]                                     # dseg:1329h，28 位元組
PERC_OP = {6: (0x0C, 0x0F), 7: (0x10,), 8: (0x0E,), 9: (0x11,), 10: (0x0D,)}      # dseg:133Bh 起的節奏模式運算子
RHYTHM_BIT = [0x10, 0x08, 0x04, 0x02, 0x01]                                        # dseg:12B4h BD SD TOM TC HH
# 預設運算子參數（14 位元組：13 參數 + 波形），dseg:12B9h 起
DEF_MOD = [1, 1, 3, 15, 5, 0, 1, 3, 15, 0, 0, 0, 1, 0]                             # 12B9h
DEF_CAR = [0, 1, 1, 15, 7, 0, 2, 4, 0, 0, 0, 1, 0, 0]                              # 12C7h
DEF_PERC = {                                                                       # 12D5h..131Bh
    0x0C: [0, 0, 0, 10, 4, 0, 8, 12, 11, 0, 0, 0, 1, 0],
    0x0F: [0, 0, 0, 13, 4, 0, 6, 15, 0, 0, 0, 0, 1, 0],
    0x10: [0, 12, 0, 15, 11, 0, 8, 5, 0, 0, 0, 0, 0, 0],
    0x0E: [0, 4, 0, 15, 11, 0, 7, 5, 0, 0, 0, 0, 0, 0],
    0x11: [0, 1, 0, 15, 11, 0, 5, 5, 0, 0, 0, 0, 0, 0],
    0x0D: [0, 1, 0, 15, 11, 0, 7, 5, 0, 0, 0, 0, 0, 0],
}
PARAM_NAMES = ["KSL", "MULT", "FEEDBACK", "ATTACK", "SUSTAIN", "EG_TYPE", "DECAY",
               "RELEASE", "TL", "AM", "VIB", "KSR", "CONNECT"]


def build_freq_rows():
    """sub_1091D / sub_108A9 / sub_1080A：25 列（細調 1/25 半音）x 12 個 fnum。全部整數運算。"""
    rows = []
    for r in range(25):
        a = 4 * r
        t = 100 * 100 + 6 * a
        prod = (t * 0xCB78) & 0xFFFFFFFF            # sub_111AE 32 位元乘法
        q = prod // (100 * 100 * 25)                  # sub_111D2，除數 10000*25
        f = (((q << 14) & 0xFFFFFFFF) * 9 & 0xFFFFFFFF) // 0x1B503
        row = [(((f & 0xFFFF) + 4) & 0xFFFF) >> 3]
        for _ in range(11):
            f = (f * 0x6A & 0xFFFFFFFF) // 100
            row.append((((f & 0xFFFF) + 4) & 0xFFFF) >> 3)
        rows.append(row)
    return rows


FREQ_ROWS = build_freq_rows()


# ---------------------------------------------------------------- 驅動合成部分
class Synth:
    """ASMDRV 的 OPL2 暫存器層。方法名後面的位址是對照的原函式。"""

    def __init__(self, emit):
        self.emit = emit                 # emit(reg, val)
        self.prm = [[0] * 14 for _ in range(18)]   # dseg:190Fh，每運算子 14 位元組
        self.opvol = [0x7F] * 18         # dseg:1A2Fh，sub_107F8
        self.rhythm = 0                  # byte_11A0B
        self.rbits = 0                   # byte_116B4
        self.am_depth = 0                # byte_116A4
        self.vib_depth = 0               # byte_116A8
        self.nts = 0                     # byte_116A5
        self.wsel = 0                    # word_11A17
        self.bend_range = 1              # word_116B5
        self.bend_mul = 25               # word_11A41
        self.vnote = [0] * 12            # dseg:16A9h
        self.vkey = [0] * 12             # dseg:1A0Ch
        self.vrow = [0] * 12             # dseg:1A19h，列索引（原為指標）
        self.voff = [0] * 12             # dseg:1A43h
        self.note_oct = [i // 12 for i in range(96)]   # dseg:15E4h
        self.note_semi = [i % 12 for i in range(96)]   # dseg:1644h

    def w(self, reg, val):
        self.emit(reg & 0xFF, val & 0xFF)

    # sub_10FC0 由 emit 取代
    def reg_bd(self):                    # sub_10DDA
        v = (0x80 if self.am_depth else 0) | (0x40 if self.vib_depth else 0) | (0x20 if self.rhythm else 0) | self.rbits
        self.w(0xBD, v)

    def reg_08(self):                    # sub_10C4F
        self.w(0x08, 0x40 if self.nts else 0)

    def reg_tl(self, op):                # sub_10BE5
        p = self.prm[op]
        lv = 0x3F - (p[8] & 0x3F)
        s = self.opvol[op] * lv
        s = s + (s + 0x7F)
        x = 0x3F - (s // 0xFE)
        x |= p[0] << 6
        self.w(0x40 + OP_REG_OFF[op], x)

    def reg_fb(self, op):                # sub_10C69
        if OP_IS_CARRIER[op]:
            return
        p = self.prm[op]
        v = (p[2] << 1) | (0 if p[12] else 1)
        self.w(0xC0 + OP_CHANNEL[op], v)

    def reg_ad(self, op):                # sub_10CBB
        p = self.prm[op]
        self.w(0x60 + OP_REG_OFF[op], (p[3] << 4) | (p[6] & 0x0F))

    def reg_sr(self, op):                # sub_10D01
        p = self.prm[op]
        self.w(0x80 + OP_REG_OFF[op], (p[4] << 4) | (p[7] & 0x0F))

    def reg_mult(self, op):              # sub_10D47
        p = self.prm[op]
        v = (0x80 if p[9] else 0) + (0x40 if p[10] else 0) + (0x20 if p[5] else 0) + (0x10 if p[11] else 0) + (p[1] & 0x0F)
        self.w(0x20 + OP_REG_OFF[op], v)

    def reg_wave(self, op):              # sub_10E1D
        v = (self.prm[op][13] & 3) if self.wsel else 0
        self.w(0xE0 + OP_REG_OFF[op], v)

    def op_refresh(self, op):            # sub_10BB1
        self.reg_bd()
        self.reg_08()
        self.reg_tl(op)
        self.reg_fb(op)
        self.reg_ad(op)
        self.reg_sr(op)
        self.reg_mult(op)
        self.reg_wave(op)

    def set_op(self, op, params13, wave):    # sub_10ABF
        self.prm[op][:13] = [x & 0xFF for x in params13[:13]]
        self.prm[op][13] = wave & 3
        self.op_refresh(op)

    def set_op_default(self, op, rec):   # sub_10AFD
        self.set_op(op, rec[:13], 0)

    def init_ops(self):                  # sub_10759
        for op in range(18):
            self.set_op_default(op, DEF_CAR if OP_IS_CARRIER[op] else DEF_MOD)
        if self.rhythm:
            for op in (0x0C, 0x0F, 0x10, 0x0E, 0x11, 0x0D):
                self.set_op_default(op, DEF_PERC[op])

    def voice_pitch(self, v, note, key):  # sub_10E58
        self.vkey[v] = key
        self.vnote[v] = note
        n = note + self.voff[v]
        n = min(n, 0x5F)
        n = max(n, 0)
        fnum = FREQ_ROWS[self.vrow[v]][self.note_semi[n]]
        self.w(0xA0 + v, fnum & 0xFF)
        self.w(0xB0 + v, (0x20 if key else 0) + self.note_oct[n] * 4 + ((fnum >> 8) & 3))

    def silence_voice(self, v):          # sub_10EF6
        self.w(0xA0 + v, 0)
        self.w(0xB0 + v, 0)

    def set_rhythm_mode(self, mode):     # sub_10469
        if mode:
            for v in (6, 7, 8):
                self.silence_voice(v)
            self.voice_pitch(8, 0x18, 0)
            self.voice_pitch(7, 0x1F, 0)
        self.rhythm = mode & 0xFF
        self.rbits = 0
        self.init_ops()
        self.reg_bd()

    def set_bend_range(self, r):         # sub_10506
        r = min(r, 12)
        r = max(r, 1)
        self.bend_range = r
        self.bend_mul = r * 25

    def set_depths(self, am, vib, nts):  # sub_1052F
        self.am_depth, self.vib_depth, self.nts = am, vib, nts
        self.reg_bd()
        self.reg_08()

    def set_wsel(self, on):              # sub_104C4
        self.wsel = 0x20 if on else 0
        for op in range(18):
            self.w(0xE0 + OP_REG_OFF[op], 0)
        self.w(0x01, self.wsel)

    def driver_init(self):               # sub_10427（AX=0 的重置）
        self.opvol = [0x7F] * 18         # sub_107F8
        self.vrow = [0] * 12             # sub_1091D
        self.voff = [0] * 12
        self.set_rhythm_mode(0)
        self.set_depths(0, 0, 0)
        for v in range(9):
            self.silence_voice(v)
        self.set_bend_range(1)
        self.set_wsel(1)

    def perc_or_melodic(self, v):
        return self.rhythm != 0 and v >= 6

    def set_instrument(self, v, inst):   # sub_10550。inst 是 28 個 u16
        wave0, wave1 = inst[26], inst[27]
        a, b = inst[0:13], inst[13:26]
        if not self.perc_or_melodic(v):
            self.set_op(VOICE_OPS[2 * v], a, wave0)
            self.set_op(VOICE_OPS[2 * v + 1], b, wave1)
        elif v == 6:
            self.set_op(0x0C, a, wave0)
            self.set_op(0x0F, b, wave1)
        else:
            self.set_op(PERC_OP[v][0], a, wave0)

    def carrier_op(self, v):
        if not self.perc_or_melodic(v):
            return VOICE_OPS[2 * v + 1]
        return PERC_OP[v][-1]

    def set_volume(self, v, vol):        # 10603h
        op = self.carrier_op(v)
        self.opvol[op] = min(vol, 0x7F)
        self.reg_tl(op)

    def note_on(self, v, midi):          # 106A1h
        n = max(midi - 12, 0)
        if not self.perc_or_melodic(v):
            self.voice_pitch(v, n, 1)
            return
        if v == 6:
            self.voice_pitch(6, n, 0)
        elif v == 8:
            self.voice_pitch(8, n, 0)
            self.voice_pitch(7, n + 7, 0)
        self.rbits |= RHYTHM_BIT[v - 6]
        self.reg_bd()

    def note_off(self, v):               # sub_1071E
        if not self.perc_or_melodic(v):
            self.voice_pitch(v, self.vnote[v], 0)
        else:
            self.rbits &= ~RHYTHM_BIT[v - 6] & 0xFF
            self.reg_bd()

    def pitch_bend(self, v, value):      # 10660h + sub_109B4
        if self.rhythm and v > 6:
            return
        value = min(value, 0x3FFF)
        prod = (value - 0x2000) * self.bend_mul
        q = abs(prod) // 0x2000          # Borland LDIV 往零截斷
        di = q if prod >= 0 else -q
        if di < 0:
            m = -di
            v6 = 0x18 + m
            self.voff[v] = -(v6 // 25)
            rem = m % 25
            row = (25 - rem) if rem else 0
        else:
            self.voff[v] = di // 25
            row = di % 25
        self.vrow[v] = row
        self.voice_pitch(v, self.vnote[v], self.vkey[v])


# ---------------------------------------------------------------- 曲目與播放器
class Mix:
    def __init__(self, buf):
        n, tab = struct.unpack_from("<HH", buf, 0)
        if tab != n * 56:
            raise ValueError("音色表長度不符")
        self.n = n
        self.inst = [list(struct.unpack_from("<28H", buf, 4 + i * 56)) for i in range(n)]
        p = 4 + tab
        self.tick_beat, self.beat_measure, self.stream_len, self.sound_mode, self.bend_range, self.tempo = \
            struct.unpack_from("<BBHBBH", buf, p)
        self.stream = buf[p + 8:]
        if len(self.stream) != self.stream_len:
            raise ValueError("串流長度不符")


def tick_rate(tick_beat, tempo):
    """sub_11145 -> sub_1102E：回傳 (PIT 除數, 實際 Hz)。"""
    si = (tick_beat * tempo) // 60
    if si == 0:
        div = 0
    else:
        si = max(si, 0x13)
        div = 0x1234DC // si
    return div, PIT_CLOCK / (div or 65536)


class Player:
    """sub_10106（開始）與 sub_10189（每個計時 tick 的事件處理）。"""

    def __init__(self, mix, synth, loop):
        self.m = mix
        self.s = synth
        self.st = mix.stream
        self.loop = loop
        self.ptr = 0
        self.status = 0
        self.chan = 0
        self.volcache = [0] * 11
        self.rate_hz = 0.0
        self.wraps = 0                   # FC 循環回頭的次數
        self.hook = None                 # hook(ch, instrument_index)：C0h 事件觀察用
        self.ended = False

    def start(self):
        s = self.s
        m = self.m
        self.volcache = [0] * 11         # 1008A: 清 dseg:1298h 共 11 位元組
        s.set_rhythm_mode(m.sound_mode)  # sub_10106 -> sub_10469
        s.set_bend_range(m.bend_range)   # sub_10506
        self.set_tempo(m.tempo)
        first = self.st[0]
        if first == 0xF8:
            raise ValueError("首個 delta 是 F8，驅動不會把它展開")
        self.ptr = 1
        return first if first else 1

    def set_tempo(self, tempo):
        div, hz = tick_rate(self.m.tick_beat, tempo)
        self.rate_hz = hz
        self.div = div

    def _delta(self):                    # 103CF / 103DD 的取 delta 與 F8 展開，回傳 (繼續處理?, 延遲)
        d = self.st[self.ptr]
        self.ptr += 1
        return self._delta_check(d)

    def _delta_check(self, d):           # 103DD
        if d == 0:
            return True, 0
        if d == 0xF8:
            nxt = self.st[self.ptr]
            if nxt != 0xF8:
                self.ptr += 1
                return False, 0xF0 + nxt
        return False, d

    def step(self):
        """處理到下一個非零 delta，回傳延遲 tick 數；曲終且不循環回傳 None。"""
        st = self.st
        s = self.s
        m = self.m
        while True:
            p = self.ptr
            b = st[p]
            if b == 0xF8:                # 101A3
                self.ptr += 1
                cont, d = self._delta_check(0xF8)
                if cont:
                    continue
                return d
            if b == 0xFC:                # 101AF
                if self.loop:
                    self.ptr = 1         # word_115CD + 1：略過第一個 delta
                    self.wraps += 1
                    continue
                self.ended = True
                return None
            if b == 0xF0:                # 10229
                if st[p + 1] == 0x7F and st[p + 2] == 0:
                    ip, fr = st[p + 3], st[p + 4]
                    base = m.tempo
                    new = (((ip * base) & 0xFFFF) + (((fr * base) >> 7) & 0xFFFF)) & 0xFFFF
                    self.set_tempo(new)
                    d = st[p + 6]
                    self.ptr = p + 7
                else:
                    q = p + 1
                    while st[q] != 0xF7:
                        q += 1
                    d = st[q + 1]
                    self.ptr = q + 2
                cont, dd = self._delta_check(d)
                if cont:
                    continue
                return dd
            if b >= 0x80:               # 102A1
                self.status = b & 0xF0
                self.chan = b & 0x0F
                self.ptr += 1
            p = self.ptr
            hi, ch = self.status, self.chan
            if hi == 0x90:
                if st[p + 1] == 0:
                    s.note_off(ch)
                    self.ptr = p + 2
                else:
                    vel = st[p + 1]
                    if ch < 11 and vel != self.volcache[ch]:
                        self.volcache[ch] = vel
                        s.set_volume(ch, vel)
                    s.note_on(ch, st[p])
                    self.ptr = p + 2
                cont, d = self._delta()
            elif hi == 0x80:
                self.ptr = p + 2
                s.note_off(ch)
                # 1031Fh 直接跳到 103DDh，不讀 delta，沿用上一個 delta 值（驅動缺陷；A/B.MIX 沒有 80h 事件）
                raise ValueError("遇到 80h 事件，驅動行為未模擬")
            elif hi == 0xC0:
                idx = st[p]
                if self.hook:
                    self.hook(ch, idx)
                s.set_instrument(ch, m.inst[idx])
                self.ptr = p + 1
                cont, d = self._delta()
            elif hi == 0xE0:
                s.pitch_bend(ch, st[p] + (st[p + 1] << 7))
                self.ptr = p + 2
                cont, d = self._delta()
            elif hi == 0xB0:
                self.ptr = p + 2
                cont, d = self._delta()
            elif hi == 0xD0:
                self.ptr = p + 1
                cont, d = self._delta()
            else:                        # 103B8：A0h 等未實作的事件，往後掃到下一個 >= 80h 的位元組
                q = p
                while st[q] < 0x80:
                    q += 1
                self.ptr = q
                if st[q] != 0xF8:
                    self.ptr = q - 1
                cont, d = self._delta()
            if cont:
                continue
            return d


def simulate(mix, passes, hook=None):
    """回傳 (寫入清單, 每次循環回頭的時間, 結束時間)。寫入清單項目：(秒, reg, val)。

    passes=1 不循環，在第一個 FC 結束。passes>1 時循環，數到第 passes 次回頭就停。
    時間：事件在第 T 個計時 tick 處理，T 為 delta 累計，首個 delta 為 0 時算 1；秒 = T / tick 頻率。
    """
    log = []
    clock = [0.0]
    synth = Synth(lambda reg, val: log.append((clock[0], reg, val)))
    synth.driver_init()                  # INT 60h AX=0
    player = Player(mix, synth, loop=passes > 1)
    player.hook = hook
    delay = player.start()               # INT 60h AX=2
    t = delay / player.rate_hz
    wraps = []
    while True:
        clock[0] = t
        n0 = player.wraps
        d = player.step()
        wraps += [t] * (player.wraps - n0)
        if d is None or len(wraps) >= passes:
            return log, wraps, t
        t += d / player.rate_hz


# ---------------------------------------------------------------- VGM
def utf16z(s):
    return s.encode("utf-16-le") + b"\0\0"


def make_vgm(writes, total_t, loop_t, title):
    """writes: [(秒, reg, val)]，loop_t: 循環起點的秒數（None 表示不循環）。"""
    def smp(t):
        return int(round(t * VGM_RATE))

    data = bytearray()
    pos = 0
    loop_off = None
    loop_pos = smp(loop_t) if loop_t is not None else None
    items = sorted(enumerate(writes), key=lambda x: (x[1][0], x[0]))
    for _, (t, reg, val) in items:
        sp = smp(t)
        if loop_pos is not None and loop_off is None and sp >= loop_pos:
            # 先補等待到循環點，再記下偏移
            if loop_pos > pos:
                wait = loop_pos - pos
                data += _wait(wait)
                pos = loop_pos
            loop_off = len(data)
        if sp > pos:
            data += _wait(sp - pos)
            pos = sp
        data += bytes([0x5A, reg, val])
    end = smp(total_t)
    if loop_pos is not None and loop_off is None:
        if loop_pos > pos:
            data += _wait(loop_pos - pos)
            pos = loop_pos
        loop_off = len(data)
    if end > pos:
        data += _wait(end - pos)
        pos = end
    data += b"\x66"
    fields = [title, "", "RS-2", "", "IBM PC / AdLib", "", "Soft-World (Daiyu)", "", "1990",
              "mix_to_vgm.py (rule-based synthesis from ASMDRV, not a recording)", "unmatched against original"]
    body = b"".join(utf16z(x) for x in fields)
    gd3 = b"Gd3 " + struct.pack("<II", 0x100, len(body)) + body
    hdr = bytearray(0x80)
    hdr[0:4] = b"Vgm "
    struct.pack_into("<I", hdr, 0x08, 0x151)
    total = pos
    struct.pack_into("<I", hdr, 0x18, total)
    if loop_off is not None:
        struct.pack_into("<I", hdr, 0x1C, 0x80 + loop_off - 0x1C)
        struct.pack_into("<I", hdr, 0x20, total - smp(loop_t))
    struct.pack_into("<I", hdr, 0x34, 0x80 - 0x34)
    struct.pack_into("<I", hdr, 0x50, YM3812_CLOCK)
    gd3_off = 0x80 + len(data)
    struct.pack_into("<I", hdr, 0x14, gd3_off - 0x14)
    struct.pack_into("<I", hdr, 0x04, 0x80 + len(data) + len(gd3) - 4)
    return bytes(hdr) + bytes(data) + gd3


def _wait(n):
    out = bytearray()
    while n > 0:
        k = min(n, 0xFFFF)
        if k == 735:
            out += b"\x62"
        elif k == 882:
            out += b"\x63"
        else:
            out += b"\x61" + struct.pack("<H", k)
        n -= k
    return bytes(out)


def parse_vgm(buf):
    """自檢用：解 VGM，回傳 (寫入清單 [(取樣, reg, val)], 總取樣, 循環偏移是否有效)。"""
    assert buf[:4] == b"Vgm "
    off = struct.unpack_from("<I", buf, 0x34)[0] + 0x34
    p = off
    t = 0
    out = []
    loop_abs = struct.unpack_from("<I", buf, 0x1C)[0]
    loop_abs = loop_abs + 0x1C if loop_abs else None
    loop_t = None
    while True:
        if loop_abs is not None and p == loop_abs:
            loop_t = t
        c = buf[p]
        if c == 0x5A:
            out.append((t, buf[p + 1], buf[p + 2]))
            p += 3
        elif c == 0x61:
            t += struct.unpack_from("<H", buf, p + 1)[0]
            p += 3
        elif c == 0x62:
            t += 735
            p += 1
        elif c == 0x63:
            t += 882
            p += 1
        elif c == 0x66:
            break
        else:
            raise ValueError("未知 VGM 指令 %02X @%X" % (c, p))
    return out, t, loop_t


def keyon_cents(log):
    """自檢：key-on 當下的 OPL 頻率（49716 Hz 取樣）相對十二平均律（A=440）最近半音的音分誤差，回傳 (最小, 最大)。"""
    import math
    low = {}
    out = []
    for _, reg, val in log:
        if 0xA0 <= reg <= 0xA8:
            low[reg - 0xA0] = val
        elif 0xB0 <= reg <= 0xB8 and val & 0x20:
            fnum = ((val & 3) << 8) | low.get(reg - 0xB0, 0)
            f = fnum * 49716 / 2 ** (20 - ((val >> 2) & 7))
            if f > 0:
                n = 12 * math.log2(f / 440) + 69
                out.append((n - round(n)) * 100)
    return (round(min(out), 1), round(max(out), 1)) if out else (None, None)


# ---------------------------------------------------------------- 音色欄位表
def instrument_registers(inst):
    """音色指派到旋律頻道（滿音量）時，兩個運算子會得到的 OPL 暫存器值。"""
    out = []
    for k in range(2):
        p = [x & 0xFF for x in inst[13 * k:13 * k + 13]] + [inst[26 + k] & 3]
        lv = 0x3F - (p[8] & 0x3F)
        s = 0x7F * lv
        tl = 0x3F - ((s + s + 0x7F) // 0xFE)
        out.append({
            "20h": (0x80 if p[9] else 0) + (0x40 if p[10] else 0) + (0x20 if p[5] else 0) + (0x10 if p[11] else 0) + (p[1] & 15),
            "40h": ((tl | (p[0] << 6)) & 0xFF),
            "60h": ((p[3] << 4) | (p[6] & 15)) & 0xFF,
            "80h": ((p[4] << 4) | (p[7] & 15)) & 0xFF,
            "C0h": (((p[2] << 1) | (0 if p[12] else 1)) & 0xFF) if k == 0 else None,
            "E0h": p[13],
        })
    return out


def instrument_table(name, mix, cmap):
    lines = ["%s：%d 個音色，每個 56 位元組 = 28 個 u16 = 運算子0(13 參數) + 運算子1(13 參數) + 波形0 + 波形1" % (name, mix.n)]
    lines.append("欄位 w[0..12] 對運算子0（調變器），w[13..25] 對運算子1（載波器），w[26] 波形0，w[27] 波形1。")
    lines.append("寫暫存器只取各 u16 的低位元組；運算子1 的 FEEDBACK、CONNECT 不會被用到。")
    for i, inst in enumerate(mix.inst):
        used = ", ".join("頻道%d" % c for c in cmap.get(i, [])) or "未被 C0h 指派"
        lines.append("")
        lines.append("音色 %d（%s）" % (i, used))
        head = "%-9s" % "欄位" + "".join("%9s" % n for n in PARAM_NAMES) + "%7s" % "WAVE"
        lines.append(head)
        for k in range(2):
            ws = inst[13 * k:13 * k + 13]
            lines.append("%-9s" % ("OP%d" % k) + "".join("%9d" % v for v in ws) + "%7d" % inst[26 + k])
        regs = instrument_registers(inst)
        for k in range(2):
            r = regs[k]
            lines.append("OP%d 暫存器  20h=%02X 40h=%02X 60h=%02X 80h=%02X C0h=%s E0h=%02X"
                         % (k, r["20h"], r["40h"], r["60h"], r["80h"],
                            ("%02X" % r["C0h"]) if r["C0h"] is not None else "--", r["E0h"]))
    return "\n".join(lines) + "\n"


def channel_instruments(mix):
    """用同一個播放器跑一輪，收集 C0h 事件，回傳 {音色索引: [頻道...]}。"""
    out = {}
    simulate(mix, 1, hook=lambda ch, idx: out.setdefault(idx, []).append(ch))
    return out


# ---------------------------------------------------------------- 驅動表比對
def verify_driver(path):
    b = open(path, "rb").read()
    img = b[struct.unpack_from("<H", b, 8)[0] * 16:]
    ok = True

    def chk(name, got, want):
        nonlocal ok
        if list(got) != list(want):
            ok = False
            print("不一致", name, list(got), list(want))
        else:
            print("一致  ", name)

    chk("1345h 運算子偏移", img[0x1345:0x1345 + 18], OP_REG_OFF)
    chk("1357h 載波旗標", img[0x1357:0x1357 + 18], OP_IS_CARRIER)
    chk("1369h 運算子頻道", img[0x1369:0x1369 + 18], OP_CHANNEL)
    chk("1329h 聲部運算子", img[0x1329:0x1329 + 28], VOICE_OPS)
    chk("12B4h 節奏位元", img[0x12B4:0x12B4 + 5], RHYTHM_BIT)
    chk("12B9h 預設調變器", img[0x12B9:0x12B9 + 14], DEF_MOD)
    chk("12C7h 預設載波器", img[0x12C7:0x12C7 + 14], DEF_CAR)
    for i, op in enumerate((0x0C, 0x0F, 0x10, 0x0E, 0x11, 0x0D)):
        a = 0x12D5 + 14 * i
        chk("%04Xh 節奏預設 op%02X" % (a, op), img[a:a + 14], DEF_PERC[op])
    chk("133Bh 節奏運算子", [img[0x133B], img[0x133C], img[0x133D], img[0x133F], img[0x1341], img[0x1343]],
        [0x0C, 0x0F, 0x10, 0x0E, 0x11, 0x0D])
    chk("12A6h INT 60h 分派表", struct.unpack_from("<7H", img, 0x12A6), [0x75, 0x84, 0x8A, 0xDC, 0xE8, 0xED, 0xF3])
    return ok


# ---------------------------------------------------------------- 主程式
def main(argv):
    if len(argv) >= 4 and argv[3] == "--verify-driver":
        return 0 if verify_driver(argv[4]) else 1
    src, dst = argv[1], argv[2]
    once = "--once" in argv
    os.makedirs(dst, exist_ok=True)
    summary = {}
    for name in ("A.MIX", "B.MIX"):
        mix = Mix(open(os.path.join(src, name), "rb").read())
        # 第一輪含首個 delta，第二輪是穩態循環；再跑第三輪，確認第二輪與第三輪的寫入相同
        steady = None
        if once:
            log, wraps, end_t = simulate(mix, 1)
            loop_t = None
        else:
            log, wraps, end_t = simulate(mix, 3)
            w0, w1, w2 = wraps
            a = [(round(t - w0, 9), r, v) for t, r, v in log if w0 <= t < w1]
            b = [(round(t - w1, 9), r, v) for t, r, v in log if w1 <= t < w2]
            steady = a == b
            log = [x for x in log if x[0] < w1]   # 輸出只含第一輪與第二輪
            end_t = w1
            loop_t = w0
        title = "RS-2 %s (rule-based OPL2 synthesis)" % name
        vgm = make_vgm(log, end_t, loop_t, title)
        with open(os.path.join(dst, name + ".vgm"), "wb") as fh:
            fh.write(vgm)
        with open(os.path.join(dst, name + ".oplwrites.tsv"), "w") as fh:
            fh.write("# seconds\treg\tval，依 ASMDRV 規則算出，未對拍\n")
            for t, r, v in log:
                fh.write("%.6f\t%02X\t%02X\n" % (t, r, v))
        cmap = channel_instruments(mix)
        with open(os.path.join(dst, name + ".instruments.txt"), "w") as fh:
            fh.write(instrument_table(name, mix, cmap))
        # 自檢：重新解 VGM，確認寫入數與總長
        w, tot, lt = parse_vgm(vgm)
        div, hz = tick_rate(mix.tick_beat, mix.tempo)
        summary[name] = {
            "tick_beat": mix.tick_beat, "beat_measure": mix.beat_measure, "stream_len": mix.stream_len,
            "sound_mode": mix.sound_mode, "bend_range": mix.bend_range, "basic_tempo": mix.tempo,
            "pit_divisor": div, "tick_hz": round(hz, 4),
            "opl_writes": len(w), "vgm_samples": tot,
            "vgm_seconds": round(tot / VGM_RATE, 3),
            "loop_start_sample": lt, "loop_steady_state": steady,
            "first_pass_seconds": round(wraps[0], 3) if wraps else None,
            "keyon_cents_min_max": keyon_cents(log),
        }
        print(name, json.dumps(summary[name], ensure_ascii=False))
    with open(os.path.join(dst, "vgm-summary.json"), "w") as fh:
        json.dump(summary, fh, ensure_ascii=False, indent=1)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
