"""Check and install verified GitHub Releases for per-user Overtone installs."""
import fcntl
import hashlib
import json
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tarfile
import tempfile
import urllib.error
import urllib.request

REPO = "devodrag/overtone"
LIMIT = 40 * 1024 * 1024
API = f"https://api.github.com/repos/{REPO}/releases/latest"
BINARIES = ("bin/overtone", "bin/overtone-ctl", "bin/overtone-background", "bin/overtone-route")
VERSION_FILE = "share/overtone/yandex/release-version"
PATHS = re.compile(r"(?:share/overtone/(?:yandex|vpn|adapter)/[a-zA-Z0-9._-]+|"
                   r"share/icons/hicolor/[0-9]+x[0-9]+/apps/io\.github\.devodrag\.Overtone\.png|"
                   r"share/doc/overtone/(?:docs/)?[a-zA-Z0-9._-]+|share/licenses/overtone/(?:LICENSE|FckCensor-MIT\.txt))")


def version(text):
    if not re.fullmatch(r"[0-9]{1,5}(?:\.[0-9]{1,5}){2}", text):
        raise ValueError("Неподдерживаемый номер версии.")
    return tuple(map(int, text.split(".")))


def latest(current):
    req = urllib.request.Request(API, headers={"User-Agent": "Overtone update",
                                               "Accept": "application/vnd.github+json"})
    try:
        with urllib.request.urlopen(req, timeout=8) as response:
            release = json.loads(response.read(512 * 1024))
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return None
        raise
    tag = release["tag_name"]
    number = tag.removeprefix("v")
    if release.get("draft") or release.get("prerelease") or version(number) <= version(current):
        return None
    prefix = f"overtone-{number}-"
    for asset in release["assets"]:
        name = asset.get("name", "")
        if not re.fullmatch(re.escape(prefix) + r"[0-9]+-x86_64\.pkg\.tar\.zst", name):
            continue
        digest = asset.get("digest") or ""
        url = f"https://github.com/{REPO}/releases/download/{tag}/{name}"
        if (asset.get("browser_download_url") == url and re.fullmatch(r"sha256:[0-9a-f]{64}", digest)
                and asset.get("state") == "uploaded" and 0 < asset.get("size", 0) <= LIMIT):
            return {"version": number, "url": url, "digest": digest[7:], "size": asset["size"]}
    raise ValueError("У релиза нет пакета Arch x86_64 с контрольной суммой GitHub.")


def fetch(asset, destination):
    req = urllib.request.Request(asset["url"], headers={"User-Agent": "Overtone update"})
    digest = hashlib.sha256()
    total = 0
    with urllib.request.urlopen(req, timeout=15) as response, destination.open("xb") as file:
        while block := response.read(256 * 1024):
            total += len(block)
            if total > asset["size"]:
                raise ValueError("Размер обновления отличается от размера в GitHub.")
            digest.update(block)
            file.write(block)
    if total != asset["size"] or digest.hexdigest() != asset["digest"]:
        raise ValueError("Проверка SHA-256 обновления не прошла.")


def unpack(package, root, expected):
    if not shutil.which("zstd"):
        raise ValueError("Для распаковки обновления нужен zstd.")
    files = {}
    total = 0
    with subprocess.Popen(["zstd", "-dc", package], stdout=subprocess.PIPE) as decoder:
        try:
            with tarfile.open(fileobj=decoder.stdout, mode="r|") as archive:
                for member in archive:
                    name = member.name.removeprefix("./").removeprefix("usr/")
                    if not member.isfile() or (name not in BINARIES and not PATHS.fullmatch(name)):
                        continue
                    total += member.size
                    if name in files or ".." in Path(name).parts or member.size > LIMIT or total > LIMIT:
                        raise ValueError("Некорректное содержимое пакета Overtone.")
                    destination = root / name
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    with destination.open("xb") as file:
                        shutil.copyfileobj(archive.extractfile(member), file)
                    destination.chmod(0o755 if name in BINARIES else 0o644)
                    files[name] = destination
            decoder.stdout.read()
            if decoder.wait(timeout=5):
                raise ValueError("Пакет обновления повреждён.")
        except BaseException:
            decoder.kill()
            raise
    if any(name not in files for name in (*BINARIES, VERSION_FILE)):
        raise ValueError("В пакете отсутствуют файлы Overtone.")
    if files[VERSION_FILE].read_text().strip() != expected:
        raise ValueError("Версия внутри пакета не совпадает с релизом.")
    return files


def install(asset, prefix):
    if platform.machine() != "x86_64":
        raise ValueError("Автоустановка поддерживается на Linux x86_64.")
    updates = prefix / "share/overtone"
    with (updates / ".update.lock").open("ab") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        current = (prefix / VERSION_FILE).read_text().strip()
        if version(current) >= version(asset["version"]):
            return
        with tempfile.TemporaryDirectory(prefix=".update-", dir=prefix) as workspace:
            folder = Path(workspace)
            package = folder / "release.pkg.tar.zst"
            fetch(asset, package)
            files = unpack(package, folder / "files", asset["version"])
            restored = []
            try:
                for name in sorted(files, key=lambda name: (name == VERSION_FILE, name in BINARIES, name)):
                    target = prefix / name
                    original = folder / "backup" / name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    if target.exists():
                        original.parent.mkdir(parents=True, exist_ok=True)
                        shutil.copy2(target, original)
                    files[name].replace(target)
                    restored.append((target, original))
            except BaseException:
                for target, backup in reversed(restored):
                    if backup.exists():
                        backup.replace(target)
                    else:
                        target.unlink(missing_ok=True)
                raise
