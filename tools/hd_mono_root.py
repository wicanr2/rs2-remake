#!/usr/bin/env python3
"""組出可以給 rs2-play --hd 用的暫存 HD 根目錄：只把試稿項目換成新圖，其他一律符號連結到現況 assets。不複製圖、不改 assets。

用法：hd_mono_root.py <workspace> <輸出資料夾名> <標籤=批次名> ...
  例：hd_mono_root.py /w pilot-mono-review m1=pilot-mono-m1 m2=pilot-mono-m2
輸出：workspace/hd/<輸出資料夾名>/root-<標籤>/
  <assets 底下除 mono 以外的每一項>  -> ../../assets/<項>
  mono/<圖庫檔名>/                  批次有改的圖庫：資料夾內每張圖各一個符號連結（改過的指向批次，其餘指向 assets）
                                    批次沒動的圖庫：整個資料夾連結到 assets/mono/<圖庫檔名>
連結一律是相對路徑，整個 workspace/hd 掛進容器（唯讀）也解得開。重跑會先清掉自己建過的 root-<標籤>（只刪符號連結與空資料夾）。
"""
import os
import sys

ws = sys.argv[1]
outname = sys.argv[2]
pairs = [a.split("=", 1) for a in sys.argv[3:]]
hd = os.path.join(ws, "hd")
assets = os.path.join(hd, "assets")


def wipe(path):
    """只刪符號連結、檔案與空資料夾，由內而外。這個目錄只有本工具建立。"""
    if not os.path.lexists(path):
        return
    if os.path.islink(path) or os.path.isfile(path):
        os.unlink(path)
        return
    for n in os.listdir(path):
        wipe(os.path.join(path, n))
    os.rmdir(path)


for label, batch in pairs:
    root = os.path.join(hd, outname, f"root-{label}")
    bm = os.path.join(hd, batch, "mono")
    if not os.path.isdir(bm):
        raise SystemExit(f"沒有 {bm}")
    wipe(root)
    os.makedirs(os.path.join(root, "mono"))
    for e in sorted(os.listdir(assets)):
        if e == "mono":
            continue
        os.symlink(os.path.join("..", "..", "assets", e), os.path.join(root, e))
    changed = 0
    for bank in sorted(os.listdir(os.path.join(assets, "mono"))):
        a_dir = os.path.join(assets, "mono", bank)
        b_dir = os.path.join(bm, bank)
        if not os.path.isdir(b_dir):
            os.symlink(os.path.join("..", "..", "..", "assets", "mono", bank), os.path.join(root, "mono", bank))
            continue
        d = os.path.join(root, "mono", bank)
        os.makedirs(d)
        over = set(os.listdir(b_dir))
        for f in sorted(os.listdir(a_dir)):
            if f in over:
                os.symlink(os.path.join("..", "..", "..", "..", batch, "mono", bank, f), os.path.join(d, f))
                changed += 1
            else:
                os.symlink(os.path.join("..", "..", "..", "..", "assets", "mono", bank, f), os.path.join(d, f))
        extra = over - set(os.listdir(a_dir))
        if extra:
            raise SystemExit(f"{batch}/mono/{bank} 有 assets 沒有的檔案：{sorted(extra)[:5]}")
    # 檢查每個連結都解得開
    bad = []
    for dp, dn, fn in os.walk(os.path.join(root, "mono")):
        for f in fn + dn:
            p = os.path.join(dp, f)
            if os.path.islink(p) and not os.path.exists(p):
                bad.append(p)
    if bad:
        raise SystemExit(f"連結解不開：{bad[:5]}")
    print(f"root-{label}：{changed} 張改成 {batch}")
