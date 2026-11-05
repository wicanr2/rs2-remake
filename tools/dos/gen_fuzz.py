#!/usr/bin/env python3
"""差分模糊測試的腳本產生器：同一組輸入餵給 dosgolem（rs2dyn）跑原版與 Rust 重製，逐幀比對。

用法（容器內，只用標準函式庫，不用內建 random；一般經 tools/dos/fuzz/gen.sh 與 check.sh 呼叫）：

  python -I gen_fuzz.py PROFILE SEED [--frames N] [--ds-every N] [--scenarios DIR] [--fuzz DIR]
  python -I gen_fuzz.py check 腳本.txt 收據.jsonl      用收據的 in1 驗證送鍵時序（鍵有沒有準時進當幀），不符回傳 1

產生兩個檔，同一組 PROFILE 與 SEED 輸出逐位元組相同：

  <scenarios>/fuzz-PROFILE-SEED.txt   rs2dyn 腳本（include prefix-to-stage1、trace、play now 與 poke 交錯）
  <fuzz>/PROFILE-SEED.pokes           每行 `n 線性位址 寬 值`：在取樣第 n-1 筆之後、第 n 步之前寫入
                                      （n 是追蹤筆數，線性位址是 IDA 位址，寬 1 或 2，值是十六進位）

輸入位元組給 `play now`（rs2dyn play.go）：
  01 上  02 下  04 右  08 左  10 開火（右 Shift 按放一次）  20 轉向（/ 鍵按放一次）
  40 右 Shift 持續按住  80 這一幀多送一個右 Shift 按下碼（模擬鍵盤自動重複）
斜向鍵（Home、PgUp、End、PgDn）不在位元裡，用 `down KEY` 與 `up KEY` 在 play 之間送出。

腳本固定從第一關第 0 筆追蹤開始，先跑 5 筆不送輸入，之後才 poke 與 play。結尾 `settle` 讓最後一筆所在的
半步走完並寫 trace-fz.end（rows 與離開碼），`trace off`。玩家死亡等讓主迴圈離開時 play 自己停下，
之後的 poke 與 play 不再有作用（.pokes 裡 n 超過實際筆數的列，重播時略過）。

Profile：
  move     不改任何變數，隨機方向（含兩鍵同按與相反方向）、開火、轉向，自然遊玩到死亡或幀數用完
  seeker   兩座砲座、B 武器、不減血，交替朝左朝右開火
  laser    兩座砲座、L 武器、不減血，交替朝向並移動
  spread   兩座砲座、S 與 K 武器逐段交替、不減血
  charge   兩座砲座、不減血，按住右 Shift（40）加重複（80）累積充能超過 14，放開觸發掃射，重複多次
  mix      兩座砲座、不減血，武器在 K S B L F 與普通彈之間隨機換，方向、斜向鍵、轉向都隨機
  roam     只開不減血，其餘同 move：玩家不死，在整個第一關（含頭目段）到處走動與開火
  sweep    隨機難度加不減血，依序走向隨機目標格（一半在右半邊）再停留，開火模式隨機，用來讓敵人從各種相對位置開火
  carrier  同 sweep，但前 180 到 230 幀貼著右邊緣不開火，讓道具載體（追蹤模式）從右邊追上來，轉向表才會輪到各種朝向
  hot      同 roam，但在中途做一件事：按住 Ctrl 加 R、Q 或 C 之一（sub_10D99 熱鍵，主迴圈以 E、Q、C 離開），
           或按 Esc 暫停再按一次恢復（sub_10ECB）。只選第一半步取樣之後的邊界，理由見 rs2dyn patch 0005 的 README
  late     同 roam，但從第 640 筆起（頭目段）恢復受傷並把血量設成 3，玩家在頭目戰中死亡

seeker、laser、spread、charge、mix、roam、sweep、carrier、hot、late 在第 5 筆另外寫隨機難度（dhp、dfire 兩個字），move 不寫任何變數。
"""
import os
import re
import sys

M64 = (1 << 64) - 1
LEAD = 5  # 追蹤開頭先跑幾筆不送輸入再 poke（等關卡初始化完）
DS_EVERY = 40  # 每隔幾筆另存整個資料段（ds-fz-n.ds），逐幀比對時核對 JSONL 沒涵蓋的變數

