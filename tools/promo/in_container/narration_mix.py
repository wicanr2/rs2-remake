#!/usr/bin/env python3
"""在影片工具容器中檢查旁白、安排時碼，並讓配樂隨人聲降低音量。"""
import hashlib
import json
import subprocess
import sys
from pathlib import Path


def run(*args):
    subprocess.run(args, check=True, timeout=180)


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def stamp(seconds):
    ms = round(seconds * 1000)
    return f"{ms // 3600000:02}:{ms // 60000 % 60:02}:{ms // 1000 % 60:02},{ms % 1000:03}"


def main():
    folder, music = map(Path, sys.argv[1:])
    plan = json.loads((folder / "plan.json").read_text())
    total = plan["duration"]
    ff = ["ffmpeg", "-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-threads", "1"]
    inputs, filters, entries, subtitles = [], [], [], []
    end_previous = 0
    for i, segment in enumerate(plan["segments"]):
        p = folder / (segment["id"] + ".mp3")
        start, end = segment["start"], segment["end"]
        assert end_previous <= start < end <= total
        end_previous = end
        duration = float(subprocess.check_output([
            "ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", str(p)
        ], text=True, timeout=30).strip())
        speed = max(1.0, duration / (end - start))
        if speed > 1.35:
            raise SystemExit(f"旁白 {p.name} 過長：{duration:.2f}s，時槽 {end-start:.2f}s，請縮短文字或重新生成")
        fitted = folder / (segment["id"] + ".wav")
        run(*ff, "-i", str(p), "-af", f"atempo={speed:.8f},loudnorm=I=-16:TP=-2:LRA=7,apad,atrim=0:{end-start}",
            "-ar", "48000", "-ac", "1", "-c:a", "pcm_s16le", str(fitted))
        inputs += ["-i", str(fitted)]
        filters.append(f"[{i}:a]adelay={round(start*1000)}:all=1[v{i}]")
        entries.append({**segment, "mp3_sha256": sha(p), "raw_seconds": duration, "tempo": speed, "wav_sha256": sha(fitted)})
        subtitles.append(f"{i+1}\n{stamp(start)} --> {stamp(end)}\n{segment['text']}\n")
    n = len(entries)
    filters.append("".join(f"[v{i}]" for i in range(n)) + f"amix=inputs={n}:normalize=0,apad,atrim=0:{total}[voice]")
    run(*ff, *inputs, "-filter_complex_threads", "1", "-filter_complex", ";".join(filters),
        "-map", "[voice]", "-c:a", "pcm_s16le", str(folder / "narration.wav"))
    run(*ff, "-i", str(music), "-i", str(folder / "narration.wav"), "-filter_complex_threads", "1", "-filter_complex",
        "[1:a]asplit=2[key][voice];[0:a]volume=0.65[music];[music][key]sidechaincompress=threshold=0.02:ratio=8:attack=20:release=350[duck];[duck][voice]amix=inputs=2:normalize=0,alimiter=limit=0.95:level=false[mix]",
        "-map", "[mix]", "-ar", "48000", "-ac", "2", "-t", str(total), "-c:a", "pcm_s16le", str(folder / "mix.wav"))
    (folder / "narration.zh-TW.srt").write_text("\n".join(subtitles), encoding="utf-8")
    receipt = {"plan_sha256": sha(folder / "plan.json"), "music_sha256": sha(music), "segments": entries,
               "narration_sha256": sha(folder / "narration.wav"), "mix_sha256": sha(folder / "mix.wav"),
               "duration": total, "voice": plan["voice"], "service": "Microsoft Edge TTS", "generation_script": "agy CLI; reviewed by Codex"}
    (folder / "mix-receipt.json").write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
    print(f"旁白 {n} 段，{total}s，混音與字幕完成")


if __name__ == "__main__":
    main()
