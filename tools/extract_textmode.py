#!/usr/bin/env python3
"""匯出 S.EXE 內層開機時的文字模式畫面：主選單（V／E／C／H）與 PASSWORD 畫面。只用標準函式庫。

用法：extract_textmode.py <S.unpacked.bin> <輸出目錄>
做法：直接從解包映像的機器碼讀出畫面繪製序列，在 80x25 文字緩衝區重播，不依賴 IDA 輸出。
重播器只認得這幾種指令：nop、mov byte/word [ds:imm],imm、mov cx/si/al/ax,imm、call rel16，
遇到其他指令就停止（fail-closed）。呼叫目標只認 sub_10643（畫矩形）、sub_10660（設游標）、
sub_10633（寫單一字元）、sub_105D8（印字串）。位址是 IDA 線性位址（基底 10000h），資料段偏移 X = 1CBC0h + X。

資料段只讀到偏移 08BDh 之前。dseg:08BDh 的密碼查表不讀、不匯出，工具以斷言保證。
規則見 docs/re/13-audio-and-misc.md。

輸出（每個畫面）：
  <name>.txt   80x25 純文字（CP437 轉 Unicode）
  <name>.ans   ANSI 色彩版（blink 屬性用 SGR 5）
  <name>.attr  80x25 屬性位元組（十六進位）
  layout.json  畫面來源、指令範圍、重播軌跡與使用到的字串
"""
import json
import os
import struct
import sys

BASE = 0x10000
DS = 0x1CBC0
LIMIT = 0x8BD               # 資料段可讀上限（不含），密碼查表從此開始
V_CHAR, V_MARGIN, V_COL, V_ROW, V_ATTR = 0x1C, 0x1D, 0x1E, 0x1F, 0x20
V_COUNT = 0x9CC0            # word，sub_10643 的重複字元數（word_26880）
ADDR_RECT, ADDR_CURSOR, ADDR_PUTC, ADDR_PRINT = 0x10643, 0x10660, 0x10633, 0x105D8
ATTR_CODES = {0x40: 0x03, 0x23: 0xB9, 0x25: 0x07, 0x5E: 0x30}   # sub_105D8 內 @ # % ^ 對應的屬性


class Screen:
    def __init__(self):
        self.cells = [[(0x20, 0x07)] * 80 for _ in range(25)]

    def fill(self, attr):
        self.cells = [[(0x20, attr)] * 80 for _ in range(25)]

    def put(self, row, col, ch, attr):
        if 0 <= row < 25 and 0 <= col < 80:
            self.cells[row][col] = (ch, attr)