# 位址都是 IDA 線性位址（docs/re/10 第 8.3 節與 11 第 9.2 節）。
POD1, POD2 = 0x1DA8D, 0x1DA8E
WPN = 0x1DAA9
NODMG = 0x273A5
HP1 = 0x1CFDE  # 玩家 1 血量
DHP, DFIRE = 0x1E6B2, 0x1E6B4  # 難度：玩家彈傷害加成與敵人開火率，Start::NewGame { difficulty: Some(d) } 的兩個字
DIFFS = ((0, 0), (0, 2), (1, 2), (1, 3))
Y = 0x59
W_K, W_S, W_B, W_L, W_F, W_N = 0x4B, 0x53, 0x42, 0x4C, 0x46, 0x4E

PROFILES = ("move", "seeker", "laser", "spread", "charge", "mix", "roam", "sweep", "carrier", "hot", "late")
DIAG = ("home", "pgup", "end", "pgdn")


class Lcg:
    """Knuth MMIX 的 64 位元線性同餘，取高 31 位元。"""

    def __init__(self, seed):
        self.s = (seed * 0x9E3779B97F4A7C15 + 0x5DEECE66D) & M64
        for _ in range(8):
            self.next()

    def next(self):
        self.s = (self.s * 6364136223846793005 + 1442695040888963407) & M64
        return self.s >> 33

    def below(self, n):
        return self.next() % n

    def range(self, lo, hi):
        """閉區間 [lo, hi]。"""
        return lo + self.below(hi - lo + 1)

    def chance(self, num, den):
        return self.below(den) < num

    def pick(self, seq):
        return seq[self.below(len(seq))]


# 方向位元組：0 不動，單向，兩鍵同按（斜向走兩個方向鍵），相反方向同按。
DIR_STILL = 0x00
DIR_SINGLE = (0x01, 0x02, 0x04, 0x08)
DIR_PAIR = (0x05, 0x09, 0x06, 0x0A)
DIR_OPP = (0x03, 0x0C, 0x0F)


def pick_dir(r, still_w):
    """still_w：不動的權重（百分比），其餘依 單向 50、斜向 35、相反 5 分。"""
    if r.chance(still_w, 100):
        return DIR_STILL
    k = r.below(90)
    if k < 45:
        return r.pick(DIR_SINGLE)
    if k < 85:
        return r.pick(DIR_PAIR)
    return r.pick(DIR_OPP)


def run_len(r):
    """方向維持幾幀：多數短，偶爾長到撞邊界。"""
    if r.chance(1, 6):
        return r.range(15, 40)
    return r.range(2, 12)


def fire_bits(r, mode, i):
    """mode 0 不開火、1 每幀、2 隔一幀、3 每三幀。"""
    if mode == 1:
        return 0x10
    if mode == 2:
        return 0x10 if i % 2 == 0 else 0
    if mode == 3:
        return 0x10 if i % 3 == 0 else 0
    return 0


def frames_free(r, n, still_w, fire_modes, turn_den):
    """自由遊玩的 n 個位元組。turn_den：平均每幾幀轉向一次（0 不轉）。"""
    out = []
    while len(out) < n:
        d = pick_dir(r, still_w)
        mode = r.pick(fire_modes)
        for i in range(run_len(r)):
            b = d | fire_bits(r, mode, i)
            if turn_den and r.chance(1, turn_den):
                b |= 0x20
            out.append(b)
    return out[:n]


def frames_facing(r, n, still_w, turn_lo, turn_hi):
    """每幀開火，每隔 turn_lo 到 turn_hi 幀轉向一次（朝向在左右之間交替）。"""
    out = []
    until = r.range(turn_lo, turn_hi)
    while len(out) < n:
        d = pick_dir(r, still_w)
        for _ in range(run_len(r)):
            b = d | (0x10 if r.chance(9, 10) else 0)
            until -= 1
            if until <= 0:
                b |= 0x20
                until = r.range(turn_lo, turn_hi)
            out.append(b)
    return out[:n]


