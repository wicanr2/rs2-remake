#!/usr/bin/env python3
"""Android 觸控版面的設計稿，輸出 docs/android/mockups/*.svg。純文字，沒有相依，也不含原版美術。

用法：android_mockups.py <輸出目錄>
尺寸與 crates/rs2-play/src/touch.rs 的 layout 相同，單位是 dp。
"""
import sys

OUT = sys.argv[1]
FONT = "'Noto Sans CJK TC','Noto Sans TC','Microsoft JhengHei',sans-serif"
BG = "#10131c"
GAME = "#2b3350"
GAME_EDGE = "#55608a"
CTRL = "#e8ecf8"
ACCENT = "#ffb454"
NOTE = "#9fb0d8"


def text(x, y, s, size=13, fill=CTRL, anchor="middle", weight="400"):
    return f'<text x="{x}" y="{y}" font-family="{FONT}" font-size="{size}" fill="{fill}" text-anchor="{anchor}" font-weight="{weight}">{s}</text>'


def circle(x, y, r, label, sub=None, fill=CTRL, opacity=0.28):
    out = [f'<circle cx="{x}" cy="{y}" r="{r}" fill="{fill}" fill-opacity="{opacity}" stroke="{fill}" stroke-width="1.5"/>']
    out.append(text(x, y + (0 if sub else 5), label, 15 if r >= 30 else 12, CTRL, weight="700"))
    if sub:
        out.append(text(x, y + 16, sub, 10, CTRL))
    return "".join(out)


def dpad(x, y, r):
    out = [f'<circle cx="{x}" cy="{y}" r="{r}" fill="{CTRL}" fill-opacity="0.18" stroke="{CTRL}" stroke-width="1.5"/>']
    # 八個扇區的分界線。
    import math
    for i in range(8):
        a = math.radians(22.5 + 45 * i)
        x0, y0 = x + 16 * math.cos(a), y + 16 * math.sin(a)
        x1, y1 = x + r * math.cos(a), y + r * math.sin(a)
        out.append(f'<line x1="{x0:.1f}" y1="{y0:.1f}" x2="{x1:.1f}" y2="{y1:.1f}" stroke="{CTRL}" stroke-opacity="0.35"/>')
    for label, dx, dy in (("上", 0, -36), ("下", 0, 44), ("左", -40, 5), ("右", 40, 5)):
        out.append(text(x + dx, y + dy, label, 15, CTRL, weight="700"))
    return "".join(out)


def game_rect(w, h):
    gw = h * 1.6
    gx = (w - gw) / 2
    return gx, gw, f'<rect x="{gx}" y="0" width="{gw}" height="{h}" fill="{GAME}" stroke="{GAME_EDGE}"/>'


def phone(name, w, h, body, title, caption):
    pad = 18
    full_h = h + 2 * pad + 62
    svg = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{w + 2 * pad}" height="{full_h}" viewBox="0 0 {w + 2 * pad} {full_h}">',
        f'<rect width="100%" height="100%" fill="{BG}"/>',
        text(pad, 24, title, 16, CTRL, "start", "700"),
        text(pad, 44, caption, 12, NOTE, "start"),
        f'<g transform="translate({pad},{pad + 44})">',
        f'<rect width="{w}" height="{h}" fill="#000" rx="14" stroke="#3a4260" stroke-width="2"/>',
        body,
        "</g></svg>",
    ]
    with open(f"{OUT}/{name}.svg", "w", encoding="utf-8") as f:
        f.write("".join(svg))


def play(w, h):
    gx, gw, r = game_rect(w, h)
    b = [r, text(gx + gw / 2, h / 2, "遊戲畫面 320×200（16:10，置中）", 14, NOTE)]
    b.append(dpad(68, h - 78, 64))
    b.append(circle(w - 62, h - 62, 42, "開火", "按住充能", ACCENT, 0.32))
    b.append(circle(w - 62, h - 146, 30, "轉向"))
    b.append(circle(w - 38, 38, 24, "暫停"))
    b.append(circle(38, 38, 24, "☰"))
    return "".join(b)


def menu(w, h):
    gx, gw, r = game_rect(w, h)
    b = [r, text(gx + gw / 2, h / 2, "標題、過場（玩家機不在畫面上）", 14, NOTE)]
    b.append(dpad(68, h - 78, 64))
    b.append(circle(w - 62, h - 62, 42, "確定", "空白鍵", ACCENT, 0.32))
    b.append(circle(w - 62, h - 146, 30, "選單", "C"))
    b.append(circle(w - 38, 38, 24, "暫停"))
    b.append(circle(38, 38, 24, "☰"))
    return "".join(b)


def bar(w, h):
    gx, gw, r = game_rect(w, h)
    b = [r]
    labels = ["補間", "幀數", "HD／原版", "模式", "手感", "關於", "離開"]
    bw = (w - 76) / len(labels)
    b.append(f'<rect x="0" y="0" width="{w}" height="64" fill="#1c1e28" fill-opacity="0.94"/>')
    for i, l in enumerate(labels):
        x = 70 + i * bw
        b.append(f'<rect x="{x:.1f}" y="9" width="{bw - 6:.1f}" height="46" rx="6" fill="#343a50"/>')
        b.append(text(x + (bw - 6) / 2, 37, l, 14, CTRL, weight="700"))
    b.append(circle(38, 32, 24, "☰"))
    b.append(text(gx + gw / 2, h / 2 + 10, "工具列展開時遊戲暫停，按鈕高 46 dp", 14, NOTE))
    return "".join(b)


def hand(w, h):
    gx, gw, r = game_rect(w, h)
    b = [r, text(gx + gw / 2, h / 2, "滑到另一個扇區就換方向，手指不用離開", 14, NOTE)]
    x, y = 68, h - 78
    b.append(dpad(x, y, 64))
    # 手指軌跡：右上斜向。
    b.append(f'<path d="M {x+10} {y+6} Q {x+40} {y-6} {x+48} {y-46}" fill="none" stroke="{ACCENT}" stroke-width="4" stroke-linecap="round"/>')
    b.append(f'<circle cx="{x+48}" cy="{y-46}" r="10" fill="{ACCENT}"/>')
    b.append(text(x + 96, y - 56, "右上＝↑＋→", 13, ACCENT, "start", "700"))
    b.append(f'<circle cx="{x}" cy="{y}" r="92" fill="none" stroke="{CTRL}" stroke-dasharray="4 5" stroke-opacity="0.5"/>')
    b.append(text(x + 104, y + 62, "判定範圍比外觀大", 11, NOTE, "start"))
    b.append(circle(w - 62, h - 62, 42, "開火", "按住充能", ACCENT, 0.32))
    b.append(circle(w - 62, h - 146, 30, "轉向"))
    return "".join(b)


W, H = 960, 432  # 2400×1080、400 dpi 的橫向手機
phone("01-play", W, H, play(W, H), "遊玩中", "20:9 手機橫向，960×432 dp。方向在左，開火與轉向在右，半透明")
phone("02-menu", W, H, menu(W, H), "標題與過場", "右手的兩顆鈕換成確定與選單，其餘相同")
phone("03-toolbar", W, H, bar(W, H), "工具列", "按 ☰ 展開，對應桌面版的 F1 到 F5、關於、離開")
phone("04-dpad", W, H, hand(W, H), "方向盤的手感", "八個扇區，中心留死區，手指可以滑動換方向")
phone("05-tablet", 800, 500, play(800, 500), "16:10 平板", "畫面填滿，控制鈕疊在遊戲上，靠半透明不擋視線")
