#!/usr/bin/env bash
# 讓 git worktree 可以跑 tools/rust.sh 與 tools/dos/wait_test.sh。
# 原版資料與收據不進版控，worktree 裡沒有。這裡用符號連結接到主工作區，唯讀使用；建置輸出各自獨立，
# 不和主工作區搶 target 目錄。
#
# 用法：在 worktree 根目錄執行 /home/anr2/cht/rs2/tools/worktree_data.sh
set -euo pipefail

MAIN="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WT="$(pwd)"
[ "$WT" != "$MAIN" ] || { echo "這是主工作區，不用跑" >&2; exit 1; }
[ -e "$WT/.git" ] || { echo "$WT 不是 worktree 根目錄" >&2; exit 1; }

link() {
  local target="$1" name="$2"
  [ -e "$target" ] || { echo "缺 $target" >&2; exit 1; }
  [ -L "$name" ] || ln -s "$target" "$name"
}

link "$MAIN/original" original
mkdir -p workspace workspace/target workspace/target-play workspace/rs2 workspace/progress
# 這些目錄由 tools/rust.sh 用 docker -v 掛進容器，符號連結可以。
for d in cargo cargo-play dosrun hd ida unpacked dosgolem; do
  link "$MAIN/workspace/$d" "workspace/$d"
done
# 測試從 /project/workspace/extracted 讀，/project 是整個 worktree 掛進去的，容器裡看不到主工作區的絕對路徑，
# 符號連結會斷。這份要真的複製。
[ -d workspace/extracted ] || cp -r "$MAIN/workspace/extracted" workspace/extracted
# 測試從 RS2_OUT（workspace/rs2）讀的參考資料。這個目錄測試也會寫，所以各用各的，只複製要讀的。
cp -n "$MAIN"/workspace/rs2/pixel-hashes.tsv "$MAIN"/workspace/rs2/boot_ds.bin "$MAIN"/workspace/rs2/*.jsonl workspace/rs2/
mkdir -p workspace/rs2/audio
cp -n "$MAIN"/workspace/rs2/audio/*.pcm workspace/rs2/audio/ 2>/dev/null || true

# 符號連結不算目錄，.gitignore 的 original/ 比對不到。info/exclude 是所有 worktree 共用的。
exclude="$(git rev-parse --git-common-dir)/info/exclude"
grep -qx 'original' "$exclude" 2>/dev/null || echo 'original' >> "$exclude"

echo "worktree 資料已接好：$WT"
