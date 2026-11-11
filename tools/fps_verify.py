#!/usr/bin/env python3
"""由 tools/rust.sh play-fps-check 在 Docker／Xvfb 跑規格 008。

展示快速入口只用於同環境效能量測。正常玩家路徑另呼叫 poster_verify.py，兩者分開記錄。
"""
import hashlib
import json
import os
import re
import subprocess as sp
import sys
from pathlib import Path


def main():
    binary, out = map(Path, sys.argv[1:])
    assert out.parent.is_dir() and out.parent.stat().st_uid == os.getuid()
    assert not out.exists(), '拒絕覆寫既有幀率收據'
    out.mkdir()
    measurements = {}
    for mode in ['off', 'classic', 'modern']:
        log = out / f'{mode}.log'
        with log.open('w') as stream:
            sp.run([str(binary), '--orig', '/orig', '--hd', '/hd', '--config', str(out / 'benchmark-config'),
                    '--no-audio', '--start', 'demo', '--poster', mode, '--max-secs', '6'],
                   check=True, timeout=25, stdout=stream, stderr=stream)
        text = log.read_text()
        m = re.search(r'\[display\] target=(\d+) fps=([\d.]+) frames=(\d+) seconds=([\d.]+)', text)
        assert m, text[-1000:]
        target, fps, frames, seconds = int(m[1]), float(m[2]), int(m[3]), float(m[4])
        steps = int(re.search(r'\[perf\] 步進 (\d+) 次', text)[1])
        assert target == 60 and 58 <= fps <= 60.5, (mode, fps)
        assert 54 <= steps <= 56, (mode, steps)
        measurements[mode] = dict(target=target, fps=fps, frames=frames, seconds=seconds, logic_steps=steps)
    sp.run([sys.executable, '/project/tools/poster_verify.py', str(binary), str(out / 'gui')], check=True, timeout=150)
    result = dict(binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), measurements=measurements,
                  benchmark='fixed demo input; Xvfb SDL software; 4 CPU 3GiB',
                  normal_player_path='gui/receipt.json')
    (out / 'receipt.json').write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(result, ensure_ascii=False))


if __name__ == '__main__':
    main()
