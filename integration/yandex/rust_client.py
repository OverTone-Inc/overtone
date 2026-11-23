"""Install the separate Overtone Rust client from the preview build manifest."""
import fcntl
import gzip
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tempfile
import time
from urllib.parse import urlsplit
from urllib.request import Request, urlopen

APP_ID = "overtone-yamusic-rust"
LIMIT = 95 * 1024 * 1024
PAYLOAD = Path(__file__).resolve().parent


def xdg(key, fallback):
    value = os.environ.get(key)
    return Path(value) if value and Path(value).is_absolute() else Path.home() / fallback


def manifest():
    try:
        release = json.loads((PAYLOAD / "rust-release.json").read_text())
    except (OSError, ValueError):
        return None
    url = urlsplit(release["url"])
    if not ((url.scheme == "http" and url.hostname == "127.0.0.1") or
            (url.scheme == "https" and url.netloc == "github.com" and
             url.path.startswith("/OverTone-Inc/overtone/releases/download/"))):
        raise ValueError("Адрес пакета Rust-клиента не поддерживается.")
    if (url.username or url.password or not re.fullmatch(r"[a-f0-9]{64}", release["sha256"])
            or type(release["size"]) is not int or not 0 < release["size"] <= LIMIT
            or not re.fullmatch(r"[a-z0-9.\-]+", release["version"])):
        raise ValueError("Некорректное описание пакета Rust-клиента.")
    return release


def download(release, target):
    digest = hashlib.sha256()
    length = 0
    request = Request(release["url"], headers={"User-Agent": "Overtone Rust installer"})
    with urlopen(request, timeout=30) as response, target.open("xb") as file:
        while chunk := response.read(65536):
            length += len(chunk)
            if length > release["size"]:
                raise ValueError("Размер полученного пакета не совпадает.")
            digest.update(chunk)
            file.write(chunk)
    if length != release["size"] or digest.hexdigest() != release["sha256"]:
        raise ValueError("Пакет не прошёл проверку SHA-256.")


def install(directory, executable, entry, release):
    if not release:
        raise ValueError("Пакет Rust-клиента пока недоступен для этой сборки Overtone.")
    with tempfile.TemporaryDirectory(dir=directory) as folder:
        stage = Path(folder)
        package = stage / "download.gz"
        download(release, package)
        candidate = stage / "client"
        with gzip.open(package, "rb") as stream, candidate.open("xb") as file:
            length = 0
            while chunk := stream.read(65536):
                length += len(chunk)
                if length > LIMIT:
                    raise ValueError("Распакованный клиент слишком большой.")
                file.write(chunk)
        with candidate.open("rb") as file:
            if file.read(5) != b"\x7fELF\x02":
                raise ValueError("В пакете отсутствует исполняемый файл Linux.")
        candidate.chmod(0o755)
        candidate.replace(executable)
        path = str(executable).replace("%", "%%").replace("\\", "\\\\")
        for character in ('"', "`", "$"):
            path = path.replace(character, "\\" + character)
        entry.parent.mkdir(parents=True, exist_ok=True)
        desktop = stage / "client.desktop"
        desktop.write_text(
            "[Desktop Entry]\nType=Application\nName=Яндекс Музыка (Rust)\n"
            f'Exec="{path}"\nIcon=io.github.devodrag.Overtone\n'
            "Terminal=false\nStartupNotify=true\nCategories=AudioVideo;Audio;Player;\n"
            f"StartupWMClass={APP_ID}\n"
        )
        with tempfile.NamedTemporaryFile(dir=entry.parent, delete=False) as stream:
            temporary = Path(stream.name)
        try:
            shutil.copyfile(desktop, temporary)
            temporary.chmod(0o644)
            temporary.replace(entry)
        finally:
            temporary.unlink(missing_ok=True)
        marker = stage / "version"
        marker.write_text(release["version"] + "\n")
        marker.replace(directory / "version")


def run(action):
    supported = platform.system() == "Linux" and platform.machine() == "x86_64"
    release = manifest() if supported else None
    directory = xdg("XDG_DATA_HOME", ".local/share") / "overtone" / "rust"
    marker = directory / "version"
    executable = directory / APP_ID
    entry = xdg("XDG_DATA_HOME", ".local/share") / "applications" / f"{APP_ID}.desktop"
    if action == "status":
        installed = marker.read_text().strip() if marker.is_file() and os.access(executable, os.X_OK) else None
        print(json.dumps({"supported": supported, "installed": installed,
                          "available": release and release["version"]}))
        return
    if not supported:
        raise ValueError("Сборка Rust-клиента доступна для Linux x86_64.")
    if action == "launch":
        if not executable.is_file():
            raise ValueError("Сначала установите Яндекс Музыку (Rust).")
        logs = xdg("XDG_STATE_HOME", ".local/state") / "overtone"
        logs.mkdir(parents=True, exist_ok=True)
        with (logs / "rust-client.log").open("ab") as log:
            process = subprocess.Popen([executable], stdin=subprocess.DEVNULL, stdout=log,
                                       stderr=log, start_new_session=True)
        time.sleep(0.4)
        if process.poll() not in (None, 0):
            raise ValueError(f"Rust-клиент завершился при запуске. Журнал: {logs / 'rust-client.log'}")
        return
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    with (directory / "install.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if action == "install":
            install(directory, executable, entry, release)
        elif action == "remove":
            for path in (marker, entry, executable):
                path.unlink(missing_ok=True)
