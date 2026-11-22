#!/usr/bin/env python3
"""Verify internal navigation and a real download from the built static site."""
import hashlib
from html.parser import HTMLParser
import http.server
from pathlib import Path
import sys
import threading
from urllib.parse import unquote, urlsplit
from urllib.request import urlopen

class Page(HTMLParser):
    def __init__(self, path):
        super().__init__()
        self.links = []
        self.ids = set()
        self.feed(path.read_text())
    def handle_starttag(self, tag, attrs):
        data = dict(attrs)
        if "id" in data:
            assert data["id"] not in self.ids
            self.ids.add(data["id"])
        for key in ("href","src"):
            if key in data:
                self.links.append(data[key])
        if tag == "img":
            assert "alt" in data

def main(directory):
    pages = list(directory.glob("*.html"))
    assert len(pages) == 5
    count = 0
    for path in pages:
        for link in Page(path).links:
            target = urlsplit(link)
            if target.scheme or target.netloc:
                continue
            relative = unquote(target.path)
            file = (path.parent / relative) if relative else path
            assert file.is_file(), (path.name,link)
            if target.fragment:
                assert unquote(target.fragment) in Page(file).ids, (path.name,link)
            count += 1
    matches = list((directory/"files").glob("*.pkg.tar.zst"))
    if not matches:
        print(f"{count} локальных ссылок проверено. Пропущена загрузка: отсутствует локальный пакет.")
        return
    def handler(*args, **kwargs):
        return http.server.SimpleHTTPRequestHandler(*args,directory=directory,**kwargs)
    with http.server.ThreadingHTTPServer(("127.0.0.1",0),handler) as server:
        t = threading.Thread(target=server.serve_forever,daemon=True)
        t.start()
        try:
            origin = f"http://127.0.0.1:{server.server_port}"
            file = matches[0]
            with urlopen(f"{origin}/files/{file.name}",timeout=3) as response:
                blob = response.read()
            sha256 = hashlib.sha256(blob).hexdigest()
            assert blob == file.read_bytes()
            with urlopen(f"{origin}/download.html",timeout=3) as response:
                assert sha256 in response.read().decode()
        finally:
            server.shutdown()
            t.join()
    print(f"{count} локальных ссылок. Скачан {file.name}, {len(blob)} байт, SHA-256 {sha256}")
if __name__ == "__main__":
    main(Path(sys.argv[1] if len(sys.argv)>1 else "dist").resolve())
