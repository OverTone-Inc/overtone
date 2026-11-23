#!/usr/bin/env python3
"""Generate the static site from shared project data and the local Arch package."""
import hashlib
import html
import json
import pathlib
import shutil
import subprocess
import tomllib
from urllib.parse import urlsplit

ROOT = pathlib.Path(__file__).resolve().parents[1]
DIST = ROOT / "dist"

def read(name):
    return json.loads((ROOT / "data" / f"{name}.json").read_text())
def esc(value):
    return html.escape(str(value))
PROJECT = read("project")
NAME = esc(PROJECT["name"])
ADDONS = read("yandex-addons")
VERSION = tomllib.loads((ROOT/"Cargo.toml").read_text())["workspace"]["package"]["version"]
NAV = {"index":"Главная", "plugins":"Плагины", "download":"Загрузки", "help":"Совместимость"}
def href(key):
    return f"{key}.html"
def screen(key, alt, cls="shot"):
    file = DIST/f"assets/{key}.webp"
    if not file.is_file():
        return ""
    return f'<figure class="{cls}" data-visual><img src="assets/{key}.webp" alt="{esc(alt)}" decoding="async"></figure>'
def page(key, title, description, contents, close=""):
    assets = ""
    if key == "index":
        assets = ('<link rel="stylesheet" href="assets/motion/motion.css">'
                  '<script>document.documentElement.classList.add("m-enter");'
                  'setTimeout(()=>document.documentElement.classList.add("m-enter-done"),700)</script>'
                  '<script type="module" src="assets/motion/motion.js"></script>'
                  '<script type="module" src="assets/moments/moments.js"></script>')
    nav = "".join(f'<a href="{href(page)}"{" aria-current=page" if page == key else ""}>{label}</a>' for page,label in NAV.items())
    links = "".join(
        f'<a href="{esc(url)}">{label}</a>'
        for name,label in [("source","Исходный код"),("issues","Обратная связь")]
        if (url:=PROJECT["links"].get(name)) and url.startswith("https://")
    )
    body = f"""<!doctype html>
<html lang="ru" data-type="software" data-radius="round" data-buttons="pill" data-ground="sand">
<head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title} · {NAME}</title>
<meta name="description" content="{esc(description)}"><meta name="theme-color" content="#f9f4ed">
<meta name="page-type" content="software"><meta name="site-kind" content="software">
<meta property="og:title" content="{title} · {NAME}"><meta property="og:description" content="{esc(description)}">
<link rel="icon" href="assets/logo.png" type="image/png">
<link rel="stylesheet" href="assets/style.css">{assets}
</head>
<body class="page-{key}">
<header class="top nav"><div class="wrap nav__bar"><a class="skip" href="#content">К содержанию</a><a class="brand" href="index.html"><img src="assets/logo.png" alt="" width="32" height="32">{NAME}</a>
<nav class="menu" aria-label="Основное меню">{nav}</nav></div></header>
<main id="content" tabindex="-1">{contents}</main>
<footer class="foot closing"><div class="wrap">{close}
<nav aria-label="Дополнительно"><a href="changes.html">Журнал изменений</a>{links}</nav></div></footer>
</body></html>
"""
    (DIST/href(key)).write_text(body)
def intro(title, lead, extra=""):
    return f'<section class="intro"><div class="wrap" data-enter><h1>{title}</h1><p class="muted">{lead}</p>{extra}</div></section>'
def band(contents, extra=""):
    return f'<section class="band {extra}"><div class="wrap">{contents}</div></section>'
def addon_cards():
    symbols = {"fckcensor": "♫", "time": "◷", "spicy": "≋", "fonts": "Aa"}
    descriptions = {
        "time": "Считает время прослушивания. В Electron также показывает серию входов.",
        "spicy": "Синхронизирует текст с треком. Пословное караоке, фон Kawarp и NowBar доступны в Electron.",
    }
    return "".join(
        f'<a class="addon-card" href="{esc(addon["url"]) if addon["url"] else "plugins.html#fonts"}">'
        f'<span class="addon-mark" role="img" aria-label="{esc(addon["category"])}">{symbols[addon["id"]]}</span>'
        f'<span class="addon-category">{esc(addon["category"])}</span><h3>{esc(addon["name"])}</h3>'
        f'<p class="addon-author">{esc(addon["author"])}</p><p>{esc(descriptions.get(addon["id"], addon["description"]))}</p>'
        '<span class="addon-arrow" aria-hidden="true">↗</span></a>' for addon in ADDONS
    )

