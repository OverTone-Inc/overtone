use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use url::Url;

pub const PROJECT: &str = include_str!("../../../data/project.json");
pub const CATALOG_LIMIT: usize = 1024 * 1024;
pub const MAX_RULES: usize = 4096;
pub const HISTORY_LIMIT: usize = 600;
pub const ADAPTER_VERSION: u64 = 1;
pub const COMPATIBILITY: &str = include_str!("../../../data/compatibility.json");

#[derive(Deserialize)]
pub struct Project {
    pub name: String,
    pub id: String,
}
pub fn project() -> Project {
    serde_json::from_str(PROJECT).expect("project.json")
}
#[derive(Clone, Debug, Deserialize)]
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub title: String,
    pub version: String,
    pub description: String,
    pub requires: String,
}
pub fn plugins() -> Vec<Plugin> {
    serde_json::from_str(include_str!("../../../data/plugins.json")).expect("plugins.json")
}
#[derive(Clone, Debug, Deserialize)]
pub struct Compatibility {
    pub client: String,
    pub version: String,
    pub control: String,
    pub replacement: String,
    pub queue: String,
    pub date: String,
}
pub fn compatibility() -> Vec<Compatibility> {
    serde_json::from_str(COMPATIBILITY).expect("compatibility.json")
}
fn xdg(name: &str, fallback: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback)
        })
        .join("overtone")
}
pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
}
pub fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share")
}
pub fn runtime_dir() -> Result<PathBuf> {
    let path = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .context("Не задан XDG_RUNTIME_DIR")?
        .join("overtone");
    private_dir(&path)?;
    Ok(path)
}
pub fn private_dir(path: &Path) -> Result<()> {
    fs::DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(path)?;
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.permissions().mode() & 0o077 != 0 {
        bail!("Каталог Overtone должен быть доступен только владельцу");
    }
    Ok(())
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn read_limited(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        bail!("Требуется обычный файл");
    }
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        bail!("Превышен лимит размера");
    }
    Ok(bytes)
}
pub fn read_json<T: for<'a> Deserialize<'a>>(path: &Path, limit: usize) -> Result<T> {
    Ok(serde_json::from_slice(&read_limited(path, limit)?)?)
}
pub fn atomic(path: &Path, contents: &[u8]) -> Result<()> {
    private_dir(path.parent().context("Не задан каталог файла")?)?;
    atomic_in_user_dir(path, contents)
}
pub fn atomic_in_user_dir(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().context("Не задан каталог файла")?;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let _ = fs::remove_file(&tmp);
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        File::open(parent)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    Ok(result?)
}
pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    atomic(path, &serde_json::to_vec(value)?)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Source {
    Local(String),
    Yandex(String),
    Audio(String),
}
impl Source {
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.len() > 2048 {
            bail!("Слишком длинная ссылка");
        }
        if Path::new(input).is_absolute() {
            return Self::file(Path::new(input));
        }
        let url = Url::parse(input).context("Введите абсолютный путь, file:// или ссылку HTTPS")?;
        if !url.username().is_empty() || url.password().is_some() {
            bail!("Ссылки с учётными данными не поддерживаются");
        }
        if url.scheme() == "file" {
            return Self::file(
                &url.to_file_path()
                    .map_err(|_| anyhow::anyhow!("Ожидается локальный файл"))?,
            );
        }
        if url.scheme() != "https" || !url.has_host() {
            bail!("Для сетевого источника требуется HTTPS");
        }
        if matches!(
            url.host_str(),
            Some("music.yandex.ru" | "music.yandex.com" | "music.yandex.kz" | "music.yandex.by")
        ) {
            let p: Vec<_> = url.path().split('/').filter(|s| !s.is_empty()).collect();
            let id = match p.as_slice() {
                ["track", id] | ["album", _, "track", id] => *id,
                _ => bail!("Укажите ссылку на одну запись Яндекс Музыки"),
            };
            if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
                bail!("Ссылка не содержит числовой ID записи");
            }
            return Ok(Self::Yandex(format!("https://music.yandex.ru/track/{id}")));
        }
        let mut url = url;
        url.set_fragment(None);
        Ok(Self::Audio(url.to_string()))
    }
    fn file(path: &Path) -> Result<Self> {
        let mut normalized = PathBuf::from("/");
        for part in path.components() {
            match part {
                std::path::Component::ParentDir => {
                    normalized.pop();
                }
                std::path::Component::Normal(p) => normalized.push(p),
                _ => (),
            }
        }
        if !path.is_absolute() || normalized == Path::new("/") || normalized.to_str().is_none() {
            bail!("Требуется абсолютный путь к файлу UTF‑8");
        }
        Ok(Self::Local(normalized.to_str().unwrap().into()))
    }
    pub fn value(&self) -> &str {
        match self {
            Self::Local(s) | Self::Yandex(s) | Self::Audio(s) => s,
        }
    }
    pub fn uri(&self) -> String {
        match self {
            Self::Local(p) => Url::from_file_path(p)
                .map(|u| u.to_string())
                .unwrap_or_default(),
            _ => self.value().into(),
        }
    }
    pub fn label(&self) -> String {
        match self {
            Self::Local(p) => Path::new(p)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            Self::Yandex(s) => format!(
                "Яндекс Музыка · {}",
                s.rsplit('/').next().unwrap_or_default()
            ),
            Self::Audio(s) => Url::parse(s)
                .ok()
                .and_then(|u| u.host_str().map(str::to_owned))
                .unwrap_or_default(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub from: Source,
    pub to: Option<Source>,
    pub enabled: bool,
}
impl Rule {
    pub fn normalized(&self) -> Result<Self> {
        let normalize = |source: &Source| -> Result<Source> {
            let normalized = Source::parse(source.value())?;
            if std::mem::discriminant(source) != std::mem::discriminant(&normalized) {
                bail!("Тип источника не совпадает с указанным");
            }
            Ok(normalized)
        };
        let from = normalize(&self.from)?;
        let to = self.to.as_ref().map(normalize).transpose()?;
        if to.as_ref() == Some(&from) {
            bail!("Оригинал и замена совпадают");
        }
        Ok(Self {
            from,
            to,
            enabled: self.enabled,
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub schema: u32,
    pub selected: String,
    pub plugins: BTreeMap<String, bool>,
    pub manual: Vec<Rule>,
    pub catalog: Vec<Rule>,
    pub catalog_url: String,
    pub catalog_minutes: u32,
    pub catalog_checked: u64,
    pub light: bool,
    pub above: bool,
    pub autostart: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            schema: 1,
            selected: String::new(),
            plugins: [("history".into(), false)].into(),
            manual: vec![],
            catalog: vec![],
            catalog_url: String::new(),
            catalog_minutes: 0,
            catalog_checked: 0,
            light: false,
            above: false,
            autostart: false,
        }
    }
}
impl Settings {
    pub fn enabled(&self, id: &str) -> bool {
        self.plugins.get(id).copied().unwrap_or(true)
    }
    pub fn rule(&self, uri: &Source) -> Option<(&Rule, &'static str)> {
        self.manual
            .iter()
            .find(|r| &r.from == uri)
            .map(|r| (r, "Ручное правило"))
            .or_else(|| {
                self.catalog
                    .iter()
                    .find(|r| &r.from == uri)
                    .map(|r| (r, "Импортированный каталог"))
            })
    }
    pub fn resolve(&self, uri: &Source) -> Option<(&Source, &'static str)> {
        let (rule, origin) = self.rule(uri)?;
        let to = rule.to.as_ref().filter(|_| rule.enabled)?;
        let mut seen = HashSet::from([uri.value()]);
        let mut cursor = to;
        loop {
            if !seen.insert(cursor.value()) {
                return None;
            }
            let Some(next) = self
                .rule(cursor)
                .and_then(|(r, _)| r.to.as_ref().filter(|_| r.enabled))
            else {
                break;
            };
            cursor = next;
        }
        // Each load uses the first edge only. The adapter opens that stream directly.
        Some((to, origin))
    }
    pub fn effective_rules(&self) -> Vec<(&Source, &Source, &'static str)> {
        let rules: HashMap<_, _> = self
            .catalog
            .iter()
            .map(|rule| (rule.from.value(), (rule, "Импортированный каталог")))
            .chain(
                self.manual
                    .iter()
                    .map(|rule| (rule.from.value(), (rule, "Ручное правило"))),
            )
            .collect();
        let mut safe = HashMap::new();
        let mut effective = Vec::new();
        for (id, (rule, origin)) in &rules {
            let Some(to) = rule.to.as_ref().filter(|_| rule.enabled) else {
                continue;
            };
            let mut seen = HashSet::new();
            let mut cursor = *id;
            let is_safe = loop {
                if let Some(known) = safe.get(cursor) {
                    break *known;
                }
                if !seen.insert(cursor) {
                    break false;
                }
                let Some(next) = rules
                    .get(cursor)
                    .and_then(|(rule, _)| rule.to.as_ref().filter(|_| rule.enabled))
                else {
                    break true;
                };
                cursor = next.value();
            };
            safe.extend(seen.into_iter().map(|id| (id, is_safe)));
            if is_safe {
                effective.push((&rule.from, to, *origin));
            }
        }
        effective
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ImportSummary {
    pub accepted: usize,
    pub skipped: usize,
    pub conflicts: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    schema: u32,
    rules: Vec<serde_json::Value>,
}
pub fn preview_import(
    bytes: &[u8],
    settings: &Settings,
    remote: bool,
) -> Result<(ImportSummary, Vec<Rule>)> {
    if bytes.len() > CATALOG_LIMIT {
        bail!("Каталог больше 1 МиБ");
    }
    let raw: Catalog =
        serde_json::from_slice(bytes).context("Ожидается каталог Overtone формата JSON v1")?;
    if raw.schema != 1 || raw.rules.len() > MAX_RULES {
        bail!("Версия формата или размер каталога не поддерживаются");
    }
    let mut stats = ImportSummary::default();
    let mut seen = HashSet::new();
    let mut accepted = Vec::new();
    for value in raw.rules {
        let rule = serde_json::from_value::<Rule>(value)
            .ok()
            .and_then(|r| r.normalized().ok());
        let Some(rule) = rule.filter(|r| {
            !remote
                || !matches!(r.to, Some(Source::Local(_))) && !matches!(r.from, Source::Local(_))
        }) else {
            stats.skipped += 1;
            continue;
        };
        if settings.manual.iter().any(|r| r.from == rule.from) || !seen.insert(rule.from.uri()) {
            stats.conflicts += 1;
            continue;
        }
        accepted.push(rule);
    }
    stats.accepted = accepted.len();
    Ok((stats, accepted))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timer {
    pub name: String,
    pub owner: String,
    pub deadline: u64,
}
impl Timer {
    pub fn due(&self, now: u64, selected: Option<(&str, &str)>) -> Option<bool> {
        (now >= self.deadline).then(|| selected == Some((&self.name, &self.owner)))
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listening {
    pub title: String,
    pub artist: String,
    pub started: u64,
    pub seconds: u64,
    pub replacement: Option<String>,
}
#[derive(Default)]
pub struct Recorder {
    current: Option<(
        String,
        Listening,
        Option<std::time::Instant>,
        std::time::Duration,
    )>,
}
impl Recorder {
    pub fn update(&mut self, id: &str, info: Listening, playing: bool) -> Option<Listening> {
        let finished = if self.current.as_ref().is_some_and(|(k, _, _, _)| k != id) {
            self.finish()
        } else {
            None
        };
        if id.is_empty() {
            return finished;
        }
        if self.current.is_none() && playing {
            self.current = Some((id.into(), info, None, Default::default()));
        }
        if let Some((_, _, since, duration)) = self.current.as_mut() {
            if playing && since.is_none() {
                *since = Some(std::time::Instant::now());
            }
            if !playing && let Some(t) = since.take() {
                *duration += t.elapsed();
            }
        }
        finished
    }
    pub fn finish(&mut self) -> Option<Listening> {
        let (_, mut info, since, duration) = self.current.take()?;
        info.seconds = (duration + since.map(|t| t.elapsed()).unwrap_or_default()).as_secs();
        (info.seconds > 0).then_some(info)
    }
    pub fn reset(&mut self) {
        self.current = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn path(s: &str) -> Source {
        Source::parse(&format!("/tmp/{s}.flac")).unwrap()
    }
    fn rule(a: &str, b: Option<&str>, enabled: bool) -> Rule {
        Rule {
            from: path(a),
            to: b.map(path),
            enabled,
        }
    }
    #[test]
    fn manual_priority_and_cycle() {
        let mut s = Settings {
            catalog: vec![rule("a", Some("b"), true)],
            ..Settings::default()
        };
        assert_eq!(s.resolve(&path("a")).unwrap().0, &path("b"));
        s.manual.push(rule("a", None, true));
        assert!(s.resolve(&path("a")).is_none());
        s.manual[0] = rule("a", Some("c"), false);
        assert!(s.resolve(&path("a")).is_none());
        s.manual[0].enabled = true;
        assert_eq!(s.resolve(&path("a")).unwrap().0, &path("c"));
        s.manual.push(rule("c", Some("a"), true));
        assert!(s.resolve(&path("a")).is_none());
        assert!(s.effective_rules().is_empty());
        s.manual.pop();
        assert_eq!(
            s.effective_rules(),
            vec![(&path("a"), &path("c"), "Ручное правило")]
        );
    }
    #[test]
    fn import_bad_or_conflicting() {
        let s = Settings {
            manual: vec![rule("a", None, true)],
            ..Settings::default()
        };
        assert!(preview_import(b"{", &s, false).is_err());
        assert!(preview_import(br#"{"schema":2,"rules":[]}"#, &s, false).is_err());
        let bytes = serde_json::to_vec(&serde_json::json!({"schema":1,"rules":[serde_json::json!(rule("a",Some("b"),true)),serde_json::json!(rule("b",Some("c"),true)),serde_json::json!({"different":true})]})).unwrap();
        let (report, list) = preview_import(&bytes, &s, false).unwrap();
        assert_eq!(
            (
                report.accepted,
                report.skipped,
                report.conflicts,
                list.len()
            ),
            (1, 1, 1, 1)
        );
        assert_eq!(s.manual.len(), 1);
        assert!(preview_import(&bytes, &s, true).unwrap().1.is_empty());
    }
    #[test]
    fn timer_after_resume_and_player_restart() {
        let t = Timer {
            deadline: 20,
            name: "p".into(),
            owner: ":1.1".into(),
        };
        assert_eq!(t.due(19, Some(("p", ":1.1"))), None);
        assert_eq!(t.due(200, Some(("p", ":1.2"))), Some(false));
        assert_eq!(t.due(200, Some(("p", ":1.1"))), Some(true));
        assert_eq!(t.due(200, None), Some(false));
    }
    #[test]
    fn identical_metadata_is_one_listening() {
        let mut r = Recorder::default();
        r.update(
            "track",
            Listening {
                title: "A".into(),
                ..Listening::default()
            },
            true,
        );
        r.current.as_mut().unwrap().2 =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(2));
        assert!(r.update("track", Listening::default(), true).is_none());
        assert_eq!(
            r.update("repeat", Listening::default(), true)
                .unwrap()
                .seconds,
            2
        );
        assert_eq!(r.current.unwrap().0, "repeat");
    }
    #[test]
    fn yandex_url_identity_and_uri_type() {
        assert_eq!(
            Source::parse("https://music.yandex.ru/album/18/track/94?from=x").unwrap(),
            Source::parse("https://music.yandex.com/track/94").unwrap()
        );
        assert!(Source::parse("https://music.yandex.ru/album/94").is_err());
        assert!(Source::parse("https://user:pw@example.org/track/94").is_err());
    }
}
