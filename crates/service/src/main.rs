use anyhow::{Context, Result, bail};
use overtone_core::{atomic, runtime_dir};
use overtone_player::{Bus, mpv};
use overtone_service::{Request, call, ensure};
use std::{
    io::{self, Write},
    os::unix::process::CommandExt,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

fn probe(mut args: impl Iterator<Item = String>) -> Result<()> {
    let bus = Bus::connect()?;
    let Some(name) = args.next() else {
        for name in bus.names()? {
            println!("{name}");
        }
        return Ok(());
    };
    let before = bus.inspect(&name)?;
    println!("{}", serde_json::to_string_pretty(&before)?);
    let Some(action) = args.next() else {
        return Ok(());
    };
    let uri = args.next();
    bus.action(&before, &action, uri.as_deref())?;
    let start = Instant::now();
    loop {
        let now = bus.inspect(&name)?;
        if now.owner != before.owner {
            bail!("Плеер сменился");
        }
        let confirmed = match action.as_str() {
            "OpenUri" => uri.as_deref() == Some(now.track.uri.as_str()) && now.status == "Playing",
            "PlayPause" | "Pause" => before.status != now.status,
            _ => before.track.object != now.track.object || before.track.uri != now.track.uri,
        };
        if confirmed {
            println!("{}", serde_json::to_string_pretty(&now)?);
            return Ok(());
        }
        if start.elapsed() >= Duration::from_secs(4) {
            bail!("Плеер не подтвердил изменение");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
fn launch_mpv() -> Result<()> {
    let runtime = runtime_dir()?;
    let script = runtime.join("overtone.lua");
    if std::fs::read(&script).as_deref().ok() != Some(mpv::SCRIPT) {
        atomic(&script, mpv::SCRIPT)?;
    }
    let pid = std::process::id();
    let plugin = Path::new("/usr/lib/mpv-mpris/mpris.so");
    if !plugin.is_file() {
        bail!("Установите пакет mpv-mpris для MPRIS");
    }
    let ipc = mpv::socket(&runtime, pid);
    let map = mpv::map(&runtime, pid);
    let _ = std::fs::remove_file(&ipc);
    let _ = std::fs::remove_file(&map);
    println!(
        "Запускается отдельный mpv · PID {pid}. Выберите его в Overtone и откройте локальную очередь."
    );
    io::stdout().flush()?;
    let err = Command::new("mpv")
        .args([
            "--no-config",
            "--idle=yes",
            "--no-video",
            "--audio-display=no",
            "--volume=35",
            "--ytdl=no",
            "--load-scripts=no",
            "--no-terminal",
        ])
        .arg(format!("--script={}", script.display()))
        .arg(format!("--script={}", plugin.display()))
        .arg(format!("--input-ipc-server={}", ipc.display()))
        .env("OVERTONE_MPV_MAP", map)
        .exec();
    Err(err.into())
}
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let mut arg = || args.next().context("Требуется аргумент");
    let request = match command.as_str() {
        "probe" => return probe(args),
        "mpv" => return launch_mpv(),
        "status" => Request::Status,
        "rescan" => Request::Rescan,
        "select" => Request::Select {
            name: args.next().unwrap_or_default(),
        },
        "play-pause" => Request::PlayPause,
        "next" => Request::Next,
        "mpv-close" => Request::CloseMpv,
        "timer" => Request::Timer {
            seconds: arg()?.parse()?,
        },
        "cancel-timer" => Request::CancelTimer,
        "rule" => Request::SaveRule {
            from: arg()?,
            to: args.next(),
            enabled: true,
        },
        "rule-enable" => Request::RuleEnable {
            from: arg()?,
            enabled: arg()?.parse()?,
            catalog: args.next().as_deref() == Some("catalog"),
        },
        "rule-delete" => Request::DeleteRule { from: arg()? },
        "toggle" => Request::Toggle {
            id: arg()?,
            enabled: arg()?.parse()?,
        },
        "preview" => Request::Preview { to: args.next() },
        "open" => Request::Open { to: arg()? },
        "queue" => Request::LocalQueue {
            paths: args.collect(),
        },
        "import" => Request::Import { path: arg()? },
        "fetch" => Request::Fetch { url: arg()? },
        "import-apply" => Request::ApplyImport {
            ticket: arg()?.parse()?,
        },
        "quit" => Request::Quit,
        "diagnostic" => {
            ensure()?;
            println!(
                "{}",
                call(&Request::Status)?
                    .state
                    .context("Фоновый процесс не вернул состояние")?
                    .diagnostic()
            );
            return Ok(());
        }
        _ => {
            println!(
                "Overtone\n\
            probe [имя MPRIS [PlayPause|OpenUri URI]] · проверить плеер напрямую\n\
            status | select [имя MPRIS] | rescan | diagnostic\n\
            mpv | mpv-close | queue /абсолютный/путь … | play-pause | next | open URI\n\
            rule ОРИГИНАЛ [ЗАМЕНА] | rule-enable ОРИГИНАЛ true|false [catalog] | rule-delete ОРИГИНАЛ\n\
            preview [ЗАМЕНА] | toggle ID true|false | timer СЕКУНДЫ | cancel-timer\n\
            import ПУТЬ | fetch URL | import-apply НОМЕР | quit"
            );
            return Ok(());
        }
    };
    if !matches!(request, Request::Quit) {
        ensure()?;
    }
    println!("{}", serde_json::to_string_pretty(&call(&request)?)?);
    Ok(())
}
