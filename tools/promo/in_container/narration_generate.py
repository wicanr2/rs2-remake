#!/usr/bin/env python3
"""由 agy CLI 的 Gemini 3.8 Flash Medium 產生，經 Codex 審查後接入。

審查修改：驗證段落 ID、既存音檔須比對收據、原子寫入音檔，避免把失敗下載當成成功。
真正的語音服務是 Microsoft Edge TTS，並非 Gemini 原生語音。
"""
import asyncio
import hashlib
import importlib.metadata
import json
from pathlib import Path
import re
import sys

import edge_tts


def compute_sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()


async def main() -> None:
    out_dir = Path("/out")
    stat_info = out_dir.stat()
    if stat_info.st_uid != 1000 or stat_info.st_gid != 1000:
        raise SystemExit("/out 必須屬於 1000:1000")
    plan_path = out_dir / "plan.json"
    plan_sha256 = compute_sha256(plan_path)
    plan = json.loads(plan_path.read_text(encoding="utf-8"))
    default_voice = plan["voice"]
    default_rate = plan["rate"]
    edge_version = importlib.metadata.version("edge_tts")
    receipt_path = out_dir / "receipt.json"
    previous = json.loads(receipt_path.read_text()) if receipt_path.exists() else {}
    reusable = {e["id"]: e for e in previous.get("segments", [])} if previous.get("plan_sha256") == plan_sha256 else {}
    results = []
    any_failed = False
    seen = set()
    assert plan["segments"], "旁白不能是空的"
    for seg in plan["segments"]:
        seg_id = str(seg["id"])
        assert re.fullmatch(r"[0-9]{2}", seg_id) and seg_id not in seen
        seen.add(seg_id)
        text = seg["text"]
        voice = seg.get("voice", default_voice)
        rate = seg.get("rate", default_rate)
        mp3_path = out_dir / f"{seg_id}.mp3"
        tmp_path = out_dir / f"{seg_id}.partial.mp3"
        old = reusable.get(seg_id, {})
        if (old.get("status") in ("success", "skipped_existing") and mp3_path.is_file()
                and mp3_path.stat().st_size > 0 and old.get("sha256") == compute_sha256(mp3_path)
                and all(old.get(k) == v for k, v in (("text", text), ("voice", voice), ("rate", rate)))):
            results.append({**old, "status": "skipped_existing"})
            continue
        success = False
        last_error = ""
        for attempt in range(2):
            try:
                communicator = edge_tts.Communicate(text, voice, rate=rate)
                await asyncio.wait_for(communicator.save(str(tmp_path)), timeout=60.0)
                if not tmp_path.is_file() or tmp_path.stat().st_size == 0:
                    raise RuntimeError("沒有產生非空音檔")
                tmp_path.replace(mp3_path)
                results.append({"id": seg_id, "status": "success", "text": text,
                                "voice": voice, "rate": rate, "sha256": compute_sha256(mp3_path),
                                "bytes": mp3_path.stat().st_size})
                print(f"{seg_id} 成功", flush=True)
                success = True
                break
            except Exception as e:
                last_error = f"{type(e).__name__}: {str(e)}"
                tmp_path.unlink(missing_ok=True)
                if attempt == 0:
                    await asyncio.sleep(1.0)
        if not success:
            any_failed = True
            results.append({"id": seg_id, "status": "failed", "text": text,
                            "voice": voice, "rate": rate, "error": last_error})
    receipt = {"actual_service": "Microsoft Edge TTS", "generation_script": "authored with agy CLI; reviewed by Codex",
               "agy_model": "gemini-3.8-flash-medium", "edge_tts_version": edge_version,
               "script_sha256": compute_sha256(Path(__file__)), "plan_sha256": plan_sha256, "segments": results}
    receipt_path.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    if any_failed:
        sys.exit("部分旁白生成失敗，見 receipt.json")


if __name__ == "__main__":
    asyncio.run(main())
