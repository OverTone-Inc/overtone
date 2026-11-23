#!/usr/bin/env python3
"""Package the Rust client, corresponding GPL source and launcher manifest."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tomllib

from prepare import PREPARED_FILES, ROOT, REVISION

VERSION = "1.1.4-ov4"
PREFIX = f"overtone-yamusic-rust-{VERSION}"


def package(checkout, binary, port, github_release):
    if binary.open("rb").read(5) != b"\x7fELF\x02":
        raise ValueError("Expected a built Linux client.")
    if subprocess.check_output(["git", "-C", checkout, "rev-parse", "HEAD"], text=True).strip() != REVISION:
        raise ValueError("Expected the pinned GPL upstream revision.")
    directory = ROOT / "release"
    directory.mkdir(exist_ok=True)
    target = directory / f"{PREFIX}-x86_64.gz"
    with binary.open("rb") as content, target.open("wb") as output, gzip.GzipFile(
            fileobj=output, mode="wb", filename="", mtime=0, compresslevel=6) as compressed:
        shutil.copyfileobj(content, compressed)
    tracked = subprocess.check_output(["git", "-C", checkout, "ls-files", "-z"]).decode().split("\0")
    with tarfile.open(directory / f"{PREFIX}-source.tar.gz", "w:gz") as archive:
        for name in sorted((set(tracked) - {""}) | set(PREPARED_FILES.values())):
            archive.add(checkout / name, arcname=f"{PREFIX}/{name}")
    with target.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    if github_release:
        launcher_version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
        base_url = f"https://github.com/OverTone-Inc/overtone/releases/download/v{launcher_version}"
    else:
        base_url = f"http://127.0.0.1:{port}/files"
    manifest = {"version": VERSION, "size": target.stat().st_size, "sha256": digest,
                "url": f"{base_url}/{target.name}"}
    (ROOT / "integration/yandex/rust-release.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(target)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("checkout", type=Path)
    parser.add_argument("binary", type=Path)
    origin = parser.add_mutually_exclusive_group()
    origin.add_argument("--port", type=int, default=8765)
    origin.add_argument("--github-release", action="store_true", help="Use GitHub Releases as the download origin.")
    args = parser.parse_args()
    package(args.checkout, args.binary, args.port, args.github_release)