def frames_charge(r, n):
    """充能循環：先點一下右 Shift 武裝，之後重複 按住 H 幀、放開 1 到 6 幀。
    首幀 40（按下），其後 reps 幀帶 80（多一個按下碼，充能加 1），其餘幀只維持按住。
    reps 常落在結算門檻 14 的前後（13、14、15），另有少於與遠大於的。
    一半的循環不夾方向鍵，因為按住期間任何鍵的 INT 9 事件也會讓充能加 1，會把 reps 弄亂。"""
    out = [0x10, 0x00]
    while len(out) < n:
        reps = r.pick((r.range(2, 12), 13, 14, 14, 15, 15, 16, r.range(17, 30)))
        hold = reps + 1 + r.range(0, 4)
        mid = [0xC0] * reps + [0x40] * (hold - 1 - reps)
        for i in range(len(mid) - 1, 0, -1):  # Fisher-Yates
            j = r.below(i + 1)
            mid[i], mid[j] = mid[j], mid[i]
        noisy = r.chance(1, 2)
        d = pick_dir(r, 40)
        for i, b in enumerate([0x40] + mid):
            if noisy and r.chance(1, 4):
                b |= d
            if r.chance(1, 25):
                b |= 0x20
            out.append(b)
        for _ in range(r.range(1, 6)):
            b = 0
            if r.chance(1, 3):
                b |= pick_dir(r, 30)
            if r.chance(1, 20):
                b |= 0x20
            out.append(b)
    return out[:n]


def walk(r, out, pos, tc, tr, mode, dwell):
    """從 pos 走向 (tc, tr) 再停留 dwell 幀，位元組接在 out 後面。位置照 sub_146AC 等的規則推算：
    列 0 到 16 每步 1，欄 1 到 33（奇數）每步 2。"""
    col, row = pos
    i = 0
    while (col, row) != (tc, tr):
        b = 0
        if col < tc:
            b |= 0x04
            col += 2
        elif col > tc:
            b |= 0x08
            col -= 2
        if row < tr:
            b |= 0x02
            row += 1
        elif row > tr:
            b |= 0x01
            row -= 1
        b |= fire_bits(r, mode, i)
        if r.chance(1, 40):
            b |= 0x20
        out.append(b)
        i += 1
    for _ in range(dwell):
        b = fire_bits(r, mode, i)
        if r.chance(1, 40):
            b |= 0x20
        out.append(b)
        i += 1
    return (col, row)


def frames_sweep(r, n, carrier_lead=0):
    """依序走向隨機目標格，到了停留一陣子再選下一個。目標一半落在右半邊，敵人從右邊進來。
    carrier_lead > 0：前這麼多幀貼著右邊緣（欄 29 到 33、列 2 到 9）不開火，等道具載體（模板 P，第 40、70、100、
    150、170、190 幀出現，從右邊飛進來朝玩家轉向）追上來。"""
    pos = (5, 10)
    out = []
    while len(out) < carrier_lead:
        pos = walk(r, out, pos, r.pick((29, 31, 33)), r.range(2, 9), 0, r.range(15, 50))
    while len(out) < n:
        tc = r.pick(range(17, 34, 2)) if r.chance(1, 2) else r.pick(range(1, 34, 2))
        pos = walk(r, out, pos, tc, r.range(0, 16), r.pick((0, 0, 1, 1, 2, 3)), r.range(3, 25))
    return out[:n]


def split_segments(r, bytes_, lo, hi):
    """依隨機長度切段，回傳 [(起始索引, 該段位元組)...]。"""
    segs, i = [], 0
    while i < len(bytes_):
        k = min(r.range(lo, hi), len(bytes_) - i)
        segs.append((i, bytes_[i:i + k]))
        i += k
    return segs


def diag_event(r, held):
    """切換一個斜向鍵：按著的放開，沒按的按下。held 是目前按著的鍵集合。"""
    if held and r.chance(2, 3):
        k = r.pick(sorted(held))
        held.discard(k)
        return ("key", "up", k)
    k = r.pick(DIAG)
    if k in held:
        held.discard(k)
        return ("key", "up", k)
    held.add(k)
    return ("key", "down", k)


