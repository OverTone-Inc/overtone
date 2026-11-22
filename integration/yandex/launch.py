"""Install Overtone into the local Yandex Music ASAR and restore its original."""
import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
import re
import stat
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import time

import plugin_config
import update

PAYLOAD = Path(__file__).resolve().parent
VERSION = "5.109.1"
ASAR_LIMIT = 8 * 1024 * 1024
MARKER = "overtone-install.json"


class Incompatible(ValueError):
    pass


class Archive:
    def __init__(self, path):
        self.path = path
        with path.open("rb") as stream:
            magic, header_size, pickle_size, json_size = struct.unpack("<4I", stream.read(16))
            if (magic != 4 or header_size != pickle_size + 4
                    or not 0 < json_size <= min(ASAR_LIMIT, pickle_size - 4)):
                raise Incompatible("Некорректный заголовок app.asar.")
            self.tree = json.loads(stream.read(json_size))
            self.base = 8 + header_size
        self.size = path.stat().st_size - self.base
        if self.size < 0 or not isinstance(self.tree.get("files"), dict):
            raise Incompatible("Некорректный архив app.asar.")

    def node(self, name):
        current = self.tree
        for part in name.split("/"):
            current = current["files"][part]
        return current

    def read(self, name):
        node = self.node(name)
        offset, size = int(node["offset"]), node["size"]
        if not 0 <= offset <= offset + size <= self.size:
            raise Incompatible("Выход за границы app.asar.")
        with self.path.open("rb") as stream:
            stream.seek(self.base + offset)
            return stream.read(size)

    def write(self, path, edited):
        position = self.size
        for name, content in edited.items():
            parts = name.split("/")
            directory = self.tree if len(parts) == 1 else self.node("/".join(parts[:-1]))
            block_size = 4 * 1024 * 1024
            directory["files"][parts[-1]] = {
                "size": len(content), "offset": str(position),
                "integrity": {
                    "algorithm": "SHA256", "hash": hashlib.sha256(content).hexdigest(),
                    "blockSize": block_size,
                    "blocks": [hashlib.sha256(content[i:i+block_size]).hexdigest()
                               for i in range(0, len(content), block_size)],
                },
            }
            position += len(content)
        header = json.dumps(self.tree, ensure_ascii=False, separators=(",", ":")).encode()
        padding = bytes(-len(header) % 4)
        with path.open("xb") as stream, self.path.open("rb") as source:
            stream.write(struct.pack("<4I", 4, len(header)+len(padding)+8,
                                     len(header)+len(padding)+4, len(header)))
            stream.write(header)
            stream.write(padding)
            source.seek(self.base)
            shutil.copyfileobj(source, stream, 1024 * 1024)
            for content in edited.values():
                stream.write(content)


def replace(source, old, new):
    if source.count(old) != 1:
        raise Incompatible("Сборка Яндекс Музыки отличается от поддерживаемой.")
    return source.replace(old, new, 1)


