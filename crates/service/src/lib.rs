use anyhow::{Context, Result, bail};
use overtone_core::{ImportSummary, Listening, Rule, Source, Timer, runtime_dir};
use overtone_player::{Player, mpv};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

pub mod daemon;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub name: String,
    pub title: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct State {
    pub players: Vec<Item>,
    pub selected: String,
    pub player: Option<Player>,
    pub version: String,
    pub adapter: Option<mpv::Adapter>,
    pub last_replacement: Option<mpv::Load>,
    pub plugins: std::collections::BTreeMap<String, bool>,
    pub manual: Vec<Rule>,
    pub catalog: Vec<Rule>,
    pub timer: Option<Timer>,
    pub history: Vec<Listening>,
    pub light: bool,
    pub above: bool,
    pub autostart: bool,
    pub catalog_url: String,
    pub catalog_minutes: u32,
    pub catalog_checked: u64,
    pub action: String,
    pub error: String,
}
impl State {
    pub fn enabled(&self, id: &str) -> bool {
        self.plugins.get(id).copied().unwrap_or(true)
    }
    pub fn adapted(&self) -> bool {
        self.adapter.as_ref().is_some_and(|a| a.supported)
    }
    pub fn original(&self) -> Option<Source> {
        self.adapter
            .as_ref()
            .and_then(|a| Source::parse(&a.load.original).ok())
            .or_else(|| {
                self.player
                    .as_ref()
                    .and_then(|p| Source::parse(&p.track.uri).ok())
            })
    }
    pub fn diagnostic(&self) -> String {
        let mut lines = vec![format!(
            "{} {}",
            overtone_core::project().name,
            env!("CARGO_PKG_VERSION")
        )];
        lines.push(format!(
            "Сессионная шина · {} плееров MPRIS",
            self.players.len()
        ));
        if let Some(p) = &self.player {
            let family = if self.adapter.is_some() || p.identity == "mpv" {
                "mpv"
            } else if p.identity == "YandexMusic" {
                "Яндекс Музыка"
            } else {
                "Другой клиент MPRIS"
            };
            lines.push(format!("{family} · {}", self.version));
            lines.push(format!(
                "Управление {} · Пуск {} · Пауза {} · Следующий {} · TrackList {}",
                yes(p.control),
                yes(p.play),
                yes(p.pause),
                yes(p.next),
                yes(p.has_tracklist)
            ));
            let known = [
                "file", "http", "https", "ftp", "mms", "rtmp", "rtsp", "sftp", "smb",
            ];
            let schemes: Vec<_> = known
                .into_iter()
                .filter(|s| p.schemes.iter().any(|v| v == s))
                .collect();
            lines.push(format!(
                "Проверяемые схемы URI · {}",
                if schemes.is_empty() {
                    "—".into()
                } else {
                    schemes.join(", ")
                }
            ));
            lines.push(format!(
                "Идентичность записи по URI · {}",
                yes(self.original().is_some())
            ));
        }
        lines.push(format!(
            "Адаптер mpv v{} · {}",
            overtone_core::ADAPTER_VERSION,
            if self.adapted() {
                "совместим"
            } else {
                "автозамена недоступна"
            }
        ));
        lines.push(format!(
            "Ручных правил {} · В каталоге {}",
            self.manual.len(),
            self.catalog.len()
        ));
        if !self.error.is_empty() {
            lines.push(format!("Последняя ошибка · {}", self.error));
        }
        lines.join("\n")
    }
}
fn yes(value: bool) -> &'static str {
    if value { "да" } else { "нет" }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Request {
    Status,
    Subscribe,
    Select {
        name: String,
    },
    Rescan,
    Toggle {
        id: String,
        enabled: bool,
    },
    PlayPause,
    Next,
    Timer {
        seconds: u64,
    },
    CancelTimer,
    SaveRule {
        from: String,
        to: Option<String>,
        enabled: bool,
    },
    RuleEnable {
        from: String,
        enabled: bool,
        catalog: bool,
    },
    DeleteRule {
        from: String,
    },
    Preview {
        to: Option<String>,
    },
    Open {
        to: String,
    },
    CloseMpv,
    LocalQueue {
        paths: Vec<String>,
    },
    Import {
        path: String,
    },
    Fetch {
        url: String,
    },
    ApplyImport {
        ticket: u64,
    },
    Catalog {
        url: String,
        minutes: u32,
    },
    ClearHistory,
    Preferences {
        light: bool,
        above: bool,
        autostart: bool,
    },
    Quit,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Response {
    pub state: Option<State>,
    pub import: Option<ImportSummary>,
    pub ticket: Option<u64>,
    pub error: Option<String>,
}
impl Response {
    pub fn checked(self) -> Result<Self> {
        if let Some(error) = &self.error {
            bail!("{error}");
        }
        Ok(self)
    }
}

const RESPONSE_LIMIT: usize = 24 * 1024 * 1024;
pub fn socket() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("service.sock"))
}
pub fn connect(request: &Request) -> Result<BufReader<UnixStream>> {
    let mut stream =
        UnixStream::connect(socket()?).context("Фоновый процесс Overtone недоступен")?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    writeln!(stream, "{}", serde_json::to_string(request)?)?;
    Ok(BufReader::new(stream))
}
pub fn receive(reader: &mut impl BufRead) -> Result<Option<Response>> {
    Ok(mpv::read_line(reader, RESPONSE_LIMIT)?
        .map(serde_json::from_value)
        .transpose()?)
}
pub fn call(request: &Request) -> Result<Response> {
    receive(&mut connect(request)?)?
        .context("Фоновый процесс закрыл соединение")?
        .checked()
}
pub fn ensure() -> Result<()> {
    let path = socket()?;
    if UnixStream::connect(&path).is_ok() {
        return Ok(());
    }
    let executable = std::env::current_exe()?.with_file_name("overtone-background");
    let mut child = Command::new(executable)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Требуется overtone-background рядом с исполняемым файлом")?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    for _ in 0..80 {
        if UnixStream::connect(&path).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    bail!("Фоновый процесс не ответил. Запустите overtone-background из терминала для диагностики")
}
