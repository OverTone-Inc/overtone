"""Build the pinned PulseSync add-ons in the user's own profile."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

SOURCES = {
    "time": ("mak7im01/YandexMusicTime", "831665b9c82fc017ff3d2f2134b4c6359bfd0dc3",
             "708d64df-6691-4659-acdc-4351765e2d12"),
    "spicy": ("saveksme/pulsesync-spicy-lyrics", "6b735afc25d3d1ef58e4082fc1496d687c0771c4", "spicy-lyrics"),
}
FILES = ("script.js", "script.css", "metadata.json", "react.js")


def versions(directory):
    installed = {}
    for name, (_, revision, _) in SOURCES.items():
        target = directory / "addons" / name
        try:
            if (target / "revision").read_text().strip() == revision and all((target / file).is_file() for file in FILES):
                installed[name] = json.loads((target / "metadata.json").read_text())["version"]
        except (OSError, ValueError, KeyError):
            continue
    return installed


def command(*args, cwd):
    result = subprocess.run(args, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            text=True, timeout=240, env=dict(os.environ, GIT_TERMINAL_PROMPT="0"))
    if result.returncode:
        raise ValueError(f"Команда {args[0]} завершилась с ошибкой.\n{result.stdout[-900:].strip()}")


def stage(directory, name, checkout):
    _, revision, addon_id = SOURCES[name]
    head = subprocess.check_output(["git", "-C", checkout, "rev-parse", "HEAD"], text=True).strip()
    if head != revision:
        raise ValueError("Ревизия аддона не совпадает с проверенной.")
    output = checkout / "dist" / addon_id
    metadata = json.loads((output / "metadata.json").read_text())
    if metadata.get("id") != addon_id:
        raise ValueError("Сборка вернула другой аддон.")
    target = directory / "addons" / name
    target.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=target) as folder:
        temporary = Path(folder)
        for file in FILES[:-1]:
            shutil.copyfile(output / file, temporary / file)
        shutil.copyfile(checkout / "node_modules/react/cjs/react.production.js", temporary / "react.js")
        (temporary / "revision").write_text(revision + "\n")
        for file in (*FILES, "revision"):
            (temporary / file).replace(target / file)
    return metadata["version"]


def install(directory, name):
    if name not in SOURCES:
        raise ValueError("Неизвестный аддон.")
    available = versions(directory).get(name)
    if available:
        return available
    for executable in ("git", "node", "npm"):
        if not shutil.which(executable):
            raise ValueError("Для локальной сборки аддона нужны Git, Node.js 22.12+ и npm.")
    repo, revision, _ = SOURCES[name]
    with tempfile.TemporaryDirectory(prefix="overtone-addon-") as folder:
        checkout = Path(folder)
        command("git", "init", "-q", cwd=checkout)
        command("git", "fetch", "-q", "--depth=1", f"https://github.com/{repo}.git", revision, cwd=checkout)
        command("git", "checkout", "-q", "--detach", "FETCH_HEAD", cwd=checkout)
        if name == "time":
            command("npm", "exec", "--yes", "--package=yarn@1.22.22", "--", "yarn", "install",
                    "--frozen-lockfile", "--ignore-scripts", "--non-interactive", cwd=checkout)
        else:
            command("npm", "ci", "--ignore-scripts", "--no-fund", "--no-audit", cwd=checkout)
        command("npm", "run", "build", cwd=checkout)
        return stage(directory, name, checkout)