def pod_pokes():
    return [("poke", POD1, 1, Y), ("poke", POD2, 1, Y), ("poke", NODMG, 2, Y)]


def diff_pokes(r):
    """隨機一種難度（四段之一），寫兩個難度字，等於 Start::NewGame { difficulty: Some(d) }。"""
    hp, fire = r.pick(DIFFS)
    return [("poke", DHP, 2, hp), ("poke", DFIRE, 2, fire)]


DEFAULT_FRAMES = {"move": (900, 1500), "seeker": (1100, 1500), "laser": (1100, 1500), "spread": (600, 1100),
                  "charge": (600, 1000), "mix": (1000, 1500), "roam": (1100, 1500), "sweep": (1100, 1500), "carrier": (1100, 1500), "hot": (500, 900), "late": (900, 1300)}


def build(profile, seed, frames):
    """回傳 (frames, [(事件列表, 位元組列表)...])。事件在該段 play 之前執行。"""
    r = Lcg(seed)
    held = set()  # 按著的斜向鍵
    if frames is None:
        frames = r.range(*DEFAULT_FRAMES[profile])
    segs_out = []
    if profile == "move":
        bs = frames_free(r, frames, 25, (0, 1, 1, 2, 3), 40)
        for idx, seg in split_segments(r, bs, 15, 90):
            ev = []
            if idx and r.chance(1, 3):
                ev.append(diag_event(r, held))
            segs_out.append((ev, seg))
    elif profile in ("seeker", "laser"):
        w = W_B if profile == "seeker" else W_L
        bs = frames_facing(r, frames, 35, 5, 22)
        for idx, seg in split_segments(r, bs, 40, 160):
            ev = diff_pokes(r) + pod_pokes() + [("poke", WPN, 2, w)] if idx == 0 else []
            if idx and r.chance(1, 4):
                ev.append(diag_event(r, held))
            segs_out.append((ev, seg))
    elif profile == "spread":
        bs = frames_facing(r, frames, 40, 4, 16)
        for idx, seg in split_segments(r, bs, 30, 100):
            w = W_S if len(segs_out) % 2 == 0 else W_K  # S 與 K 逐段交替
            ev = diff_pokes(r) + pod_pokes() if idx == 0 else []
            ev.append(("poke", WPN, 2, w))
            segs_out.append((ev, seg))
    elif profile == "charge":
        bs = frames_charge(r, frames)
        ev = diff_pokes(r) + pod_pokes() + [("poke", WPN, 2, r.pick((W_N, W_K, W_S, W_B, W_L)))]
        segs_out.append((ev, bs))
    elif profile == "mix":
        bs = frames_free(r, frames, 30, (1, 1, 2, 3), 12)
        for idx, seg in split_segments(r, bs, 20, 80):
            ev = diff_pokes(r) + pod_pokes() if idx == 0 else []
            ev.append(("poke", WPN, 2, r.pick((W_K, W_S, W_B, W_L, W_F, W_N))))
            if idx and r.chance(1, 3):
                ev.append(diag_event(r, held))
            segs_out.append((ev, seg))
    elif profile == "roam":
        bs = frames_free(r, frames, 20, (0, 1, 1, 2, 3), 30)
        for idx, seg in split_segments(r, bs, 25, 100):
            ev = diff_pokes(r) + [("poke", NODMG, 2, Y)] if idx == 0 else []
            if idx and r.chance(1, 3):
                ev.append(diag_event(r, held))
            segs_out.append((ev, seg))
    elif profile == "hot":
        bs = frames_free(r, frames, 20, (0, 1, 1, 2, 3), 30)
        segs = split_segments(r, bs, 25, 100)
        # 熱鍵與 Esc 暫停由第二半步的尾端（sub_10ECB、sub_10D99）檢查。鍵在取樣之後才送，原版的尾端會看到，
        # 重製的 step 卻在下一步開頭才收到，所以只能在第一半步（偶數筆）取樣之後送，下一筆（奇數）才是第二半步。
        cand = [i for i, (idx, _) in enumerate(segs) if 2 <= i <= len(segs) - 3 and (LEAD + idx) % 2 == 1]
        hot_at = r.pick(cand or [2])
        hot_key = r.pick(("r", "q", "c", "esc"))
        for idx, seg in segs:
            ev = diff_pokes(r) + [("poke", NODMG, 2, Y)] if idx == 0 else []
            k = segs.index((idx, seg))
            if k == hot_at and hot_key == "esc":
                # Esc 暫停（sub_10ECB）：按一次，跑到暫停迴圈 10ED8h，再按一次恢復。放開鍵留到下一段，
                # 恢復那一列的 sc 才只有一個按下碼。untilpc 期間多取樣的筆數不可預知，之後不再 poke。
                ev += [("key", "down", "esc"), ("cmd", "untilpc 0x10ED8"), ("key", "down", "esc"), ("cmd", "runsteps 5000")]
                # 恢復後下一步很快就讀輸入，這一段第一個位元組不送鍵，免得鍵趕不上。
                seg = [0] + seg[1:]
            elif k == hot_at + 1 and hot_key == "esc":
                ev += [("key", "up", "esc")]
            elif k == hot_at:
                ev += [("key", "down", "ctrl"), ("key", "down", hot_key)]
            segs_out.append((ev, seg))
    elif profile == "late":
        bs = frames_free(r, frames, 20, (0, 1, 1, 2, 3), 30)
        armed = False
        for idx, seg in split_segments(r, bs, 25, 100):
            ev = diff_pokes(r) + [("poke", NODMG, 2, Y)] if idx == 0 else []
            if not armed and LEAD + idx >= 640:
                ev += [("poke", NODMG, 2, 0x4E), ("poke", HP1, 2, 3)]
                armed = True
            segs_out.append((ev, seg))
    elif profile in ("sweep", "carrier"):
        bs = frames_sweep(r, frames, r.range(180, 230) if profile == "carrier" else 0)
        for idx, seg in split_segments(r, bs, 40, 200):
            ev = diff_pokes(r) + [("poke", NODMG, 2, Y)] if idx == 0 else []
            segs_out.append((ev, seg))
    else:
        raise SystemExit(f"未知 profile {profile}，可用：{' '.join(PROFILES)}")
    return frames, segs_out