def prepare(archive, runtime_version):
    package = json.loads(archive.read("package.json"))
    if (package.get("name"), package.get("main"), package.get("version")) != (
            "YandexMusic", "./index.js", VERSION):
        raise Incompatible(f"Поддерживается исходная Яндекс Музыка {VERSION} для Linux.")
    root = "app/_next/static/chunks/"
    runtime = archive.node(root[:-1])["files"]

    def chunk(prefix):
        paths = [name for name in runtime if name.startswith(prefix) and name.endswith(".js")]
        if len(paths) != 1:
            raise Incompatible("Не найден ожидаемый компонент Яндекс Музыки.")
        return root + paths[0]

    edits = {}
    path = chunk("3642.")
    data = archive.read(path).decode()
    data = replace(data,
        'async getLyrics(t,e){return(await this.httpClient.get("tracks/".concat(t.trackId,"/lyrics"),',
        'async getLyrics(t,e){const original=()=>this.overtoneOriginalLyrics(t,e);'
        'return window.OvertoneRuntime?.lyrics(t,original,()=>this.getTracksMeta({trackIds:[t.trackId]},e))??original()}'
        'async overtoneOriginalLyrics(t,e){return(await this.httpClient.get("tracks/".concat(t.trackId,"/lyrics"),')
    data = replace(data, 'async getLyricsText(t,e){return(await',
        'async getLyricsText(t,e){const local=window.OvertoneRuntime?.text(t);if(local!==undefined)return local;return(await')
    data = replace(data, 'async getTracksMeta(t,e){return(await',
        'async getTracksMeta(t,e){window.OvertoneRuntime?.tracks(this);const tracks=await this.overtoneOriginalTracks(t,e);'
        'return window.OvertoneRuntime?.metadata(tracks)??tracks}'
        'async overtoneOriginalTracks(t,e){return(await')
    edits[path] = data.encode()

    path = chunk("3265-")
    data = archive.read(path).decode()
    data = replace(data,
        'let{entity:t,disableCache:a=!1,codecs:r,quality:i,transports:s}=e;if(this.variables.isMediaSourcePreloadEnabled)',
        'let{entity:t,disableCache:a=!1,codecs:r,quality:i,transports:s}=e;'
        'const original=t.data.meta.id,replacement=window.OvertoneRuntime?.replacement(original);'
        'if(replacement&&String(original)!==replacement){'
        'const read=(trackId)=>this.getFileInfoFromResource({trackId,quality:i,codecs:r,transports:s});'
        'return read(replacement).then(result=>result?.downloadInfo?.urls?.length?result:read(original),()=>read(original));}'
        'if(this.variables.isMediaSourcePreloadEnabled)')
    data = replace(data, 'handlePlayerEvents(e){let t,a;e.state.queueState.currentEntity.onChange',
        'handlePlayerEvents(e){window.OvertoneRuntime?.player(e,{playing:Y.MT.PLAYING,paused:Y.MT.PAUSED,'
        'buffering:Y.MT.BUFFERING,seek:Y.Iu.SET_PROGRESS});let t,a;e.state.queueState.currentEntity.onChange')
    data = replace(data, 'onYnisonStateUpdated(e){this.variables.shouldApplyState&&this.applyYnisonDiff(e)}'
        'applyYnisonDiff(e){var t,a,r,i,s,n;',
        'onYnisonStateUpdated(e){window.OvertoneRuntime?.ynison(e.state);'
        'this.variables.shouldApplyState&&this.applyYnisonDiff(e)}applyYnisonDiff(e){var t,a,r,i,s,n;')
    edits[path] = data.encode()

    path = chunk("3773-")
    data = archive.read(path).decode()
    data = replace(data, 'return!!e.hasSyncLyrics&&!t.isOfflineModeEnabled',
        'return(!!e.hasSyncLyrics||(this.isTrackMusic&&window.OvertoneRuntime?.externalLyrics()))&&!t.isOfflineModeEnabled')
    data = replace(data, 'return!!e.hasLyrics&&!t.isOfflineModeEnabled',
        'return(!!e.hasLyrics||(this.isTrackMusic&&window.OvertoneRuntime?.externalLyrics()))&&!t.isOfflineModeEnabled')
    edits[path] = data.encode()

    path = root + "6035-c39d078770d5bf6a.js"
    data = archive.read(path).decode()
    data = replace(data, '(null==(a=m.meta)?void 0:a.isLyricsAvailable);',
        '((null==(a=m.meta)?void 0:a.isLyricsAvailable)||window.OvertoneRuntime?.externalLyrics());')
    data = replace(data, '(null==(s=p.meta)?void 0:s.isLyricsAvailable)&&(0,c.jsx)(B,',
        'p.meta&&((null==(s=p.meta)?void 0:s.isLyricsAvailable)||window.OvertoneRuntime?.externalLyrics())&&(0,c.jsx)(B,')
    edits[path] = data.encode()

    path = root + "6346-1db4a6e2284dd018.js"
    data = archive.read(path).decode()
    data = replace(data, 'f&&f.isLyricsAvailable&&u.modal.isOpened',
        'f&&(f.isLyricsAvailable||window.OvertoneRuntime?.externalLyrics())&&u.modal.isOpened')
    edits[path] = data.encode()

    path = chunk("8512-")
    data = archive.read(path).decode()
    data = replace(data, 'children:eA}),(0,O.jsx)(ed.aQ,{fallback:',
        'children:eA}),'
        '(0,O.jsx)("div",{className:"overtone-nav-slot",ref:window.OvertoneRuntime?.navigation}),'
        '(0,O.jsx)(ed.aQ,{fallback:')
    edits[path] = data.encode()

    # Keep Yandex Music's main/preload paths and Electron security preferences.
    main = archive.read("index.js").decode()
    directive = "'use strict';"
    if not main.startswith(directive):
        raise Incompatible("Не найден ожидаемый вход в приложение.")
    main = replace(main, "  await loadURL(window);",
        '  await require("./overtone-main.cjs").configureNetwork(window.webContents.session);\n'
        '  await loadURL(window);')
    edits["index.js"] = main.replace(directive,
        directive + '\nrequire("./overtone-main.cjs");\n', 1).encode()
    edits["preload.js"] = (archive.read("preload.js") + b'\n;(() => {\n'
        + (PAYLOAD/"preload.js").read_bytes()
        + b'\nbridge.executeInMainWorld({func: function () {\n'
        + f'const overtoneVersion = {json.dumps(runtime_version)};\n'.encode()
        + (PAYLOAD/"renderer.js").read_bytes()
        + b'\n}});\n})();\n')
    discord_id = json.loads((PAYLOAD/"discord-application.json").read_text())["applicationId"]
    if discord_id is not None and (not isinstance(discord_id, str) or not re.fullmatch(r"[0-9]{16,22}", discord_id)):
        raise Incompatible("В сборке указан некорректный Discord Application ID.")
    route = (PAYLOAD.parents[2] / "bin/overtone-route" if (PAYLOAD / "release-version").is_file()
             else PAYLOAD.parents[1] / "target/release/overtone-route")
    definitions = (f'"use strict";\nconst version = {json.dumps(runtime_version)};\n'
                   f'const applicationId = {json.dumps(discord_id)};\n'
                   f'const routeEntry = {json.dumps(str(route))};\n')
    edits["overtone-main.cjs"] = definitions.encode() + (PAYLOAD/"main.cjs").read_bytes()
    edits["overtone-discord.cjs"] = (PAYLOAD/"discord.cjs").read_bytes()
    edits["overtone-plugins.cjs"] = (PAYLOAD/"plugins.cjs").read_bytes()
    return edits