def home(pkg):
    download_href = f"files/{esc(pkg[0])}" if pkg else "download.html"
    download_attr = " download" if pkg else ""
    visual = screen("desktop","Главное окно Overtone с двумя клиентами Яндекс Музыки","visual-stage hero-visual span-7")
    if not visual:
        visual = """<div class="visual-stage hero-visual span-7 app-frame" data-visual role="img" aria-label="Обзор приложения Overtone">
        <div class="app-header"><b>Overtone</b><span>Обзор</span></div>
        <div class="app-product"><img src="assets/logo.png" width="74" height="74" alt="">
        <div><strong>Яндекс Музыка<br>на Rust</strong><span>Скачать и установить</span></div></div>
        <div class="app-client">Яндекс Музыка + Overtone <span>Electron</span></div></div>"""
    hero = f"""<section class="hero music-hero" data-enter aria-labelledby="hero-title"><div class="wrap"><div class="hero-text hero-layout grid">
    <div class="hero-copy span-5"><p class="eyebrow">OVERTONE</p>
    <h1 id="hero-title">Яндекс Музыка<br><span>для Linux</span></h1>
    <p class="hero__lead">Отдельный клиент на Rust и мод для настольной Яндекс Музыки. Тексты, Discord RPC и плагины собраны в одном приложении.</p>
    <div class="actions hero__actions"><a class="button primary" href="{download_href}"{download_attr}>Скачать Overtone <span aria-hidden="true">↓</span></a>
    <a class="button secondary" href="#addons">Плагины</a></div>
    <p class="clarity">Для Linux x86_64</p></div>
    {visual}
    </div></div></section>"""
    addons = f"""<section class="music-addons" id="addons"><div class="wrap">
    <h2 data-moment="liquid-type" data-moment-opts='{{"strength":0.48}}'>Плагины для музыки</h2>
    <p class="addon-intro">Замена аудио из каталога, статистика, синхронизированные тексты и свои шрифты. Для обоих клиентов Яндекс Музыки.</p>
    <div class="addon-grid">{addon_cards()}</div>
    <a class="button secondary" href="plugins.html">Открыть каталог <span aria-hidden="true">↗</span></a>
    </div></section>"""
    if pkg:
        filename, _, size, digest = pkg
        command = f"sudo pacman -U ./{filename}"
        proof = f'<dt>Размер пакета</dt><dd>{size // (1024 * 1024)} МБ</dd><dt>SHA-256</dt><dd><code>{esc(digest)}</code></dd>'
    else:
        command = "CARGO_BUILD_JOBS=1 cargo build --locked --release"
        proof = '<dt>Платформа</dt><dd>Linux x86_64</dd>'
    close = f"""<div class="close-body grid">
    <div class="close-content">
    <h2>Сборка для Arch Linux</h2>
    <p>Установите Overtone, затем выберите нужный клиент в разделе «Обзор».</p>
    <a class="button primary" href="{download_href}"{download_attr}>Скачать Overtone <span aria-hidden="true">↓</span></a>
    <a class="close-setup" href="download.html">Все способы установки</a>
    </div>
    <div class="install-panel" data-ui aria-label="Команда установки и данные пакета">
    <p class="install-heading">Overtone {VERSION} · x86_64</p>
    <pre><code>{esc(command)}</code></pre>
    <dl>{proof}</dl>
    </div></div>"""
    page("index", "Яндекс Музыка для Linux", "Нативный Rust-клиент, мод Яндекс Музыки, плагины и статистика.", hero + addons, close)

def plugin_pages():
    built_in = """<div id="fonts" class="prose"><h2>Custom Fonts</h2><p>Плагин Overtone применяет системный или импортированный шрифт. В Rust поддерживаются TTF, OTF и WOFF.</p></div>
    <div class="prose"><h2>Плагины в Rust-клиенте</h2>
    <p>Нажмите «Overtone» в верхней панели и откройте «Плагины». Там можно включить FckCensor, посмотреть статистику YandexMusicTime, настроить шрифт и включить построчный режим SP Lyrics.</p></div>"""
    page("plugins", "Плагины", "Каталог дополнений для Яндекс Музыки.",
        intro("Плагины", "Дополнения Overtone для настольной Яндекс Музыки.") +
        band(f'<div class="addon-grid">{addon_cards()}</div>', "catalog-list") +
        band(built_in + screen("addons", "Каталог плагинов с авторами и настройками"), "plugin-screenshot"))


def package_metadata():
    for package in sorted((ROOT/"release").glob(f"overtone-{VERSION}-*-x86_64.pkg.tar.zst"),reverse=True):
        info = subprocess.check_output(["bsdtar","-xOf",package,".PKGINFO"],text=True)
        fields = dict(line.split(" = ",1) for line in info.splitlines() if " = " in line)
        if fields.get("pkgname") == "overtone" and fields.get("arch") == "x86_64":
            out = DIST/"files"
            out.mkdir(exist_ok=True)
            shutil.copy2(package,out/package.name)
            with package.open("rb") as file:
                sha = hashlib.file_digest(file,"sha256").hexdigest()
            return package.name,fields["pkgver"],package.stat().st_size,sha
    return None