def tokens(bs):
    """連續相同的位元組合成 HH*N。"""
    out, i = [], 0
    while i < len(bs):
        j = i
        while j < len(bs) and bs[j] == bs[i]:
            j += 1
        out.append(f"{bs[i]:02X}" if j - i == 1 else f"{bs[i]:02X}*{j - i}")
        i = j
    return out


def render(profile, seed, frames, segs, ds_every=DS_EVERY):
    lines = [
        f"# 差分模糊測試 profile={profile} seed={seed} frames={frames}。tools/dos/gen_fuzz.py 產生，不要手改。",
        "# 第一關從追蹤第 0 筆開始，先跑 %d 筆不送輸入，之後 poke 與 play now 交錯；對應的 poke 記在 tools/dos/fuzz/%s-%d.pokes。"
        % (LEAD, profile, seed),
        "include /tools-dos/scenarios/prefix-to-stage1.txt",
        "trace fz full",
        f"dsdump {ds_every}",
        f"untiltrace {LEAD} 400_000_000",
    ]
    pokes = []
    n = LEAD
    for ev, bs in segs:
        for e in ev:
            if e[0] == "poke":
                _, addr, width, val = e
                pokes.append(f"{n} 0x{addr:X} {width} 0x{val:X}")
                if width == 1:
                    lines.append(f"poke 0x{addr:X} {val:02X}")
                else:
                    lines.append(f"pokew 0x{addr:X} 0x{val:X}")
            elif e[0] == "cmd":
                lines.append(e[1])
            else:
                _, how, key = e
                lines.append(f"{how} {key}")
        lines.append("play now " + " ".join(tokens(bs)))
        n += len(bs)
    lines += ["settle", "trace off", ""]
    return "\n".join(lines), "\n".join(pokes) + ("\n" if pokes else "")