def find_installation():
    configured = os.environ.get("OVERTONE_YANDEX_EXECUTABLE")
    executables = [configured] if configured else [
        shutil.which("yandex-music"), shutil.which("yandexmusic"),
        "/opt/yandex-music/yandexmusic", "/opt/YandexMusic/yandexmusic",
    ]
    for candidate in executables:
        if candidate:
            path = Path(candidate).resolve()
            if path.is_file() and os.access(path, os.X_OK) and (path.parent/"resources").is_dir():
                return path
    raise Incompatible("Установите Яндекс Музыку для Linux. Лаунчер ищет её в PATH и /opt.")


def xdg(name, fallback):
    value = os.environ.get(name)
    return Path(value) if value and Path(value).is_absolute() else Path.home()/fallback


def checksum(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def file_id(path):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode):
        raise Incompatible(f"Ожидался обычный файл. {path}")
    return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns)


def manifest(path):
    archive = Archive(path)
    if MARKER not in archive.tree["files"]:
        return None
    marker = json.loads(archive.read(MARKER))
    if (not isinstance(marker, dict) or marker.get("product") != "Overtone"
            or marker.get("music") != VERSION
            or not re.fullmatch(r"[0-9a-f]{64}", marker.get("original", ""))
            or not re.fullmatch(r"[0-9a-f]{64}", marker.get("build", ""))):
        raise Incompatible("Повреждена отметка установки Overtone в app.asar.")
    return marker


