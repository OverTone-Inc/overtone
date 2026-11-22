"""Read and save settings shared by the launcher and the desktop mod."""
from datetime import date, timedelta
import json
from pathlib import Path
import tempfile

DEFAULTS = {"fckcensor": False, "time": True, "fonts": False, "font": "Sans",
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
    current = configuration(directory)
    boolean = ("fckcensor", "time", "fonts", "direct", "autoUpdate")
    for key in boolean:
        if type(changes.get(key)) is bool:
            current[key] = changes[key]
    font = changes.get("font")
    if isinstance(font, str):
        current["font"] = font.strip()[:100] or "Sans"
    write(directory / "plugins.json", current)
    return current


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
    days = load_json(directory / "listening-time.json").get("days", {})
    if not isinstance(days, dict):
        days = {}
    today = date.today()
    monday = (today - timedelta(days=today.weekday())).isoformat()
    value = lambda x: x if isinstance(x, (int, float)) and 0 <= x <= 86_400_000 else 0
    return {
        "today": int(value(days.get(today.isoformat(), 0)) / 1000),
        "week": int(sum(value(v) for k, v in days.items() if monday <= k <= today.isoformat()) / 1000),
        "total": int(sum(map(value, days.values())) / 1000),
    }
