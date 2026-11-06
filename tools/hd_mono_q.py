#!/usr/bin/env python3
"""單色 HD 全量工作的臨時查詢：在 rs2-hdtools 容器內執行 <workspace>/hd/<批次>/q/<檔名>.py，並預先提供 S、E、np、Image。

用法：hd_mono_q.py <original/rs-2> <workspace> <批次名> <檔名（不含 .py）> [參數...]
腳本內可用變數：src、ws、root（批次資料夾）、args、S（MonoSource）、E（sprite-usage 的 (lib, index) 字典）、np、Image。
"""
import json
import os
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_mono_lib import MonoSource  # noqa: E402
from rs2_common import unrle  # noqa: E402,F401

src, ws, batch, name = sys.argv[1:5]
args = sys.argv[5:]
root = os.path.join(ws, "hd", batch)
S = MonoSource(src)
E = {(e["lib"], e["index"]): e for e in json.load(open(os.path.join(ws, "extracted/sprite-usage.json"), encoding="utf-8"))["entries"]}
code = open(os.path.join(root, "q", name + ".py"), encoding="utf-8").read()
exec(compile(code, name + ".py", "exec"), globals())
