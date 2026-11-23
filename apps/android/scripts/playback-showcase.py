#!/usr/bin/env python3
"""Validate current instrumentation captures before promotion."""
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import struct
import sys
import zlib


def validate(document, slugs):
    assert document["schema_version"] == 1 and document["journey"] == "playback"
    assert document["backend_instance_id"] == os.environ["SCREENROOM_BACKEND_INSTANCE_ID"]
    assert document["item"] == {
        "id": os.environ["SCREENROOM_SHOWCASE_ITEM_ID"],
        "title": os.environ["SCREENROOM_SHOWCASE_TITLE"],
        "year": int(os.environ["SCREENROOM_SHOWCASE_YEAR"]),
    }
    assert [p["slug"] for p in document["platforms"]] == slugs
    images = []
    for platform in document["platforms"]:
        assert platform["runtime"] and platform["outcome"] in ("observed", "blocked")
        assert platform["states"]
        ids = set()
        for state in platform["states"]:
            key = state["id"]
            assert key and all(c in "abcdefghijklmnopqrstuvwxyz0123456789-" for c in key)
            assert key not in ids
            ids.add(key)
            assert state["label"] and state["viewport"] and state["observation"]
            assert state["status"] in ("captured", "unavailable", "blocked", "not-observed")
            if state["status"] == "captured":
                image = state["image"]
                parts = PurePosixPath(image).parts
                assert len(parts) == 2 and parts[0] == platform["slug"]
                assert parts[1] == key + ".png" and image == "/".join(parts)
                assert image not in images
                images.append(image)
            else:
                assert state["reason"] and "image" not in state
        required = {"playing", "paused", "idle", "seek-before", "seek-after", "volume-0",
                    "volume-50", "volume-100", "mute", "tracks", "subtitles-off",
                    "settings", "fullscreen", "loading", "error", "stop", "cleanup"}
        assert required <= ids
        if platform["outcome"] == "blocked":
            assert any(s["id"] == "error" and s["status"] == "captured" for s in platform["states"])
    return images


def png(path):
    assert not path.is_symlink()
    data = path.read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    offset, pixels, ended = 8, bytearray(), False
    width = height = 0
    while offset < len(data):
        size = struct.unpack(">I", data[offset:offset + 4])[0]
        kind = data[offset + 4:offset + 8]
        body = data[offset + 8:offset + 8 + size]
        crc = struct.unpack(">I", data[offset + 8 + size:offset + 12 + size])[0]
        assert zlib.crc32(kind + body) & 0xffffffff == crc
        if kind == b"IHDR":
            width, height = struct.unpack(">II", body[:8])
            assert width > 0 and height > 0
        if kind == b"IDAT":
            pixels.extend(body)
        offset += size + 12
        if kind == b"IEND":
            ended = True
            break
    assert ended and offset == len(data) and width and height and zlib.decompress(pixels)
    return f"{width}x{height}"


def main():
    command, directory, *slugs = sys.argv[1:]
    directory = Path(directory)
    if command == "paths":
        document = json.loads((directory / "manifest.json").read_text())
        print("\n".join(validate(document, slugs)))
    elif command == "promote":
        target = directory.parent / "playback"
        merged = None
        for slug in slugs:
            document = json.loads((directory / f"{slug}.json").read_text())
            for image in validate(document, [slug]):
                dimensions = png(directory / image)
                state = next(s for s in document["platforms"][0]["states"] if s.get("image") == image)
                assert state["viewport"] == dimensions
            if merged is None:
                merged = document
            else:
                merged["platforms"].extend(document["platforms"])
        validate(merged, slugs)
        for slug in slugs:
            (directory / f"{slug}.json").unlink()
        (directory / "manifest.json").write_text(json.dumps(merged, indent=2) + "\n")
        backup = directory.parent / (directory.name + "-previous")
        assert not backup.exists()
        if target.exists():
            target.rename(backup)
        try:
            directory.rename(target)
        except BaseException:
            if backup.exists():
                backup.rename(target)
            raise
        if backup.exists():
            shutil.rmtree(backup)
    else:
        raise ValueError(command)


if __name__ == "__main__":
    main()
