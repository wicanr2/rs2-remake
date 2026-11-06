#!/usr/bin/env python3
"""單色 HD 全量計畫前的探測：列出每個單色圖庫項與彩色圖庫項的對應、遮罩、相位。不呼叫外部服務。

用法：hd_mono_probe.py <original/rs-2> <workspace> <輸出資料夾名>
輸出：workspace/hd/<輸出資料夾名>/probe/items.tsv、bh2-phases.txt、map.txt
"""
import json
import os
import struct
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_mono_lib import MonoSource  # noqa: E402
from rs2_common import unrle  # noqa: E402

src, ws, outname = sys.argv[1:4]
out = os.path.join(ws, "hd", outname, "probe")
os.makedirs(out, exist_ok=True)
S = MonoSource(src)
U = json.load(open(os.path.join(ws, "extracted/sprite-usage.json"), encoding="utf-8"))
E = {(e["lib"], e["index"]): e for e in U["entries"]}
IMG = {"BEG1.BDB": "BEG1", "BH1.BDB": "BH1", "BH2.BDB": "BH2", "BH4.BDB": "BH4", "J1.GOB": "J1", "J2.GOB": "J2", "J3.GOB": "J3", "J4.GOB": "J4",
       "MY.BDB": "MY", "SHT1.BDB": "SHT1", "SHT2.BDB": "SHT2", "SHT3.BDB": "SHT3", "SHT4.BDB": "SHT4", "SHT5.BDB": "SHT5"}
MASK = {"MY.BDB": "MYB.BDT", "SHT1.BDB": "SHT1.BDH", "SHT2.BDB": "SHT2.BDH", "SHT3.BDB": "SHT3.BDH", "SHT4.BDB": "SHT4.BDH", "SHT5.BDB": "SHT5.BDH"}
lines = ["bank\tidx\tstride\th\tlogw\tcolour\tstatus\trole\tgroup\tmask_idx\tmask_used\tmask_wmatch\tdraw"]
for bank, lib in IMG.items():
    for i, it in sorted(S.bank(bank).items()):
        e = E.get((lib, i))
        if it["height"] <= 0 or it["stride"] == 0 or (it["height"] & 0x8000):
            lines.append(f"{bank}\t{i}\t{it['stride']}\t{it['height']}\tsentinel")
            continue
        logw = it["stride"] * 4
        col = "-"
        st = role = grp = mi = mu = mw = dm = "-"
        if e:
            cw, ch = e["size"]
            col = f"{cw}x{ch}" + ("=" if cw == logw and abs(ch - it["height"]) <= 2 else "!")
            st, role, grp = e["status"], e["role"], e["group"]
            mk = e["mask"]
            mi, mu = mk.get("index"), mk.get("used")
            dm = ",".join(sorted({d.split("(")[0].split("+")[0] for d in e["draw_modes"]}))[:50]
            if bank in MASK and mi is not None and mi in S.bank(MASK[bank]):
                mw = S.item(MASK[bank], mi)["stride"] == it["stride"]
        lines.append(f"{bank}\t{i}\t{it['stride']}\t{it['height']}\t{logw}x{it['height']}\t{col}\t{st}\t{role}\t{grp}\t{mi}\t{mu}\t{mw}\t{dm}")
open(os.path.join(out, "items.tsv"), "w", encoding="utf-8").write("\n".join(lines) + "\n")

# BH2 相位：單色資料裡每個成員對基礎圖的最佳橫向循環位移（單色像素）與吻合率
ph = []
for base, members in ((5, range(6, 13)), (13, range(14, 17)), (17, [18]), (19, range(20, 23))):
    b = S.plane("BH2.BDB", base)
    ph.append(f"base {base} {b.shape}")
    for i in members:
        t = S.plane("BH2.BDB", i)
        if t.shape != b.shape:
            ph.append(f"  {i} shape {t.shape} != base")
            continue
        best = max(range(b.shape[1]), key=lambda k: int((np.roll(b, k, axis=1) == t).sum()))
        ph.append(f"  {i} shift {best} match {float((np.roll(b, best, axis=1) == t).mean()):.4f}")
open(os.path.join(out, "bh2-phases.txt"), "w", encoding="utf-8").write("\n".join(ph) + "\n")

# 第三關 SHT.SCR 的圖塊配置
raw = unrle(open(os.path.join(src, "SHT.SCR"), "rb").read(), 0)[0]
G = np.array(struct.unpack_from("<12000H", raw, 0), np.int32).reshape(20, 600)
raw2 = unrle(open(os.path.join(src, "SHT.SCN"), "rb").read(), 0)[0]
N = np.array(struct.unpack_from("<12000H", raw2, 0), np.int32).reshape(20, 600)
used = set(G.ravel().tolist()) - {0}
bank = S.bank("MAP.BDB")
ml = [f"SCR 非空格 {(G != 0).sum()} 唯一圖塊 {len(used)}；SCN 非空格 {(N != 0).sum()}；非空位置一致率 {((G != 0) == (N != 0)).mean():.4f}",
      f"MAP.BDB 單色項數 {len(bank)}，未被 SCR 使用 {sorted(set(bank) - used)[:60]}"]
sizes = {}
for k, it in bank.items():
    sizes.setdefault((it["stride"], it["height"]), []).append(k)
ml.append("尺寸分佈 " + str({k: len(v) for k, v in sizes.items()}))
ml.append(f"tile 0 = {S.plane('MAP.BDB', 0).sum()} 亮點；SCR 中值為 0 的格 {(G == 0).sum()}")
open(os.path.join(out, "map.txt"), "w", encoding="utf-8").write("\n".join(ml) + "\n")
print("done", out)
