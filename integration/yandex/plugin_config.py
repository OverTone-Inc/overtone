"""Read and save settings shared by the launcher and the desktop mod."""
from datetime import date, timedelta
import fcntl
import json
from pathlib import Path
import tempfile

DEFAULTS = {"fckcensor": False, "time": True, "spicy": False, "fonts": False, "font": "Sans",
            "direct": True, "autoUpdate": True}


def load_json(path):
    try:
        with path.open("rb") as stream:
            obj = json.load(stream)
        return obj if isinstance(obj, dict) else {}
    except (OSError, ValueError):
        return {}


def configuration(directory):
    previous = load_json(directory / "plugins.json")
    return {key: value if type(value) is type(DEFAULTS[key]) else DEFAULTS[key]
            for key, value in (DEFAULTS | previous).items() if key in DEFAULTS}


def save(directory, changes):
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    with (directory / "plugins.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        current = configuration(directory)
        for key, default in DEFAULTS.items():
            value = changes.get(key)
            if type(value) is type(default):
                current[key] = value.strip()[:100] or "Sans" if isinstance(value, str) else value
        write(directory / "plugins.json", current)
        return current


def mod_settings(directory):
    config = load_json(directory / "yandex-mod.json")
    modes = ("yandex", "external", "fallback", "prefer")
    value = config.get("lyricsMode")
    return {"enabled": config.get("enabled") is True, "lyricsMode": value if value in modes else "yandex"}


def save_mod(directory, changes):
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    with (directory / "plugins.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        file = directory / "yandex-mod.json"
        config = load_json(file)
        if type(changes.get("enabled")) is bool:
            config["enabled"] = changes["enabled"]
        if changes.get("lyricsMode") in ("yandex", "external", "fallback", "prefer"):
            config["lyricsMode"] = changes["lyricsMode"]
        write(file, config)


def write(path, value):
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=path.parent) as folder:
        temporary = Path(folder) / path.name
        temporary.write_text(json.dumps(value, ensure_ascii=False) + "\n")
        temporary.replace(path)


def font_label(directory):
    name = load_json(directory / "custom-font.json").get("name", "")
    return name if isinstance(name, str) else ""


def import_font(directory, source):
    signatures = {".ttf": {b"\x00\x01\x00\x00", b"true"}, ".otf": {b"OTTO"},
                  ".woff": {b"wOFF"}, ".woff2": {b"wOF2"}}
    with source.open("rb") as stream:
        data = stream.read(10 * 1024 * 1024 + 1)
    if len(data) > 10 * 1024 * 1024 or data[:4] not in signatures.get(source.suffix.lower(), ()):
        raise ValueError("Выберите шрифт TTF, OTF, WOFF или WOFF2 до 10 МБ.")
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=directory) as folder:
        target = Path(folder) / "custom-font"
        target.write_bytes(data)
        target.replace(directory / "custom-font")
    write(directory / "custom-font.json", {"name": source.name[:160]})
    return source.name[:160]


def clear_font(directory):
    (directory / "custom-font").unlink(missing_ok=True)
    (directory / "custom-font.json").unlink(missing_ok=True)


def summary(directory):
    previous = load_json(directory / "listening-time.json").get("days", {})
    addon = load_json(directory / "time-addon.json")
    original = addon.get("days", {})
    native = load_json(directory / "rust-listening-time.json").get("days", {})
    days = {}
    for record, scale in ((previous, 1000), (original, 1), (native, 1)):
        if not isinstance(record, dict):
            continue
        for day, value in record.items():
            if isinstance(value, (int, float)) and 0 <= value <= 86_400 * scale:
                days[day] = days.get(day, 0) + value / scale
    today = date.today()
    monday = (today - timedelta(days=today.weekday())).isoformat()
    streak = addon.get("streak")
    active_streak = {}
    if isinstance(streak, dict):
        since = streak.get("lastDate")
        count = streak.get("count", 0)
        best = streak.get("best", 0)
        if type(count) is int and type(best) is int:
            active_streak = {"current": count if since in (today.isoformat(), (today - timedelta(days=1)).isoformat()) else 0,
                             "best": best}
    return {
        "streak": active_streak,
        "today": int(days.get(today.isoformat(), 0)),
        "week": int(sum(v for k, v in days.items() if monday <= k <= today.isoformat())),
        "month": int(sum(v for k, v in days.items() if k.startswith(today.strftime("%Y-%m-")))),
        "total": int(sum(days.values())),
        "days": { (today - timedelta(days=i)).isoformat():
                  int(days.get((today - timedelta(days=i)).isoformat(), 0)) for i in reversed(range(7)) },
    }
