#!/usr/bin/env python3
"""在版本交付根目錄建立四平台、推廣片與旁白的統一雜湊索引。"""
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys


def main():
    root, version, work = Path(sys.argv[1]), sys.argv[2], Path(sys.argv[3])
    assert re.fullmatch(r"v\.\d+\.\d+\.\d+-\d{8}", version)
    assert root.name == version
    checks = {"linux": "linux/verify.log", "windows_wine": "windows/verify.log"}
    for name, path in checks.items():
        text = (root / "smoke" / path).read_text()
        assert "== 結果：FAIL 0 項" in text, name
    assert "== 結果：FAIL 0 項" in (root / "smoke/static.log").read_text()
    android_log = (root / "smoke/android/verify.log").read_text()
    assert "ANR 紀錄：0 筆" in android_log and "完成" in android_log and version in android_log
    static = json.loads((root / "smoke/android-static.json").read_text())
    assert static["payload_files_verified"] == 3442 and len(static["posters_verified"]) == 4
    assert set(static["architectures"]) == {"arm64-v8a", "x86_64"}
    media = json.loads((root / "promo/media-verification.json").read_text())
    assert media["status"] == "passed"
    promo = root / "promo"
    shutil.copyfile(work / "narration/narration.wav", promo / f"RS-2-remake-narration-{version}.wav")
    subprocess.run(["ffmpeg", "-nostdin", "-hide_banner", "-loglevel", "error", "-y",
                    "-i", str(work / "narration/narration.wav"), "-t", "14", "-c:a", "pcm_s16le",
                    str(promo / "narration-preview.wav")], check=True, timeout=30)
    shutil.copyfile(work / "narration/narration.zh-TW.srt", promo / f"RS-2-remake-promo-{version}.zh-TW.srt")
    for source, target in [("receipt.json", "narration-generation.json"), ("mix-receipt.json", "narration-mix.json"), ("plan.json", "narration-plan.json")]:
        shutil.copyfile(work / "narration" / source, promo / target)
    shutil.copyfile(work / "review/montage.png", promo / "contact-sheet.png")
    (promo / "frames").mkdir(exist_ok=True)
    for p in (work / "review").glob("*.png"):
        if p.name != "montage.png":
            shutil.copyfile(p, promo / "frames" / p.name)
    rights = {"distribution": "local only", "game_art_and_music": "original RS-2 assets and derived HD artwork; no public redistribution authorization",
              "music_source": "original A.MIX and B.MIX register sequences rendered with the established Nuked OPL3 core; see docs/spec/003-audio.md",
              "narration_script": "tools/promo/narration.json; agy CLI generated the synthesis script, reviewed by Codex",
              "speech_service": "Microsoft Edge TTS", "voice": "zh-TW-HsiaoChenNeural",
              "authorization": "user explicitly authorized sending these 11 narration texts to Microsoft Edge TTS on 2026-11-11",
              "input_music_sha256": json.loads((work / "narration/mix-receipt.json").read_text())["music_sha256"]}
    source_screenshot = root / "smoke/android/06-moved.png"
    rights["android_screenshot"] = {"path": source_screenshot.relative_to(root).as_posix(),
                                    "sha256": hashlib.sha256(source_screenshot.read_bytes()).hexdigest()}
    (promo / "rights.json").write_text(json.dumps(rights, ensure_ascii=False, indent=2) + "\n")
    expected = {"linux-x86_64": "full-linux-x86_64.AppImage", "windows-x86_64": "full-windows-x86_64.zip",
                "macos-universal": "full-macos-universal.zip", "android-arm64-x86_64": "android-full.apk"}
    artifacts = []
    for platform, suffix in expected.items():
        files = list((root / "full-local").glob(f"RS-2-remake-{version}-{suffix}"))
        assert len(files) == 1, (platform, files)
        artifacts.append((platform, files[0]))
    for p in sorted((root / "promo").iterdir()):
        if p.suffix in (".mp4", ".wav", ".srt"):
            artifacts.append(("promo", p))
    entries = [{"kind": kind, "path": p.relative_to(root).as_posix(), "bytes": p.stat().st_size,
                "sha256": hashlib.sha256(p.read_bytes()).hexdigest()} for kind,p in artifacts]
    manifest = {"version": version, "rights": "full-local only; contains original and derived artwork and original music; no redistribution",
                "artifacts": entries, "source": json.loads((root / "full-local/manifest.json").read_text())["source_sha256"],
                "verification": {"linux": "smoke/linux/verify.log", "windows_wine": "smoke/windows/verify.log",
                                 "android_static": "smoke/android-static.json", "android_emulator": "smoke/android/",
                                 "macos": "static only; not tested on a physical Mac", "media": "promo/media-verification.json"}}
    (root / "SHA256SUMS.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    print(f"{version}：{len(entries)} 項正式交付物已建立雜湊索引")


if __name__ == "__main__":
    main()