def download(pkg):
    if pkg:
        name,version,size,sha = map(esc,pkg)
        parts = f"""<h2>Пакет Arch</h2>
        <p><a class="button primary" href="files/{name}" download>Скачать пакет x86_64</a></p>
        <dl><dt>Файл</dt><dd><code>{name}</code></dd><dt>Версия пакета</dt><dd>{version}</dd>
        <dt>Размер</dt><dd>{size} байт</dd><dt>SHA-256</dt><dd><code>{sha}</code></dd></dl>
        <h3>Проверить и установить</h3><pre>echo '{sha}  {name}' | sha256sum -c -
sudo pacman -U ./{name}</pre>
        <h3>Удалить приложение</h3><pre>overtone --quit
sudo pacman -R overtone</pre><p>Настройки хранятся в пользовательском каталоге.</p>"""
    else:
        parts = '<h2>Пакет Arch</h2><p>Локального файла пакета в этой сборке сайта пока нет. Соберите приложение из рабочей копии исходников.</p>'
    build = """<h2>Сборка из исходников</h2>
    <pre>CARGO_BUILD_JOBS=1 cargo build --locked --release
./target/release/overtone
PKGDEST="$PWD/release" CARGO_BUILD_JOBS=1 makepkg --clean</pre>
    <p>Для сборки нужны Rust, GTK4, pkgconf и Python 3. Для сборки пакета нужны инструменты Arch Linux.</p>"""
    manifest = ROOT / "integration/yandex/rust-release.json"
    if manifest.is_file():
        binary = pathlib.PurePosixPath(urlsplit(json.loads(manifest.read_text())["url"]).path).name
        source = binary.removesuffix("-x86_64.gz") + "-source.tar.gz"
        if (DIST / "files" / source).is_file():
            build += f'<h2>Яндекс Музыка (Rust)</h2><p>Установите отдельный клиент через «Обзор» Overtone.</p><p><a href="files/{esc(source)}" download>Скачать исходники Rust-клиента</a> · GPL-3.0</p>'
    page("download","Загрузки","Пакет Arch x86_64, SHA‑256 и команды установки.",
        intro("Загрузки","Пакет Overtone для Arch Linux x86_64 и исходники приложения.")
        +band(f'<div class="prose">{parts}{build}</div>'))
def help_page():
    help_ = """<div class="prose"><h2>Два клиента Яндекс Музыки</h2>
    <h3>Яндекс Музыка (Rust)</h3><p>В «Обзоре» нажмите «Скачать и установить». Клиент появится отдельным приложением в меню Linux. Для воспроизведения войдите в аккаунт Яндекса.</p>
    <h3>Яндекс Музыка + Overtone</h3><p>Поддерживается настольная Яндекс Музыка 5.109.1 для Linux x86_64. Перед установкой и обновлением мода закройте её. Восстановить исходное приложение можно в настройках Overtone.</p>
    <h3>Где включить дополнения</h3><p>В Rust-клиенте нажмите «Overtone» в верхней панели и выберите «Плагины». Для Electron откройте «Плагины» в приложении Overtone. Авторы и ссылки на исходники собраны в <a href="plugins.html">каталоге</a>.</p></div>"""
    page("help", "Совместимость", "Требования к двум клиентам Яндекс Музыки.",
         intro("Совместимость", "Сборки Overtone рассчитаны на Linux x86_64.") + band(help_))
def changes():
    records = "".join(f'<div class="prose"><h2>{esc(r["version"])}</h2><p class="muted">{esc(r["status"])}</p><ul>'
       +"".join(f'<li>{esc(item)}</li>' for item in r["items"])+"</ul></div>" for r in read("changes") if r["version"] != "0.1.0")
    page("changes","Журнал изменений","Сведения из записей о сборках.",intro("Журнал изменений","Сведения о доступных сборках.")+band(records))
def main():
    if DIST.exists():
        shutil.rmtree(DIST)
    shutil.copytree(ROOT/"site/assets",DIST/"assets")
    shutil.copy2(ROOT/"assets/icons/overtone-app-64.png",DIST/"assets/logo.png")
    for name in ("desktop.webp", "addons.webp"):
        shot = ROOT / "site/screens" / name
        if shot.is_file():
            shutil.copy2(shot, DIST / "assets" / name)
    pkg = package_metadata()
    for file in (ROOT / "release").glob("overtone-yamusic-rust-*.gz"):
        (DIST / "files").mkdir(exist_ok=True)
        shutil.copy2(file, DIST / "files" / file.name)
    home(pkg)
    plugin_pages()
    download(pkg)
    help_page()
    changes()
    print(DIST)
if __name__ == "__main__":
    main()
