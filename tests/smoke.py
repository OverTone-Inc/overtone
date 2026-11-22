#!/usr/bin/env python3
"""Smoke test using real mpv, D-Bus and the PipeWire/Pulse stream monitor."""
import array
import contextlib
import json
import math
import os
import pathlib
import socket
import subprocess as sp
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]

def retry(read, message):
    until = time.monotonic() + 8
    while time.monotonic() < until:
        try:
            result = read()
            if result:
                return result
        except (OSError, sp.CalledProcessError, KeyError, TypeError):
            pass
        time.sleep(0.10)
    raise AssertionError(message)

def ipc(path, *command):
    with socket.socket(socket.AF_UNIX) as conn:
        conn.settimeout(2)
        conn.connect(str(path))
        conn.sendall(json.dumps({"command": command}).encode() + b"\n")
        response = json.loads(conn.makefile().readline())
        assert response["error"] == "success", response
        return response.get("data")

def frequency(pid):
    def input_stream():
        streams = json.loads(sp.check_output(["pactl", "-f", "json", "list", "sink-inputs"]))
        return next((s for s in streams if s["properties"].get("application.process.id") == str(pid)), None)
    stream = retry(input_stream, "Нет аудиопотока mpv")
    sinks = json.loads(sp.check_output(["pactl", "-f", "json", "list", "sinks"]))
    monitor = next(s["monitor_source"] for s in sinks if s["index"] == stream["sink"])
    with sp.Popen(["parec", "--raw", "--latency-msec=100", "--rate=48000", "--format=s16le", "--channels=1",
                   f"--device={monitor}"], stdout=sp.PIPE) as rec:
        try:
            pcm, _ = rec.communicate(timeout=1.6)
        except sp.TimeoutExpired:
            rec.terminate()
            pcm, _ = rec.communicate(timeout=2)
        print(f"Захвачено {len(pcm)} байт из {monitor}",flush=True)
    samples = array.array("h", pcm[:len(pcm)//2*2])[-24000:]
    assert len(samples) > 8000, "Нет PCM в monitor stream"
    rms = math.sqrt(sum(x * x for x in samples) / len(samples))
    assert rms > 20, ("Сигнал слишком тихий", rms)
    def score(hz):
        angle = 2 * math.pi * hz / 48000
        return abs(sum(x * complex(math.cos(i * angle), math.sin(i * angle)) for i, x in enumerate(samples)))
    hz = max([440, 660, 880], key=score)
    print(f"PipeWire monitor-stream → {hz} Гц, RMS {rms:.1f}", flush=True)
    return hz

def main():
    bin_dir = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / "target/release").resolve()
    ctl = bin_dir / "overtone-ctl"
    with tempfile.TemporaryDirectory(prefix="ot-smoke-") as tmp, contextlib.ExitStack() as stack:
        folder = pathlib.Path(tmp)
        env = os.environ.copy()
        env["PULSE_SERVER"] = env.get("PULSE_SERVER", f"unix:{env['XDG_RUNTIME_DIR']}/pulse/native")
        env.update(XDG_RUNTIME_DIR=tmp, XDG_CONFIG_HOME=str(folder/"config"), XDG_DATA_HOME=str(folder/"data"))
        sink = f"overtone_test_{os.getpid()}"
        module = sp.check_output(["pactl","load-module","module-null-sink",f"sink_name={sink}"],text=True).strip()
        stack.callback(sp.run,["pactl","unload-module",module],check=True)
        env["PULSE_SINK"] = sink
        def cmd(*args):
            return sp.check_output([ctl, *map(str,args)],env=env,text=True,stderr=sp.PIPE)
        def status():
            return json.loads(cmd("status"))["state"]
        def cleanup(process):
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
        def tone(hz, name):
            path = folder/f"{name}.flac"
            sp.run(["ffmpeg","-nostdin","-v","error","-f","lavfi","-i",f"sine=frequency={hz}:sample_rate=48000:duration=25",
                    "-metadata",f"title=Версия {name} · {hz} Гц","-metadata","artist=Overtone · тестовый сигнал","-c:a","flac",path],check=True)
            return str(path)
        a,b,c = tone(440,"A"),tone(880,"B"),tone(660,"C")
        second = sp.Popen(["mpv","--no-config","--idle=yes","--no-video","--no-terminal","--mute=yes",f"--input-ipc-server={folder/'plain.sock'}","--script=/usr/lib/mpv-mpris/mpris.so"],env=env)
        stack.callback(cleanup,second)
        mpv = sp.Popen([ctl,"mpv"],env=env)
        stack.callback(cleanup,mpv)
        daemon = sp.Popen([bin_dir/"overtone-background"],env=env)
        stack.callback(cleanup,daemon)
        apath = folder/"overtone"/f"mpv-{mpv.pid}.sock"
        player = retry(lambda: next((n for n in cmd("probe").splitlines() if json.loads(cmd("probe",n))["pid"] == mpv.pid), None),"MPRIS mpv не зарегистрирован")
        state = retry(lambda: (s if len((s:=status())["players"]) == 2 else None), "Требуются два плеера")
        assert state["player"] is None
        cmd("select",player)
        state = retry(lambda: status()["adapter"],"Нет адаптера")
        assert state["supported"], state
        print("Два экземпляра MPRIS · выбран только mpv с адаптером",state["version"],flush=True)
        cmd("rule",a,b)
        cmd("toggle","history","true")
        cmd("queue",a,c)
        def is_load(expected, key):
            return retry(lambda: (load if (load:=(status()["adapter"] or {})["load"])[key] == expected else None),f"Нет подтверждения: {key} {expected}")
        is_load("applied","state")
        assert ipc(apath,"get_property","stream-open-filename") == b
        assert [t["filename"] for t in ipc(apath,"get_property","playlist")] == [a,c]
        assert frequency(mpv.pid) == 880
        print("A → B до старта · очередь осталась A, C", flush=True)
        cmd("preview")
        is_load("original","state")
        assert frequency(mpv.pid) == 440
        cmd("preview",b)
        is_load("applied","state")
        assert frequency(mpv.pid) == 880
        print("Предпрослушивание и возврат оригинала работают в той же позиции", flush=True)
        cmd("play-pause")
        retry(lambda: status()["player"]["status"] == "Paused","Нет паузы")
        cmd("play-pause")
        retry(lambda: status()["player"]["status"] == "Playing","Нет продолжения")
        cmd("next")
        is_load(c,"original")
        assert frequency(mpv.pid) == 660
        hist = status()["history"]
        assert sum(h["replacement"] == "B.flac" for h in hist) == 2, hist
        cmd("rule-enable",a,"false")
        cmd("queue",a,c)
        is_load("original","state")
        assert frequency(mpv.pid) == 440
        cmd("rule",a,str(folder/"missing.flac"))
        cmd("queue",a,c)
        is_load("fallback","state")
        assert frequency(mpv.pid) == 440
        cmd("timer", "2")
        retry(lambda: status()["player"]["status"] == "Paused", "Таймер не поставил паузу")
        assert status()["timer"] is None
        print("Следующий C · выключенное правило слышно как A · недоступный файл упал обратно на A · таймер поставил паузу", flush=True)
        answer = sp.run([ctl,"import",folder/"missing.json"],env=env,capture_output=True)
        assert answer.returncode
        saved = status()["manual"]
        broken = folder/"broken.json"
        broken.write_text("{")
        assert sp.run([ctl,"import",broken],env=env,capture_output=True).returncode
        assert status()["manual"] == saved
        catalog = folder/"valid.json"
        catalog.write_text(json.dumps({"schema": 1, "rules":[
            {"from":{"type":"local","value":c},"to":{"type":"local","value":b},"enabled":True}
        ]}))
        ready = json.loads(cmd("import",catalog))
        assert ready["import"] == {"accepted": 1, "skipped": 0, "conflicts": 0}
        cmd("import-apply",ready["ticket"])
        # Failed HTTPS request keeps the previous working copy.
        assert sp.run([ctl,"fetch","https://127.0.0.1:9/catalog"],env=env,capture_output=True).returncode
        assert len(status()["catalog"]) == 1
        cmd("play-pause")
        cmd("timer","4")
        cleanup(mpv)
        second_socket = folder/"plain.sock"
        ipc(second_socket,"loadfile",c)
        other = retry(lambda: next((row["name"] for row in status()["players"] if row["name"] != player),None),"Не найден второй плеер")
        cmd("select",other)
        retry(lambda: status()["timer"] is None, "Таймер не завершился")
        assert status()["player"]["status"] == "Playing"
        print("После закрытия выбранного плеера таймер не поставил новый экземпляр на паузу", flush=True)
        cmd("quit")
        daemon.wait(timeout=5)
        assert not (folder/"overtone"/f"mpv-{mpv.pid}.json").exists()
        again = sp.Popen([bin_dir/"overtone-background"],env=env)
        stack.callback(cleanup,again)
        assert retry(lambda: status()["manual"],"Нет сохранённого правила") == saved
        assert len(status()["catalog"]) == 1
        cmd("quit")
        again.wait(timeout=5)
        print("Битый импорт и недоступный HTTPS сохранили данные · настройки пережили перезапуск · полный выход завершил фоновый процесс", flush=True)
if __name__ == "__main__":
    main()
