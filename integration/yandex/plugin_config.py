"""Read and save settings shared by the launcher and the desktop mod."""
from datetime import date, timedelta
import json
from pathlib import Path
import re
import tempfile
import urllib.request

DEFAULTS = {"fckcensor": False, "time": True, "fonts": False, "font": "Sans",
            "direct": True, "autoUpdate": True, "replacements": {}}


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
    if "replacements" in changes:
        values = changes["replacements"]
        if not isinstance(values, dict) or len(values) > 5000:
            raise ValueError("Список замен должен содержать до 5000 пар ID.")
        full_id = re.compile("[0-9]{1,24}").fullmatch
        for source, target in values.items():
            if not isinstance(target, str) or not full_id(source) or not full_id(target) or source == target:
                raise ValueError("Укажите разные числовые ID исходного и нового трека.")
        current["replacements"] = values
    write(directory / "plugins.json", current)
    return current


def write(path, value):
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=path.parent) as folder:
        temporary = Path(folder) / path.name
        temporary.write_text(json.dumps(value, ensure_ascii=False) + "\n")
        temporary.replace(path)


def import_catalog(directory):
    url = "https://raw.githubusercontent.com/Hazzz895/FckCensorData/main/list_v2.json"
    request = urllib.request.Request(url, headers={"User-Agent": "Overtone FckCensor adapter"})
    with urllib.request.urlopen(request, timeout=10) as response:
        data = response.read(1024 * 1024 + 1)
    if len(data) > 1024 * 1024:
        raise ValueError("Каталог FckCensor слишком большой.")
    value = json.loads(data)
    tracks = {}
    for source in value.get("sources", []):
        for track_id, payload in source.get("tracks", {}).items():
            if not re.fullmatch(r"[0-9]{1,24}", track_id) or not isinstance(payload, dict):
                continue
            fields = {key: text.strip()[:260] for key in ("title", "version", "coverUri")
                      if isinstance(text := payload.get(key), str) and text.strip()}
            if fields:
                tracks[track_id] = fields
    if not tracks or len(tracks) > 5000:
        raise ValueError("Каталог FckCensor не содержит поддерживаемых метаданных.")
    write(directory / "fckcensor-metadata.json", tracks)
    return len(tracks)


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
