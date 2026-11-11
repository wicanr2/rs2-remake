#!/usr/bin/env python3
"""直接檢查完成 APK 的架構、版本與資料包逐檔雜湊。"""
import hashlib
import io
import json
from pathlib import Path
import struct
import sys
import zipfile


def main():
    apk, catalog, version = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
    expected = json.loads(catalog.read_text())["files"]
    with zipfile.ZipFile(apk) as z:
        assert z.testzip() is None, "APK CRC 失敗"
        machines = {"arm64-v8a": 183, "x86_64": 62}
        architectures = sorted({n.split("/")[1] for n in z.namelist() if n.startswith("lib/") and n.endswith("/libmain.so")})
        assert architectures and set(architectures) <= set(machines), architectures
        for abi in architectures:
            machine = machines[abi]
            for name in ("libmain.so", "libSDL2.so"):
                b = z.read(f"lib/{abi}/{name}")
                assert b[:5] == b"\x7fELF\x02" and struct.unpack_from("<H", b, 18)[0] == machine
        manifest = z.read("AndroidManifest.xml")
        assert version.encode() in manifest or version.encode("utf-16le") in manifest, "APK 缺版本字串"
        for name in ("LICENSE", "THIRD-PARTY-NOTICES.txt", "SDL2-LICENSE.txt"):
            assert z.read(f"assets/{name}"), name
        p = io.BytesIO(z.read("assets/rs2-data.pak"))
        assert p.read(8) == b"RS2PAK1\n"
        count = struct.unpack("<I", p.read(4))[0]
        entries = []
        for _ in range(count):
            length = struct.unpack("<H", p.read(2))[0]
            name = p.read(length).decode("utf-8")
            size = struct.unpack("<Q", p.read(8))[0]
            digest = p.read(32).hex()
            entries.append((name, size, digest))
        assert len({e[0] for e in entries}) == count
        assert {e[0] for e in entries} == set(expected), "APK 資料檔清單不符"
        for name, size, digest in entries:
            data = p.read(size)
            assert len(data) == size and hashlib.sha256(data).hexdigest() == digest, name
            assert expected[name]["sha256"] == digest and expected[name]["bytes"] == size, name
        assert not p.read(1), "資料包尾端多出資料"
    posters = sorted(n for n in expected if n.startswith("hd/posters/"))
    assert len(posters) == 4
    print(json.dumps({"apk": apk.name, "sha256": hashlib.sha256(apk.read_bytes()).hexdigest(),
                      "version": version, "architectures": architectures,
                      "payload_files_verified": count, "posters_verified": posters,
                      "zip_crc": "passed", "package_version": "passed"}, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
