#!/usr/bin/env python3
"""Three release runs on X11. Each window state has a 30 second warmup."""
import argparse
import contextlib
import json
import os
import pathlib
import subprocess as sp
import tempfile
import time

from smoke import ipc, retry

ROOT = pathlib.Path(__file__).resolve().parents[1]

def proc_kb(pid):
    output = {}
    for line in pathlib.Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines():
        key, _, value = line.partition(":")
        if key in {"Pss", "Rss"}:
            output[key] = int(value.split()[0])
    return output

def pids(binary):
    expected = {str(binary / name) for name in ("overtone","overtone-background","overtone-ctl")}
    found = set()
    for path in pathlib.Path("/proc").iterdir():
        if path.name.isnumeric():
            try:
                if os.readlink(path/"exe") in expected:
                    found.add(int(path.name))
            except OSError:
                pass
    return found

def aggregate(processes):
    counts = [proc_kb(pid) for pid in processes]
    return {key: sum(p[key] for p in counts) for key in ("Pss","Rss")}

def ticks(proc_ids):
    return sum(sum(int(x) for x in pathlib.Path(f"/proc/{pid}/stat").read_text().rsplit(") ",1)[1].split()[11:13])
               for pid in proc_ids)

def terminate(process):
    if process.poll() is None:
        process.terminate()
        process.wait(timeout=4)

def windows(title, pid):
    result = sp.run(["xdotool","search","--onlyvisible","--name",title],text=True,capture_output=True)
    return [w for w in result.stdout.split()
        if sp.run(["xdotool","getwindowpid",w],text=True,capture_output=True).stdout.strip()==str(pid)]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", type=pathlib.Path, default=ROOT/"target/release")
    parser.add_argument("--warm", type=float, default=30)
    parser.add_argument("--cpu", type=float, default=60)
    parser.add_argument("--runs", type=int, default=3)
    args = parser.parse_args()
    directory = args.bin.resolve()
    existing = pids(directory)
    name = json.loads((ROOT/"data/project.json").read_text())["name"]
    full = []
    for index in range(args.runs):
        samples = {}
        with tempfile.TemporaryDirectory(prefix="ot-measure-") as d, contextlib.ExitStack() as stack:
            folder = pathlib.Path(d)
            pulse = os.environ.get("PULSE_SERVER",f"unix:{os.environ['XDG_RUNTIME_DIR']}/pulse/native")
            env = dict(os.environ,XDG_RUNTIME_DIR=d,XDG_CONFIG_HOME=str(folder/"config"),
                       XDG_DATA_HOME=str(folder/"data"),PULSE_SERVER=pulse,GSK_RENDERER="cairo")
            def ctl(*parts):
                return json.loads(sp.check_output([directory/"overtone-ctl",*map(str,parts)],env=env,text=True,stderr=sp.DEVNULL))
            a,b = folder/"A.flac",folder/"B.flac"
            for hz,path in [(440,a),(880,b)]:
                sp.run(["ffmpeg","-nostdin","-v","error","-f","lavfi","-i",f"sine=frequency={hz}:duration=120",
                    "-metadata",f"title=Контрольная запись {hz} Гц", "-c:a","flac",path],check=True)
            # Same audio options and MPRIS plugin in both mpv processes.
            plain_socket = folder/"plain.sock"
            options = ["--no-config","--idle=yes","--no-video","--audio-display=no","--volume=35",
                       "--ytdl=no","--load-scripts=no","--no-terminal","--mute=yes",
                       "--script=/usr/lib/mpv-mpris/mpris.so"]
            plain = sp.Popen(["mpv",*options,"--pause=yes",f"--input-ipc-server={plain_socket}","--",b],env=env)
            stack.callback(terminate,plain)
            retry(lambda: ipc(plain_socket,"get_property","media-title"),"Нет контрольного mpv")
            time.sleep(args.warm)
            samples["standalone_mpv"] = proc_kb(plain.pid)
            terminate(plain)
            mpv = sp.Popen([directory/"overtone-ctl","mpv"],env=env,stdout=sp.DEVNULL,stderr=sp.DEVNULL)
            stack.callback(terminate,mpv)
            daemon = sp.Popen([directory/"overtone-background"],env=env)
            stack.callback(terminate,daemon)
            apath = folder/"overtone"/f"mpv-{mpv.pid}.sock"
            retry(lambda: ctl("status")["state"]["adapter"],"Нет mpv с адаптером")
            ctl("rule",a,b)
            ipc(apath,"set_property","mute",True)
            ctl("queue",a)
            retry(lambda: ctl("status")["state"]["adapter"]["load"]["state"]=="applied","Нет подтверждения замены")
            ctl("play-pause")
            retry(lambda: ctl("status")["state"]["player"]["status"]=="Paused","Нет паузы")
            time.sleep(args.warm)
            samples["background"] = proc_kb(daemon.pid)
            samples["adapted_mpv"] = proc_kb(mpv.pid)
            samples["combined_background"] = aggregate([daemon.pid,mpv.pid])
            samples["rule_bytes"] = (folder/"config/overtone/settings.json").stat().st_size
            if index == 0:
                before = ticks([daemon.pid,mpv.pid])
                own_before = ticks([daemon.pid])
                begin = time.monotonic()
                time.sleep(args.cpu)
                duration = time.monotonic() - begin
                multiplier = 100 / os.sysconf("SC_CLK_TCK") / duration
                samples["paused_cpu_percent"] = (ticks([daemon.pid]) - own_before) * multiplier
                samples["combined_paused_cpu_percent"] = (ticks([daemon.pid,mpv.pid]) - before) * multiplier
            for mode,title,flags in [
                ("mini",f"^{name} · Мини-плеер$",["--mini"]),
                ("manager",f"^{name}$",[]),
            ]:
                ui = sp.Popen([directory/"overtone",*flags],env=env)
                stack.callback(terminate,ui)
                win = retry(lambda: next(iter(windows(title,ui.pid)),None),f"Нет окна {mode}")
                time.sleep(args.warm)
                samples[mode] = aggregate([ui.pid,daemon.pid])
                samples[f"combined_{mode}"] = aggregate([ui.pid,daemon.pid,mpv.pid])
                sp.run(["xdotool","windowactivate","--sync",win,"key","Alt+F4"],check=True)
                ui.wait(timeout=5)
            ctl("quit")
            daemon.wait(timeout=5)
            samples["own_processes_after_exit"] = len(pids(directory)-existing)
            full.append(samples)
            print(json.dumps({"run":index+1,**samples},ensure_ascii=False),flush=True)
    print(json.dumps(full,ensure_ascii=False),flush=True)

if __name__ == "__main__":
    main()
