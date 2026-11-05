#!/usr/bin/env python3
"""檢查 SDL 磁碟音訊驅動寫出的 raw PCM（s16le 單聲道）是否有聲音。用法：pkg_audio_check.py <檔案> [最少秒數] [取樣率]"""
import array
import sys

path = sys.argv[1]
min_secs = float(sys.argv[2]) if len(sys.argv) > 2 else 5.0
rate = int(sys.argv[3]) if len(sys.argv) > 3 else 22050
data = open(path, "rb").read()
a = array.array("h")
a.frombytes(data[: len(data) // 2 * 2])
n = len(a)
nz = sum(1 for x in a if x)
peak = max((abs(x) for x in a), default=0)
rms = (sum(x * x for x in a) / n) ** 0.5 if n else 0
secs = n / rate
ok = n > 0 and nz / n > 0.01 and peak > 200 and secs >= min_secs
print(f"audio：{n} 取樣（約 {secs:.1f} 秒 @ {rate} Hz），非零 {100 * nz / max(n, 1):.1f}%，峰值 {peak}，RMS {rms:.0f} -> {'非靜音' if ok else '靜音或太短'}")
sys.exit(0 if ok else 1)
