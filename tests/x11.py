#!/usr/bin/env python3
"""Verify app activation from an extracted desktop entry and the XFCE mini window."""
import contextlib
import json
import os
import pathlib
import re
import socket
import subprocess as sp
import sys
import tempfile
import time
from measure import terminate
from smoke import retry

ROOT = pathlib.Path(__file__).resolve().parents[1]

def xprop(window, atom):
    return sp.check_output(["xprop","-id",window,atom],text=True)
def visible(pattern):
    return sp.run(["xdotool","search","--onlyvisible","--name",pattern],
                  text=True,capture_output=True).stdout.split()
def commands(*args):
    sp.run(["xdotool",*map(str,args)],check=True,timeout=4)
def main():
    project = json.loads((ROOT/"data/project.json").read_text())
    name, app_id = project["name"], project["id"]
    stage = pathlib.Path(sys.argv[1]).resolve()
    binary = stage/"usr/bin"
    with tempfile.TemporaryDirectory(prefix="ot-x11-") as d, contextlib.ExitStack() as stack:
        folder = pathlib.Path(d)
        position = folder/"data/overtone/mini-position.json"
        position.parent.mkdir(parents=True,mode=0o700)
        position.write_text("[50000,50000]")
        env = dict(os.environ,PATH=f'{binary}:{os.environ["PATH"]}',XDG_RUNTIME_DIR=d,
            XDG_CONFIG_HOME=str(folder/"config"),XDG_DATA_HOME=str(folder/"data"),
            XDG_DATA_DIRS=f"{stage/'usr/share'}:/usr/local/share:/usr/share",GSETTINGS_BACKEND="memory")
        daemon = sp.Popen([binary/"overtone-background"],env=env)
        stack.callback(terminate,daemon)
        service=folder/"overtone/service.sock"
        retry(service.exists,"Нет фонового сокета")
        with socket.socket(socket.AF_UNIX) as conn:
            conn.connect(str(service))
            conn.sendall(b'{"command":"preferences","light":true,"above":true,"autostart":false}\n')
            assert json.loads(conn.makefile().readline())["error"] is None
        desktop=stage/f"usr/share/applications/{app_id}.desktop"
        sp.run(["desktop-file-validate",desktop],check=True)
        sp.run(["gio","launch",desktop],env=env,check=True,timeout=5)
        home=retry(lambda:next(iter(visible(f"^{name}$")),None),"Менеджер не запущен из desktop-entry")
        def run(*flags):
            sp.run([binary/"overtone",*flags],env=env,check=True,timeout=5)
        stack.callback(run,"--quit")
        pid = xprop(home,"_NET_WM_PID")
        print(xprop(home,"WM_CLASS").strip(),pid.strip(),flush=True)
        run("--page","plugins")
        assert visible(f"^{name}$") == [home]
        assert xprop(home,"_NET_WM_PID")==pid
        geometry = sp.check_output(["xwininfo","-id",home],text=True)
        width = int(re.search(r"Width:\s+(\d+)",geometry)[1])
        height = int(re.search(r"Height:\s+(\d+)",geometry)[1])
        assert width <= 1366 and height <= 768,(width,height)
        print(f"Desktop-entry и второй запуск · одно окно {width}×{height}",flush=True)
        run("--mini")
        mini=retry(lambda:next(iter(visible(f"^{name} · Мини-плеер$")),None),"Нет мини-плеера")
        retry(lambda:"_NET_WM_STATE_ABOVE" in xprop(mini,"_NET_WM_STATE"),"XFCE не подтвердил режим поверх окон")
        geom = sp.check_output(["xwininfo","-id",mini],text=True)
        x,y = map(int,re.findall(r"Absolute upper-left [XY]:\s+(-?\d+)",geom))
        root = sp.check_output(["xwininfo","-root"],text=True)
        rw = int(re.search(r"Width:\s+(\d+)",root)[1])
        rh = int(re.search(r"Height:\s+(\d+)",root)[1])
        assert 0 <= x < rw and 0 <= y < rh,(x,y,rw,rh)
        commands("windowmove",mini,110,120)
        time.sleep(.3)
        commands("windowactivate","--sync",mini,"key","Alt+F4")
        retry(lambda: json.loads(position.read_text())[0] < rw,"Положение не записано")
        print("XFCE подтвердил _NET_WM_STATE_ABOVE · положение после выхода за экран ограничено и сохранено",flush=True)
if __name__=="__main__":
    main()
