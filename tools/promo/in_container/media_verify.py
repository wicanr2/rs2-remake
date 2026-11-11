#!/usr/bin/env python3
"""對完成 MP4 做可重跑的視訊、音訊與旁白收據驗證。只在 Docker 執行。"""
import array
import hashlib
import json
import math
from pathlib import Path
import re
import subprocess as sp
import sys


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    movie, voice, out = map(Path, sys.argv[1:])
    probe = json.loads(sp.check_output(["ffprobe", "-v", "error", "-show_format", "-show_streams", "-of", "json", str(movie)], timeout=30))
    (out / "ffprobe.json").write_text(json.dumps(probe, indent=2) + "\n")
    v = next(s for s in probe["streams"] if s["codec_type"] == "video")
    a = next(s for s in probe["streams"] if s["codec_type"] == "audio")
    s = next(s for s in probe["streams"] if s["codec_type"] == "subtitle")
    assert (v["codec_name"], v["width"], v["height"], v["r_frame_rate"]) == ("h264", 1920, 1080, "60/1")
    assert (a["codec_name"], int(a["sample_rate"]), a["channels"]) == ("aac", 48000, 2)
    assert s["codec_name"] == "mov_text"
    assert abs(float(v["duration"]) - 80) < .05 and int(v["nb_frames"]) == 4800
    assert abs(float(a["duration"]) - 80) < .10
    pcm = sp.check_output(["ffmpeg", "-nostdin", "-v", "error", "-i", str(movie), "-vn", "-ac", "2", "-ar", "48000", "-f", "s16le", "-"], timeout=90)
    samples = array.array("h", pcm)
    peak = max(map(abs, samples))
    rms = math.sqrt(sum(x*x for x in samples) / len(samples))
    assert 200 < peak < 32767 and rms > 100, "混音靜音、過小或削波"
    scan = sp.run(["ffmpeg", "-nostdin", "-hide_banner", "-threads", "2", "-i", str(movie), "-an",
                   "-vf", "blackdetect=d=0.2:pix_th=0.04,freezedetect=n=0.003:d=0.6", "-f", "null", "-"],
                  stdout=sp.DEVNULL, stderr=sp.PIPE, text=True, timeout=240, check=True).stderr
    (out / "black-freeze.log").write_text(scan)
    blacks = [(float(x), float(y)) for x,y in re.findall(r"black_start:([\d.]+) black_end:([\d.]+)", scan)]
    assert all(start < .5 or end > 79.2 for start,end in blacks), ("片中全黑", blacks)
    freezes = []
    begin = None
    for kind, value in re.findall(r"freeze_(start|end): ([\d.]+)", scan):
        if kind == "start":
            begin = float(value)
        elif begin is not None:
            freezes.append((begin, float(value)))
            begin = None
    if begin is not None:
        freezes.append((begin, 80.0))
    allowed = [(0,14), (30,36), (72,80)]
    unexpected = [(x,y) for x,y in freezes if not any(x >= l-.15 and y <= r+.15 for l,r in allowed)]
    assert not unexpected, ("遊玩段可能凍結，需檢查", unexpected)
    mix = json.loads((voice / "mix-receipt.json").read_text())
    generated = json.loads((voice / "receipt.json").read_text())
    assert mix["plan_sha256"] == generated["plan_sha256"] == sha(voice / "plan.json")
    voice_levels = []
    for segment in generated["segments"]:
        assert segment["status"] in ("success", "skipped_existing")
        assert sha(voice / (segment["id"] + ".mp3")) == segment["sha256"]
        data = sp.check_output(["ffmpeg", "-nostdin", "-v", "error", "-i", str(voice / (segment["id"] + ".mp3")),
                                "-ac", "1", "-ar", "24000", "-f", "s16le", "-"], timeout=20)
        chunk = array.array("h", data)
        level = math.sqrt(sum(x*x for x in chunk) / max(1,len(chunk)))
        assert len(chunk) > 24000 and level > 100, (segment["id"], "旁白靜音或過短")
        voice_levels.append({"id": segment["id"], "rms_dbfs": 20*math.log10(level/32768)})
    assert len(generated["segments"]) == 11
    assert sha(voice / "narration.wav") == mix["narration_sha256"]
    assert sha(voice / "mix.wav") == mix["mix_sha256"]
    result = {"status": "passed", "file": movie.name, "sha256": sha(movie), "bytes": movie.stat().st_size,
              "duration_seconds": float(v["duration"]), "video_frames": int(v["nb_frames"]), "fps": 60,
              "audio_peak_dbfs": 20*math.log10(peak/32768), "audio_rms_dbfs": 20*math.log10(rms/32768),
              "black_intervals": blacks, "freeze_intervals": freezes, "unexpected_freezes": unexpected,
              "narration_segments": 11, "narration_service": generated["actual_service"],
              "individual_speech_levels": voice_levels,
              "voice": mix["voice"], "speech_source": "agy CLI generated script, reviewed by Codex; Edge TTS synthesis",
              "listening_review": "technical signal checks only; user listening preview supplied"}
    (out / "media-verification.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(result, ensure_ascii=False))


if __name__ == "__main__":
    main()
