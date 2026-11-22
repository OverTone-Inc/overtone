#!/usr/bin/env python3
"""Generate the static site from shared project data and the local Arch package."""
import hashlib
import html
import json
import pathlib
import shutil
import subprocess
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[1]
DIST = ROOT / "dist"

def read(name):
    return json.loads((ROOT / "data" / f"{name}.json").read_text())
def esc(value):
    return html.escape(str(value))
PROJECT = read("project")
NAME = esc(PROJECT["name"])
PLUGINS = read("plugins")
VERSION = tomllib.loads((ROOT/"Cargo.toml").read_text())["workspace"]["package"]["version"]
NAV = {"index":"Главная", "plugins":"Плагины", "download":"Загрузки", "help":"Совместимость"}
ICON = {
    "versions": '<path d="M4 8h12m-3-3 3 3-3 3M20 16H8m3-3-3 3 3 3"/>',
    "mini": '<rect x="4" y="6" width="16" height="12" rx="2"/><path d="m10 9 5 3-5 3z"/>',
    "timer": '<circle cx="12" cy="13" r="7"/><path d="M12 10v3l2 2M10 3h4"/>',
    "history": '<path d="M5 6v4h4M6 10a7 7 0 1 1 .3 6M12 9v4l3 1"/>',
}
DETAIL = {
    "versions": "Сохраните пару из оригинала и своей версии. Ручное правило имеет приоритет, а исключение «Всегда играть оригинал» блокирует замену из каталога. Нажмите «Предпрослушать в mpv», чтобы применить версию к текущей позиции.",
    "mini": "Откройте мини-плеер из шапки менеджера. Он показывает обложку, название, исполнителя и состояние. В X11 можно сохранить положение окна и закрепить его поверх остальных.",
    "timer": "Выберите 15, 30, 45, 60 минут или задайте свой интервал. Таймер продолжает работать при закрытом менеджере. После пробуждения компьютера он проверяет экземпляр плеера перед паузой.",
    "history": "Включите запись в разделе «Плагины». Найдите трек по названию или исполнителю, экспортируйте JSON или очистите записи после подтверждения. Хранится до 600 завершённых прослушиваний."
}
def href(key):
    return f"{key}.html"
def mark(key):
    return f'<span class="plugin-mark" aria-hidden="true"><svg viewBox="0 0 24 24">{ICON[key]}</svg></span>'
def screen(key, alt, cls="shot"):
    file = DIST/f"assets/{key}.webp"
    if not file.is_file():
        return ""
    return f'<figure class="{cls}" data-visual><img src="assets/{key}.webp" alt="{esc(alt)}" decoding="async"></figure>'
def page(key, title, description, contents):
    nav = "".join(f'<a href="{href(page)}"{" aria-current=page" if page == key else ""}>{label}</a>' for page,label in NAV.items())
    links = "".join(
        f'<a href="{esc(url)}">{label}</a>'
        for name,label in [("source","Исходный код"),("issues","Обратная связь")]
        if (url:=PROJECT["links"].get(name)) and url.startswith("https://")
    )
    canonical = PROJECT["links"].get("home")
    canonical = f'<link rel="canonical" href="{esc(canonical.rstrip("/"))}/{"" if key=="index" else href(key)}">' if canonical else ""
    body = f"""<!doctype html>
<html lang="ru" data-type="software" data-radius="soft" data-buttons="solid">
<head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title} · {NAME}</title>
<meta name="description" content="{esc(description)}"><meta name="theme-color" content="#181b1a">
<meta name="page-type" content="software"><meta name="site-kind" content="software">
<meta property="og:title" content="{title} · {NAME}"><meta property="og:description" content="{esc(description)}">
<link rel="icon" href="assets/logo.png" type="image/png">{canonical}
<link rel="stylesheet" href="assets/style.css">
</head>
<body>
<header class="top nav"><a class="skip" href="#content">К содержанию</a><div class="wrap"><a class="brand" href="index.html"><img src="assets/logo.png" alt="" width="32" height="32">{NAME}</a>
<nav class="menu" aria-label="Основное меню">{nav}</nav></div></header>
<main id="content" tabindex="-1">{contents}</main>
<footer class="foot closing"><div class="wrap grid"><span class="muted">{NAME} · Linux x86_64</span><nav aria-label="Дополнительно"><a href="changes.html">Журнал изменений</a>{links}</nav></div></footer>
</body></html>
"""
    (DIST/href(key)).write_text(body)
