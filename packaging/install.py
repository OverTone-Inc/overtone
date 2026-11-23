#!/usr/bin/env python3
"""Stage the release and generate the desktop entry from the project configuration."""
import argparse
import json
from pathlib import Path
import shutil
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
PROJECT = json.loads((ROOT/"data/project.json").read_text())
NAME, APP_ID = PROJECT["name"], PROJECT["id"]
def entry(command="overtone"):
    return f"""[Desktop Entry]
Type=Application
Name={NAME}
Comment=Клиенты Яндекс Музыки, плагины и статистика прослушивания
Exec={command} --launcher
Icon={APP_ID}
Terminal=false
Categories=AudioVideo;Audio;Player;
StartupNotify=true
StartupWMClass=overtone
Actions=Manager;Mini;

[Desktop Action Manager]
Name=Менеджер плееров
Exec={command} --manager

[Desktop Action Mini]
Name=Мини-плеер
Exec={command} --mini
"""


def copy_file(source, destination):
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=destination.parent) as directory:
        staged = Path(directory)/destination.name
        shutil.copy2(source, staged)
        staged.replace(destination)


def install(base, binary, command="overtone"):
    items = [
        (binary / name, base / "bin" / name)
        for name in ["overtone", "overtone-background", "overtone-ctl", "overtone-route"]
    ]
    license_file = base / "share/licenses/overtone/LICENSE"
    items.extend([
        (ROOT/"LICENSE",license_file),
        (ROOT/"README.md",base/"share/doc/overtone/README.md"),
        (ROOT/"docs/integration.md",base/"share/doc/overtone/docs/integration.md"),
        (ROOT/"docs/catalog.md",base/"share/doc/overtone/docs/catalog.md"),
        (ROOT/"docs/catalog-example.json",base/"share/doc/overtone/docs/catalog-example.json"),
        (ROOT/"integration/overtone.lua",base/"share/overtone/adapter/overtone.lua"),
    ])
    mod = ROOT/"integration/yandex"
    for name in ("launch.py", "main.cjs", "discord.cjs", "preload.js", "renderer.js", "discord-application.json",
                 "plugins.cjs", "fckcensor.cjs", "plugin_config.py", "update.py", "addons.py",
                 "addon-host.js", "rust_client.py"):
        items.append((mod/name, base/"share/overtone/yandex"/name))
    if (mod / "rust-release.json").is_file():
        items.append((mod / "rust-release.json", base / "share/overtone/yandex/rust-release.json"))
    items.append((mod/"licenses/FckCensor-MIT.txt", base/"share/licenses/overtone/FckCensor-MIT.txt"))
    for name in ("yandex_music.py", "music-domains.json"):
        items.append((ROOT/"integration/vpn"/name, base/"share/overtone/vpn"/name))
    items.append((ROOT/"docs/vpn.md", base/"share/doc/overtone/docs/vpn.md"))
    for size in [32,64,128,256,512]:
        items.append((ROOT/f"assets/icons/overtone-app-{size}.png",base/f"share/icons/hicolor/{size}x{size}/apps/{APP_ID}.png"))
    for source,destination in items:
        copy_file(source, destination)
    version = tomllib.loads((ROOT/"Cargo.toml").read_text())["workspace"]["package"]["version"]
    (base/"share/overtone/yandex/release-version").write_text(version+"\n")
    desktop = base/f"share/applications/{APP_ID}.desktop"
    desktop.parent.mkdir(parents=True,exist_ok=True)
    desktop.write_text(entry(command))
    desktop.chmod(0o644)
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version")
    target = parser.add_mutually_exclusive_group()
    target.add_argument("--stage",type=Path)
    target.add_argument("--user",action="store_true")
    parser.add_argument("--bin-dir",type=Path)
    parser.add_argument("--desktop-entry",action="store_true")
    args = parser.parse_args()
    if args.version:
        version = tomllib.loads((ROOT/"Cargo.toml").read_text())["workspace"]["package"]["version"]
        if version != args.version:
            parser.error(f"PKGBUILD {args.version} не совпадает с Cargo {version}")
    if (args.user or args.stage) and not args.bin_dir:
        parser.error("--bin-dir is required with an installation target")
    if args.stage:
        install(args.stage/"usr", args.bin_dir)
    if args.user:
        base = Path.home()/".local"
        command = str(base/"bin/overtone")
        command = command.replace("%", "%%").replace("\\", "\\\\")
        for char in ('"', '`', '$'):
            command = command.replace(char, "\\" + char)
        install(base, args.bin_dir, f'"{command}"')
    if args.desktop_entry:
        print(entry(), end="")
if __name__ == "__main__":
    main()
