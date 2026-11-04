#!/usr/bin/env python3
"""產生 source-manifest.json：輸入 zip 與每個原版檔的 SHA-256、大小、壓縮檔內時間與角色分類。

用法：make_manifest.py <zip> <輸出 json>
"""
import hashlib
import json
import sys
import zipfile

ROLE = {
    "S.EXE": "遊戲主程式（兩層 LZEXE，含破解組 intro，見 docs/re/01-boot-chain.md）",
    "ASMDRV.EXE": "AdLib 音樂驅動 TSR，INT 60h",
    "JGAME.EXE": "大宇 JOYMOUSE 按鍵設定工具 Ver 1.50",
    "JOYMOUSE.CFG": "JOYMOUSE 介面卡設定",
    "RS2.JOY": "JGAME 的 RS-2 按鍵對照資料",
    "RS2.BAT": "啟動批次檔",
    "SEN.EXE": "第三方破解 TSR（非原版）",
    "SEN.PAS": "SEN.EXE 的 Turbo Pascal 原始碼（非原版）",
    "FILE_ID.DIZ": "BBS 發行標示（非原版）",
    "A.MIX": "標題音樂", "B.MIX": "關卡音樂",
    "DM1.LCK": "展示模式輸入錄影，第一關", "DM2.LCK": "展示模式輸入錄影，第二關",
    "SHT.SCN": "第三關地圖（VGA/EGA）", "SHT.SCR": "第三關地圖（CGA/MGA）",
    "SHT4.GOT": "第四關遮罩庫（VGA/EGA）",
    "MYB.BDT": "主角共用圖庫遮罩（CGA/MGA）",
}
EXT = {"GO": "彩色圖庫（VGA/EGA 四平面）", "GOB": "單色圖庫（CGA/MGA）", "BDB": "單色圖庫（CGA/MGA）", "BDH": "單色遮罩庫（CGA/MGA）",
       "BD1": "開場彩色圖庫 平面0", "BD2": "開場彩色圖庫 平面1", "BD3": "開場彩色圖庫 平面2", "BD4": "開場彩色圖庫 平面3"}

z = zipfile.ZipFile(sys.argv[1])
raw = open(sys.argv[1], "rb").read()
files = []
for i in z.infolist():
    if i.is_dir():
        continue
    d = z.read(i)
    n = i.filename.split("/")[-1]
    files.append({"name": n, "size": len(d), "sha256": hashlib.sha256(d).hexdigest(),
                  "zip_mtime": "%04d-%02d-%02d %02d:%02d:%02d" % i.date_time,
                  "role": ROLE.get(n) or EXT.get(n.rsplit(".", 1)[-1], "未分類")})
m = {"archive": {"name": sys.argv[1].split("/")[-1], "size": len(raw), "sha256": hashlib.sha256(raw).hexdigest(), "entries": len(files)}, "files": files}
json.dump(m, open(sys.argv[2], "w", encoding="utf-8"), ensure_ascii=False, indent=1)
print(m["archive"], [f["name"] for f in files if f["role"] == "未分類"])