def check(scenario_path, trace_path):
    """用收據的 in1 驗證送鍵時序：每一列玩家 1 讀到的方向位元，要等於腳本在那一列送出的方向鍵狀態。

    鍵若遲到一幀（事件排在下一幀讀輸入之後），in1 會落後，差分比對會把它誤判成移植錯誤，所以先檢查這個。
    只比方向位元（in1 的 0F）。玩家血量歸零那幾幀與過關（clear = 46h）之後 in1 不更新，不比。
    """
    import json

    held_diag = set()
    per_row = {}  # n -> (byte, 斜向鍵集合)
    n = LEAD
    with open(scenario_path, encoding="utf-8") as f:
        for line in f:
            t = line.split()
            if not t:
                continue
            if t[0] in ("down", "up") and len(t) == 2:
                (held_diag.add if t[0] == "down" else held_diag.discard)(t[1])
            elif t[0] == "untilpc":
                break  # 暫停期間多取樣的筆數不可預知，之後的列對不上
            elif t[0] == "play" and t[1] == "now":
                for tok in t[2:]:
                    hh, _, cnt = tok.partition("*")
                    for _ in range(int(cnt) if cnt else 1):
                        per_row[n] = (int(hh, 16), frozenset(held_diag))
                        n += 1
    bad = checked = 0
    with open(trace_path, encoding="utf-8") as f:
        for i, line in enumerate(f):
            if i not in per_row:
                continue
            row = json.loads(line)
            if row["p1"]["hp"] == 0 or not row["alive"] & 1 or row["clear"] == 0x46:  # 過關後 sub_14458 直接返回
                continue
            b, dg = per_row[i]
            up = b & 1 or "home" in dg or "pgup" in dg
            down = b & 2 or "end" in dg or "pgdn" in dg
            left = b & 8 or "home" in dg or "end" in dg
            right = b & 4 or "pgup" in dg or "pgdn" in dg
            want = (1 if up else 2 if down else 0) | (8 if left else 4 if right else 0)
            got = row["in1"] & 0x0F
            checked += 1
            if got != want:
                bad += 1
                if bad <= 10:
                    print(f"  第 {i} 列：腳本 {b:02X} 斜向 {sorted(dg)} 預期方向 {want:X}，in1={row['in1']:02X}")
    print(f"{os.path.basename(trace_path)}：檢查 {checked} 列，方向位元不符 {bad} 列")
    return bad


def main(argv):
    if len(argv) == 4 and argv[1] == "check":
        raise SystemExit(1 if check(argv[2], argv[3]) else 0)
    args, opts = [], {}
    i = 1
    while i < len(argv):
        if argv[i].startswith("--"):
            if argv[i] not in ("--frames", "--scenarios", "--fuzz", "--ds-every") or i + 1 >= len(argv):
                raise SystemExit(f"{argv[i]} 不認得或缺參數")
            opts[argv[i]] = argv[i + 1]
            i += 2
        else:
            args.append(argv[i])
            i += 1
    if len(args) != 2:
        raise SystemExit(__doc__)
    profile, seed_s = args
    if profile not in PROFILES:
        raise SystemExit(f"未知 profile {profile}，可用：{' '.join(PROFILES)}")
    if not re.fullmatch(r"[0-9]{1,9}", seed_s):
        raise SystemExit("SEED 要是 1 到 9 位數字")
    seed = int(seed_s)
    frames = int(opts["--frames"]) if "--frames" in opts else None
    frames, segs = build(profile, seed, frames)
    scen, pokes = render(profile, seed, frames, segs, int(opts.get("--ds-every", DS_EVERY)))
    sdir = opts.get("--scenarios", "/dos/scenarios")
    fdir = opts.get("--fuzz", "/dos/fuzz")
    name = f"{profile}-{seed}"
    with open(os.path.join(sdir, f"fuzz-{name}.txt"), "w", encoding="utf-8") as f:
        f.write(scen)
    with open(os.path.join(fdir, f"{name}.pokes"), "w", encoding="utf-8") as f:
        f.write(pokes)
    print(f"{name}: frames={frames} segments={len(segs)} pokes={len(pokes.splitlines())}")


if __name__ == "__main__":
    main(sys.argv)