class VM:
    def __init__(self, img, screen, trace):
        self.img = img
        self.s = screen
        self.trace = trace
        self.mem = {V_CHAR: 0, V_MARGIN: 0, V_COL: 0, V_ROW: 0, V_ATTR: 0x07, V_COUNT: 0}
        self.cx = self.si = self.al = self.ax = 0
        self.used_strings = []

    def code(self, lin):
        return self.img[lin - BASE]

    def u16(self, lin):
        return struct.unpack_from("<H", self.img, lin - BASE)[0]

    def run(self, start, end):
        ip = start
        while ip < end:
            op = self.code(ip)
            if op == 0x90:
                ip += 1
            elif op == 0xC6 and self.code(ip + 1) == 0x06:
                a, v = self.u16(ip + 2), self.code(ip + 4)
                if a not in self.mem or a == V_COUNT:
                    raise ValueError("%05X 寫入未知位址 ds:%04X" % (ip, a))
                self.mem[a] = v
                ip += 5
            elif op == 0xC7 and self.code(ip + 1) == 0x06:
                a, v = self.u16(ip + 2), self.u16(ip + 4)
                if a != V_COUNT:
                    raise ValueError("%05X 寫入未知字組 ds:%04X" % (ip, a))
                self.mem[a] = v
                ip += 6
            elif op == 0xB9:
                self.cx = self.u16(ip + 1)
                ip += 3
            elif op == 0xBE:
                self.si = self.u16(ip + 1)
                ip += 3
            elif op == 0xB0:
                self.al = self.code(ip + 1)
                ip += 2
            elif op == 0xB8:
                self.ax = self.u16(ip + 1)
                ip += 3
            elif op == 0xE8:
                rel = struct.unpack_from("<h", self.img, ip + 1 - BASE)[0]
                self.call(BASE + (((ip - BASE) + 3 + rel) & 0xFFFF), ip)
                ip += 3
            else:
                raise ValueError("%05X 不認得的指令 %02X，停止" % (ip, op))

    def call(self, tgt, ip):
        m = self.mem
        if tgt == ADDR_CURSOR:
            self.trace.append("%05X cursor row=%d col=%d" % (ip, m[V_ROW], m[V_COL]))
        elif tgt == ADDR_PUTC:
            self.s.put(m[V_ROW], m[V_COL], self.al, m[V_ATTR])
            self.trace.append("%05X putc row=%d col=%d ch=%02X attr=%02X" % (ip, m[V_ROW], m[V_COL], self.al, m[V_ATTR]))
        elif tgt == ADDR_RECT:
            self.trace.append("%05X rect row=%d col=%d rows=%d width=%d ch=%02X attr=%02X"
                              % (ip, m[V_ROW], m[V_COL], self.cx, m[V_COUNT], m[V_CHAR], m[V_ATTR]))
            for _ in range(self.cx):
                for k in range(m[V_COUNT]):
                    self.s.put(m[V_ROW], m[V_COL] + k, m[V_CHAR], m[V_ATTR])
                m[V_ROW] += 1
            self.cx = 0
        elif tgt == ADDR_PRINT:
            self.print_str(ip)
        else:
            raise ValueError("%05X 呼叫未列入白名單的目標 %05X" % (ip, tgt))

    def print_str(self, ip):
        """sub_105D8：$ 結束；CR 換行回到左邊界；空白只前進；@ # % ^ 改屬性；其他字元寫入後前進。"""
        m = self.mem
        si = self.si
        start = si
        self.trace.append("%05X print ds:%04X row=%d col=%d margin=%d attr=%02X" % (ip, si, m[V_ROW], m[V_COL], m[V_MARGIN], m[V_ATTR]))
        while True:
            if si >= LIMIT:
                raise ValueError("字串讀取越過 ds:%04X" % LIMIT)
            ch = self.img[DS - BASE + si]
            si += 1
            if ch == 0x0D:
                m[V_ROW] += 1
                m[V_COL] = m[V_MARGIN]
            elif ch == 0x24:
                break
            elif ch == 0x20:
                m[V_COL] += 1
            elif ch in ATTR_CODES:
                m[V_ATTR] = ATTR_CODES[ch]
            else:
                self.s.put(m[V_ROW], m[V_COL], ch, m[V_ATTR])
                m[V_COL] += 1
        self.used_strings.append((start, si - 1))
        self.si = si


IBM2ANSI = [0, 4, 2, 6, 1, 5, 3, 7]


def sgr(attr):
    fg, bg, blink = attr & 0x0F, (attr >> 4) & 0x07, attr & 0x80
    codes = [0]
    codes.append((90 + IBM2ANSI[fg - 8]) if fg >= 8 else (30 + IBM2ANSI[fg]))
    codes.append(40 + IBM2ANSI[bg])
    if blink:
        codes.append(5)
    return "\x1b[" + ";".join(map(str, codes)) + "m"