def intro(title, lead, extra=""):
    return f'<section class="intro"><div class="wrap" data-enter><h1>{title}</h1><p class="muted">{lead}</p>{extra}</div></section>'
def band(contents, extra=""):
    return f'<section class="band {extra}"><div class="wrap">{contents}</div></section>'
def home(pkg):
    tiles = "".join(f'<a class="tile" href="plugins.html#{p["id"]}">{mark(p["id"])}<h3>{esc(p["title"])}</h3><p>{esc(p["description"])}</p></a>' for p in PLUGINS)
    download_href = f"files/{esc(pkg[0])}" if pkg else "download.html"
    download_attr = " download" if pkg else ""
    hero = f"""<section class="hero" data-enter aria-labelledby="hero-title"><div class="wrap"><div class="hero-text hero-grid grid">
    <div class="hero-copy"><p class="eyebrow">Нативное приложение для Linux</p><h1 id="hero-title">Выберите версию трека</h1>
    <p class="hero__lead">Сохраняйте свои замены, управляйте плеером и ставьте музыку на паузу по таймеру.</p>
    <div class="actions hero__actions"><a class="button primary" href="{download_href}"{download_attr}>Скачать сборку</a>
    <a class="button" href="help.html">Проверить совместимость</a></div>
    <p class="clarity">Автозамена проверена на локальном аудио в mpv.</p></div>
    {screen("overview","Менеджер с подтверждённой заменой аудиопотока в mpv","hero-image")}
    </div></div></section>"""
    mini = screen("mini","Мини-плеер с управлением текущей записью")
    page("index","Управление музыкой на Linux","Версии треков, мини-плеер, таймер и история в нативном приложении для Linux.",
        hero+band(f'<div class="section-head"><h2>Четыре встроенных плагина</h2></div><div class="plugin-feature grid"><div class="plugin-grid grid">{tiles}</div><div class="plugin-aside">{mini}<a class="button" href="plugins.html">Открыть каталог плагинов</a></div></div>',"home-plugins"))
def plugin_pages():
    links = "".join(f'<a href="#{p["id"]}">{esc(p["title"])}</a>' for p in PLUGINS)
    cards = []
    for p in PLUGINS:
        key = p["id"]
        cards.append(f"""<section class="band plugin-detail" id="{key}"><div class="wrap detail-grid">
        <div class="detail-copy"><h2>{esc(p["title"])}</h2><p class="plugin-summary">{esc(p["name"])} · {esc(p["version"])}</p>
        <p>{DETAIL[key]}</p><p class="muted">{esc(p["requires"])}</p></div>
        {screen(key,f'Окно приложения. {p["title"]}')}</div></section>""")
    page("plugins","Плагины","Возможности и настройки четырёх встроенных плагинов.",
        intro("Плагины","Каждый плагин можно включать и выключать в менеджере.",f'<nav class="jump" aria-label="К плагину">{links}</nav>')+"".join(cards))
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
        <p>Для локальной автозамены установите mpv и mpv-mpris.</p><pre>sudo pacman -S mpv mpv-mpris</pre>
        <h3>Удалить приложение</h3><pre>overtone --quit
sudo pacman -R overtone</pre><p>Настройки и правила остаются в пользовательском каталоге.</p>"""
    else:
        parts = '<h2>Пакет Arch</h2><p>Локального файла пакета в этой сборке сайта пока нет. Соберите приложение из рабочей копии исходников.</p>'
    build = """<h2>Сборка из исходников</h2>
    <pre>CARGO_BUILD_JOBS=1 cargo build --locked --release
