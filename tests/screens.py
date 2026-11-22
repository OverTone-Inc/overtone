#!/usr/bin/env python3
"""Capture the release GTK window with generated audio and isolated user data."""
import contextlib
import json
import os
import pathlib
import subprocess as sp
import sys
import tempfile
import time

from measure import terminate, windows
from smoke import ipc, retry

ROOT = pathlib.Path(__file__).resolve().parents[1]

def main():
    bins = pathlib.Path(sys.argv[1] if len(sys.argv)>1 else ROOT/"target/release").resolve()
    output = ROOT/"site/screens"
    with tempfile.TemporaryDirectory(prefix="ot-screen-") as tmp, contextlib.ExitStack() as stack:
        folder = pathlib.Path(tmp)
        env = dict(os.environ,XDG_RUNTIME_DIR=tmp,XDG_CONFIG_HOME=str(folder/"cfg"),
            XDG_DATA_HOME=str(folder/"share"),GSETTINGS_BACKEND="memory",
            PULSE_SERVER=os.environ.get("PULSE_SERVER",f"unix:{os.environ['XDG_RUNTIME_DIR']}/pulse/native"))
        def ctl(*args):
            return json.loads(sp.check_output([bins/"overtone-ctl",*map(str,args)],env=env,text=True))
        def start(*args):
            p=sp.Popen(args,env=env)
            stack.callback(terminate,p)
            return p
        def shot(file, window, page=None):
            if page:
                sp.run([bins/"overtone","--page",page],env=env,check=True,timeout=4)
            time.sleep(.55)
            path = output/f"{file}.webp"
            sp.run(["import","-window",window,"-quality","89",path],check=True)
            assert path.stat().st_size>1200,path
        a = folder/"A · Оригинал.flac"
        b = folder/"B · Другая запись.flac"
        c = folder/"C · Следующий сигнал.flac"
        for (hz,title,path) in [(440,"Оригинал · 440 Гц",a),(880,"Другая запись · 880 Гц",b),(660,"Следующий сигнал · 660 Гц",c)]:
            sp.run(["ffmpeg","-nostdin","-v","error","-f","lavfi","-i",f"sine=frequency={hz}:duration=90",
                    "-metadata",f"title={title}","-metadata","artist=Overtone · Локальная демонстрация", "-c:a","flac",path],check=True)
        p=start(bins/"overtone-ctl","mpv")
        daemon=start(bins/"overtone-background")
        path=folder/"overtone"/f"mpv-{p.pid}.sock"
        retry(lambda: ctl("status")["state"]["adapter"],"Нет mpv с адаптером")
        ipc(path,"set_property","mute",True)
        ctl("toggle","history","true")
        ctl("rule",a,b)
        ctl("queue",a,c)
        retry(lambda: ctl("status")["state"]["adapter"]["load"]["state"]=="applied","Нет замены")
        # Make a genuine history entry before returning to the replacement.
        time.sleep(2.1)
        ctl("next")
        retry(lambda: ctl("status")["state"]["history"],"Нет записи истории")
        ctl("queue",a,c)
        ui = start(bins/"overtone")
        home = retry(lambda: next(iter(windows("^Overtone$",ui.pid)),None),"Менеджер не открылся")
        shot("overview",home)
        shot("versions",home,"versions")
        ctl("timer",1800)
        shot("timer",home,"timer")
        shot("history",home,"history")
        sp.run([bins/"overtone","--mini"],env=env,check=True,timeout=4)
        mini=retry(lambda: next(iter(windows("^Overtone · Мини-плеер$",ui.pid)),None),"Мини-плеер не открылся")
        shot("mini",mini)
        sp.run([bins/"overtone","--quit"],env=env,check=True,timeout=5)
        ui.wait(timeout=5)
        daemon.wait(timeout=5)
        print("GTK4: сохранены 5 снимков собранного приложения.",flush=True)
if __name__=="__main__":
    main()