def dump(screen, dst, name):
    txt = []
    ans = []
    att = []
    for row in screen.cells:
        txt.append("".join(bytes([c]).decode("cp437") for c, a in row).rstrip())
        line = ""
        last = None
        for c, a in row:
            if a != last:
                line += sgr(a)
                last = a
            line += bytes([c]).decode("cp437")
        ans.append(line + "\x1b[0m")
        att.append("".join("%02X" % a for c, a in row))
    with open(os.path.join(dst, name + ".txt"), "w", encoding="utf-8") as fh:
        fh.write("\n".join(txt) + "\n")
    with open(os.path.join(dst, name + ".ans"), "w", encoding="utf-8") as fh:
        fh.write("\n".join(ans) + "\n")
    with open(os.path.join(dst, name + ".attr"), "w") as fh:
        fh.write("\n".join(att) + "\n")


def build(img, ranges, scroll_fill_at):
    """scroll_fill_at：該函式開頭的 25 次 sub_105C7 迴圈，驗證位元組後把整個畫面填 0x66 屬性。"""
    s = Screen()
    # 驗證 sub_105C7：mov ah,7 / mov bh,66h / mov al,1 / mov ch,0 / mov cl,0 / mov dl,4Fh / mov dh,18h，整個 80x25 視窗往下捲 1 列
    ref = bytes.fromhex("b407b766b001b500b100b24fb618")
    if img[0x105C7 - BASE:0x105C7 - BASE + len(ref)] != ref:
        raise ValueError("sub_105C7 位元組不符")
    if img[scroll_fill_at - BASE:scroll_fill_at - BASE + 3] != bytes.fromhex("b91900"):
        raise ValueError("捲動迴圈次數不是 25")
    s.fill(0x66)
    trace = ["%05X scroll-down x25 fill attr 66h" % scroll_fill_at]
    vm = VM(img, s, trace)
    for a, b in ranges:
        vm.run(a, b)
    return s, trace, vm


def main(argv):
    src, dst = argv[1], argv[2]
    img = open(src, "rb").read()
    if len(img) != 95908:
        raise SystemExit("S.unpacked.bin 大小不符，預期 95908")
    os.makedirs(dst, exist_ok=True)
    screens = {
        # 主選單：sub_10297 的繪製段，再加 sub_10A90 的 AdLib 偵測訊息（兩種結果各一份）
        "menu-noadlib": ([(0x102BF, 0x10551), (0x10A90, 0x10AAB), (0x10AB5, 0x10ABB)], 0x102AA),
        "menu-adlib": ([(0x102BF, 0x10551), (0x10A90, 0x10AAB), (0x10AC7, 0x10ACD)], 0x102AA),
        # PASSWORD：sub_10671 的繪製段，到 call loc_12DD0 之前
        "password": ([(0x10699, 0x10971)], 0x10684),
    }
    layout = {"input": {"file": os.path.basename(src), "size": len(img)},
              "note": "dseg 只讀到 08BDh 之前；08BDh 起的密碼查表不匯出", "screens": {}}
    for name, (ranges, sf) in screens.items():
        s, trace, vm = build(img, ranges, sf)
        dump(s, dst, name)
        layout["screens"][name] = {
            "ranges": ["%05X-%05X" % r for r in ranges],
            "scroll_fill_loop": "%05X" % sf,
            "strings_ds_offsets": ["%04X-%04X" % (a, b) for a, b in vm.used_strings],
            "trace": trace,
        }
    # 其他字串（由 sub_10A35 / sub_10A49 在選色時印出）：只列內容，不重播
    extra = {}
    for off in (0x2A9, 0x2D0, 0x2D7, 0x2DE, 0x2E5, 0x2EC, 0x364, 0x370, 0x37A):
        j = img.index(b"$", DS - BASE + off) - (DS - BASE)
        assert j < LIMIT
        extra["%04X" % off] = bytes(img[DS - BASE + off: DS - BASE + j]).decode("cp437")
    layout["other_strings"] = extra
    with open(os.path.join(dst, "layout.json"), "w", encoding="utf-8") as fh:
        json.dump(layout, fh, ensure_ascii=False, indent=1)
    for n in screens:
        print(n, "ok")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