./target/release/overtone
PKGDEST="$PWD/release" CARGO_BUILD_JOBS=1 makepkg --clean</pre>
    <p>Для сборки нужны Rust, GTK4, pkgconf и Python 3. Для сборки пакета нужны инструменты Arch Linux.</p>"""
    page("download","Загрузки","Пакет Arch x86_64, SHA‑256 и команды установки.",
        intro("Загрузки","Предварительная сборка для Arch Linux x86_64. Публичный релиз ещё не опубликован.")
        +band(f'<div class="prose">{parts}{build}</div>'))
def help_page():
    cols = ["client","version","control","replacement","queue","date"]
    rows = "".join("<tr><th scope=\"row\">"+esc(r["client"])+"</th>"+"".join(f"<td>{esc(r[k])}</td>" for k in cols[1:])+"</tr>" for r in read("compatibility"))
    table = f"""<div class="table-wrap" tabindex="0" role="region" aria-label="Таблица совместимости"><table>
    <caption>Результаты проверок</caption><thead><tr>
    <th scope="col">Клиент</th><th scope="col">Версия</th><th scope="col">Управление</th><th scope="col">Замена</th><th scope="col">Очередь</th><th scope="col">Дата проверки</th>
    </tr></thead><tbody>{rows}</tbody></table></div>"""
    help_ = """<div class="prose"><h2>Проверка после обновления</h2>
    <h3>Обновился плеер</h3><p>В «Обзоре» выберите нужный экземпляр и нажмите «Перепроверить». В «Диагностике» видны версия и разрешённые команды. Для новой версии mpv управление MPRIS остаётся доступным, а автозамена ожидает проверки адаптера.</p>
    <h3>Пропала запись</h3><p>Проверьте, что исходная запись доступна в плеере, а файл замены открывается сам по себе. В «Заменах» можно поменять источник, отключить правило и вернуть оригинал.</p>
    <h3>Не работает один плагин</h3><p>Откройте «Плагины» и прочитайте статус нужной строки. Проверьте настройки плагина и возможности плеера. Из раздела «Диагностика» можно скопировать отчёт без ссылки на трек и пути к файлу.</p>
    <h3>Яндекс Музыка показывает ошибку загрузки</h3><p>Когда клиент снова воспроизводит записи, выберите его в Overtone и проверьте Play/Pause. Затем проверьте, передаёт ли он URI исходного трека. Для открытия альтернативы клиент должен объявить нужную схему и подтвердить URI при воспроизведении. Автозамена для Яндекс Музыки пока не готова.</p>
    <h3>Сохранить собственную пару</h3><p>Откройте локальную очередь в mpv с адаптером, выберите «Заменить этот трек» и укажите файл другой версии. Сохраните пару и запустите исходный трек снова. Единичный предпросмотр и возврат оригинала оставляют остальные позиции очереди на месте.</p>
    <h3>Импортировать список правил</h3><p>В «Заменах» выберите JSON или укажите HTTPS-адрес в настройках. Проверьте результат предпросмотра и подтвердите импорт. <a href="assets/catalog-example.json" download>Скачать пример каталога Overtone JSON v1</a>.</p>
    </div>"""
    page("help","Совместимость","Проверенные плееры и восстановление после обновления.",
        intro("Совместимость","Выберите режим по возможностям своего плеера.")+band(table)+band(help_))
def changes():
    records = "".join(f'<div class="prose"><h2>{esc(r["version"])}</h2><p class="muted">{esc(r["status"])}</p><ul>'
       +"".join(f'<li>{esc(item)}</li>' for item in r["items"])+"</ul></div>" for r in read("changes"))
    page("changes","Журнал изменений","Сведения из записей о сборках.",intro("Журнал изменений","Сведения о доступных сборках.")+band(records))
def main():
    if DIST.exists():
        shutil.rmtree(DIST)
    shutil.copytree(ROOT/"site/assets",DIST/"assets")
    shutil.copy2(ROOT/"assets/icons/overtone-app-64.png",DIST/"assets/logo.png")
    for shot in (ROOT/"site/screens").glob("*.webp"):
        shutil.copy2(shot,DIST/"assets"/shot.name)
    pkg = package_metadata()
    shutil.copy2(ROOT/"docs/catalog-example.json", DIST/"assets/catalog-example.json")
    home(pkg)
    plugin_pages()
    download(pkg)
    help_page()
    changes()
    print(DIST)
if __name__ == "__main__":
    main()