def backup_path(active, original):
    return active.with_name(f"app.overtone-original.{original[:16]}.asar")


def verified_backup(active, marker):
    backup = backup_path(active, marker["original"])
    try:
        file_id(backup)
        if checksum(backup) == marker["original"]:
            return backup
    except FileNotFoundError:
        pass
    raise Incompatible(f"Отсутствует подходящая резервная копия. {backup}")


def running_clients(executable):
    # Copies left by earlier Overtone releases share the same profile.
    cache = xdg("XDG_CACHE_HOME", ".cache")/"overtone/yandex"
    paths = set()
    for process in Path("/proc").iterdir():
        if not process.name.isdecimal():
            continue
        try:
            if process.stat().st_uid != os.getuid():
                continue
            running = (process/"exe").resolve(strict=True)
            if running == executable or (running.name == executable.name and running.is_relative_to(cache)):
                paths.add(running)
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
    return paths


def require_closed(executable):
    if running_clients(executable):
        raise Incompatible("Полностью закройте Яндекс Музыку и повторите операцию.")


@contextmanager
def locked(directory):
    fd = os.open(directory/".overtone.lock", os.O_CREAT | os.O_NOFOLLOW | os.O_RDWR, 0o600)
    with os.fdopen(fd, "rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        yield


def inspect(active, runtime_version):
    file_id(active)
    marker = manifest(active)
    original = verified_backup(active, marker) if marker else active
    archive = Archive(original)
    edits = prepare(archive, runtime_version)
    digest = hashlib.sha256()
    for name, content in edits.items():
        digest.update(name.encode())
        digest.update(hashlib.sha256(content).digest())
    build = digest.hexdigest()
    if marker and marker["build"] == build:
        state = "installed"
    elif marker:
        state = "update"
    else:
        state = "ready"
    return state, marker, archive, edits, build


def commit(candidate, active, expected):
    with candidate.open("rb") as stream:
        os.fsync(stream.fileno())
    if file_id(active) != expected:
        raise Incompatible("app.asar изменился во время операции. Повторите проверку клиента.")
    os.replace(candidate, active)
    fd = os.open(active.parent, os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def install(executable, active, runtime_version):
    with locked(active.parent):
        expected = file_id(active)
        state, marker, archive, edits, build = inspect(active, runtime_version)
        if state == "installed":
            return state
        require_closed(executable)
        original = marker["original"] if marker else checksum(active)
        backup = backup_path(active, original)
        with tempfile.TemporaryDirectory(prefix=".overtone-", dir=active.parent) as directory:
            directory = Path(directory)
            if not marker:
                if not backup.exists():
                    copy = directory/"original.asar"
                    shutil.copy2(active, copy)
                    if checksum(copy) != original:
                        raise Incompatible("Исходный архив изменился во время резервного копирования.")
                    with copy.open("rb") as stream:
                        os.fsync(stream.fileno())
                    os.link(copy, backup, follow_symlinks=False)
                verified_backup(active, {"original": original})
                archive = Archive(backup)
            candidate = directory/"app.asar"
            edits[MARKER] = json.dumps({"product": "Overtone", "release": runtime_version,
                                        "music": VERSION, "original": original, "build": build}).encode()
            archive.write(candidate, edits)
            shutil.copymode(active, candidate)
            check = Archive(candidate)
            for name, content in edits.items():
                if check.read(name) != content:
                    raise Incompatible("Проверка собранного app.asar не прошла.")
            require_closed(executable)
            commit(candidate, active, expected)
    return "installed"


def restore(executable, active):
    with locked(active.parent):
        expected = file_id(active)
        marker = manifest(active)
        if not marker:
            return "ready"
        require_closed(executable)
        backup = verified_backup(active, marker)
        with tempfile.TemporaryDirectory(prefix=".overtone-", dir=active.parent) as directory:
            candidate = Path(directory)/"app.asar"
            shutil.copy2(backup, candidate)
            if checksum(candidate) != marker["original"]:
                raise Incompatible("Проверка исходного архива не прошла.")
            require_closed(executable)
            commit(candidate, active, expected)
    return "ready"


def launch(executable):
    existing = running_clients(executable)
    if existing - {executable}:
        raise Incompatible("Закройте копию Яндекс Музыки из предыдущей версии Overtone.")
    state = xdg("XDG_STATE_HOME", ".local/state")/"overtone"
    state.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="start-", dir=state) as directory:
        ready = Path(directory)/"ready"
        env = dict(os.environ, OVERTONE_READY_FILE=str(ready))
        with (state/"yandex.log").open("w", encoding="utf-8") as log:
            process = subprocess.Popen([executable], env=env, start_new_session=True,
                                       stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        for _ in range(160):
            if ready.exists():
                return
            if process.poll() is not None:
                if process.returncode == 0 and existing and executable in running_clients(executable):
                    return
                raise Incompatible(f"Яндекс Музыка закрылась во время запуска. Журнал в {state/'yandex.log'}.")
            time.sleep(0.1)
    raise Incompatible(f"Окно мода не подтвердило запуск. Журнал в {state/'yandex.log'}.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["status", "install", "restore", "launch",
                                           "plugins", "plugins-save", "catalog", "update-check", "update-install"])
    parser.add_argument("version")
    args = parser.parse_args()
    config = xdg("XDG_CONFIG_HOME", ".config") / "overtone"
    if args.action == "plugins":
        print(json.dumps({**plugin_config.configuration(config),
                          "stats": plugin_config.summary(config)}, ensure_ascii=False))
        return
    if args.action == "catalog":
        print(plugin_config.import_catalog(config))
        return
    if args.action == "plugins-save":
        result = plugin_config.save(config, json.loads(sys.stdin.read(256 * 1024)))
        print(json.dumps(result, ensure_ascii=False))
        return

    current = args.version
    version_file = PAYLOAD / "release-version"
    if version_file.is_file():
        current = version_file.read_text().strip()
    if args.action.startswith("update-"):
        prefix = Path.home() / ".local"
        supported = PAYLOAD == prefix / "share/overtone/yandex" and version_file.is_file()
        asset = update.latest(current)
        if args.action == "update-install" and asset:
            if not supported:
                raise Incompatible("Эта установка обновляется системным менеджером пакетов.")
            update.install(asset, prefix)
        print(json.dumps({"available": asset and asset["version"], "supported": supported}))
        return

    executable = find_installation()
    active = executable.parent/"resources/app.asar"
    if args.action == "status":
        result = inspect(active, current)[0]
    elif args.action == "restore":
        result = restore(executable, active)
    else:
        result = install(executable, active, current)
        if args.action == "launch":
            launch(executable)
    print(f"{VERSION} {result}")


if __name__ == "__main__":
    os.umask(0o077)
    try:
        main()
    except PermissionError as error:
        print(f"Нет прав на запись в каталог Яндекс Музыки. {error.filename}", file=sys.stderr)
        sys.exit(1)
    except (OSError, Incompatible) as error:
        print(error, file=sys.stderr)
        sys.exit(1)
    except ValueError as error:
        print(error, file=sys.stderr)
        sys.exit(1)
    except (KeyError, TypeError, struct.error, AttributeError):
        print(f"Некорректные данные. Поддерживается Яндекс Музыка {VERSION} для Linux.", file=sys.stderr)
        sys.exit(1)
