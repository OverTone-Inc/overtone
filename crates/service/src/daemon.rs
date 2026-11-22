use crate::{Item, Request, Response, State, socket};
use anyhow::{Context, Result, anyhow, bail};
use overtone_core::{self as core, Listening, Recorder, Rule, Settings, Source, Timer};
use overtone_player::{Bus, Player, Signal, mpv};
use serde_json::json;
use std::{
    fs::{self, OpenOptions},
    io::{BufReader, Read, Write},
    net::Shutdown,
    os::{
        fd::AsRawFd,
        unix::{
            fs::OpenOptionsExt,
            net::{UnixListener, UnixStream},
        },
    },
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender, SyncSender},
    },
    time::{Duration, Instant},
};
const CONFIG_LIMIT: usize = 18 * 1024 * 1024;
const REQUEST_LIMIT: usize = 72 * 1024;

enum Event {
    Signal(Signal),
    Mpv(u32, mpv::Load),
    Client(Request, Sender<Response>),
    Subscribe(SyncSender<Arc<Vec<u8>>>),
    Download(Result<Vec<u8>>, Option<Sender<Response>>),
    Shutdown,
}

struct Pending {
    owner: String,
    uri: Source,
    before: String,
    deadline: Instant,
}
struct Engine {
    settings: Settings,
    state: State,
    bus: Bus,
    runtime: PathBuf,
    send: Sender<Event>,
    clients: Vec<SyncSender<Arc<Vec<u8>>>>,
    mpv_subscription: Option<(u32, UnixStream)>,
    recorder: Recorder,
    repeat: u64,
    last_seek: Option<Instant>,
    present: u64,
    staged: Option<(u64, Vec<Rule>)>,
    downloading: bool,
    config_read_only: bool,
    retry_at: u64,
    pending: Option<Pending>,
    running: bool,
}
impl Engine {
    fn new(bus: Bus, runtime: PathBuf, send: Sender<Event>) -> Self {
        let mut state = State::default();
        let config = core::config_dir().join("settings.json");
        let settings = match core::read_json::<Settings>(&config, CONFIG_LIMIT) {
            Ok(settings) if settings.schema == 1 => settings,
            _ if !config.exists() => Settings::default(),
            _ => {
                state.error = "Настройки повреждены или используют новый формат. Сохраните копию settings.json и исправьте файл.".into();
                Settings::default()
            }
        };
        let config_read_only = !state.error.is_empty();
        state.history = core::read_json(&core::data_dir().join("history.json"), CONFIG_LIMIT)
            .unwrap_or_default();
        let mut this = Self {
            settings,
            state,
            bus,
            runtime,
            send,
            clients: vec![],
            mpv_subscription: None,
            recorder: Recorder::default(),
            repeat: 0,
            last_seek: None,
            present: 0,
            staged: None,
            downloading: false,
            config_read_only,
            retry_at: 0,
            pending: None,
            running: true,
        };
        this.sync_config();
        this
    }
    fn sync_config(&mut self) {
        self.state.selected.clone_from(&self.settings.selected);
        self.state.plugins = core::plugins()
            .into_iter()
            .map(|p| {
                let value = self.settings.enabled(&p.id);
                (p.id, value)
            })
            .collect();
        self.state.manual.clone_from(&self.settings.manual);
        self.state.catalog.clone_from(&self.settings.catalog);
        self.state.light = self.settings.light;
        self.state.above = self.settings.above;
        self.state.autostart = self.settings.autostart;
        self.state
            .catalog_url
            .clone_from(&self.settings.catalog_url);
        self.state.catalog_minutes = self.settings.catalog_minutes;
        self.state.catalog_checked = self.settings.catalog_checked;
    }
    fn save(&mut self, settings: Settings) -> Result<()> {
        if self.config_read_only {
            bail!(
                "Текущий файл настроек сохранён. Исправьте settings.json и перезапустите Overtone."
            );
        }
        let path = core::config_dir().join("settings.json");
        let data = serde_json::to_vec(&settings)?;
        if data.len() > CONFIG_LIMIT {
            bail!("Превышен допустимый размер настроек");
        }
        core::atomic(&path, &data).context("Сохранение настроек не удалось")?;
        self.settings = settings;
        self.sync_config();
        Ok(())
    }
    fn wire_state(&self) -> Arc<Vec<u8>> {
        let mut line =
            serde_json::to_vec(&json!({"state":self.state})).expect("serializable state");
        line.push(b'\n');
        Arc::new(line)
    }
    fn broadcast(&mut self) {
        if self.clients.is_empty() {
            return;
        }
        let state = self.wire_state();
        self.clients
            .retain(|client| match client.try_send(state.clone()) {
                Ok(()) => true,
                // Disconnect slow clients. They can reconnect and get the latest full state.
                Err(_) => false,
            });
    }
    fn rescan(&mut self) {
        let names = match self.bus.names() {
            Ok(names) => names,
            Err(_) => {
                self.state.error = "Сессионная шина MPRIS недоступна".into();
                return;
            }
        };
        let old_name = self.effective_name().map(str::to_owned);
        let previous = std::mem::take(&mut self.state.players);
        self.state.players = names
            .into_iter()
            .map(|name| {
                let title = previous
                    .iter()
                    .find(|p| p.name == name)
                    .map(|p| p.title.clone())
                    .or_else(|| self.bus.inspect(&name).ok().map(|p| p.identity))
                    .unwrap_or_else(|| name.clone());
                Item { name, title }
            })
            .collect();
        if old_name.as_deref() != self.effective_name() && self.pending.take().is_some() {
            self.state.action = "Открытие отменено · выбран другой плеер".into();
        }
        self.refresh(true);
    }
    fn effective_name(&self) -> Option<&str> {
        if self.settings.selected.is_empty() {
            return (self.state.players.len() == 1).then(|| self.state.players[0].name.as_str());
        }
        self.state
            .players
            .iter()
            .find(|p| p.name == self.settings.selected)
            .map(|p| p.name.as_str())
    }
    fn player(&self) -> Result<&Player> {
        self.state
            .player
            .as_ref()
            .context("Выберите подключённый плеер")
    }
    fn refresh(&mut self, force: bool) {
        let next = self
            .effective_name()
            .and_then(|name| self.bus.inspect(name).ok());
        let changed = self.state.player.as_ref().map(|p| (&p.name, &p.owner))
            != next.as_ref().map(|p| (&p.name, &p.owner));
        if changed {
            self.last_seek = None;
            self.repeat = 0;
            self.state.last_replacement = None;
        }
        self.state.player = next;
        if changed || force {
            let adapter = self
                .state
                .player
                .as_ref()
                .and_then(|p| mpv::detect(&self.runtime, p.pid));
            self.state.version = self
                .state
                .player
                .as_ref()
                .map(|p| {
                    adapter
                        .as_ref()
                        .map(|a| a.version.clone())
                        .unwrap_or_else(|| client_version(p))
                })
                .unwrap_or_default();
            if !self.state.enabled("versions")
                || changed
                || adapter.as_ref().map(|a| a.supported)
                    != self.state.adapter.as_ref().map(|a| a.supported)
            {
                self.detach_mpv();
                self.state.adapter = adapter;
                self.attach_mpv();
                self.publish_map();
            }
        }
        self.confirm_open();
        self.history();
    }
    fn attach_mpv(&mut self) {
        if !self.state.adapted()
            || (!self.state.enabled("versions") && !self.state.enabled("history"))
        {
            return;
        }
        let Some(pid) = self.state.player.as_ref().map(|p| p.pid) else {
            return;
        };
        let tx = self.send.clone();
        match mpv::subscribe(&self.runtime, pid, move |load| {
            let _ = tx.send(Event::Mpv(pid, load));
        }) {
            Ok(stream) => self.mpv_subscription = Some((pid, stream)),
            Err(_) => {
                self.state.error =
                    "Подписка на события mpv недоступна. Переподключите адаптер.".into()
            }
        }
    }
    fn detach_mpv(&mut self) {
        if let Some((pid, stream)) = self.mpv_subscription.take() {
            let _ = stream.shutdown(Shutdown::Both);
            let _ = fs::remove_file(mpv::map(&self.runtime, pid));
        }
    }
    fn publish_map(&mut self) {
        let Some(p) = &self.state.player else { return };
        let map = mpv::map(&self.runtime, p.pid);
        if !self.state.adapted() || !self.state.enabled("versions") {
            let _ = fs::remove_file(map);
            return;
        }
        let mut rules = std::collections::BTreeMap::new();
        for (from, to, origin) in self.settings.effective_rules() {
            if !matches!(from, Source::Yandex(_)) && !matches!(to, Source::Yandex(_)) {
                rules.insert(from.value(), json!({"to":to.value(),"origin":origin}));
            }
        }
        let pid = std::process::id();
        if let Some(start) = proc_start(pid)
            && let Ok(bytes) =
                serde_json::to_vec(&json!({"version":1,"pid":pid,"proc_start":start,"rules":rules}))
            && bytes.len() <= core::CATALOG_LIMIT
            && core::atomic(&map, &bytes).is_ok()
        {
            return;
        }
        let _ = fs::remove_file(map);
        self.state.error =
            "Не удалось подготовить правила для mpv. Проверьте размер списка и права каталога."
                .into();
    }
    fn confirm_open(&mut self) {
        let Some(pending) = self.pending.as_ref() else {
            return;
        };
        let result = match &self.state.player {
            Some(p) if p.owner != pending.owner => {
                Some(("Открытие отменено · экземпляр плеера сменился", false))
            }
            Some(p)
                if Source::parse(&p.track.uri).ok() == Some(pending.uri.clone())
                    && p.status == "Playing" =>
            {
                Some((
                    "Альтернативная запись открыта · плеер подтвердил URI и воспроизведение",
                    false,
                ))
            }
            Some(p) if p.track.uri != pending.before && !p.track.uri.is_empty() => {
                Some(("Открытие отменено · плеер перешёл на другую запись", false))
            }
            _ if Instant::now() >= pending.deadline => {
                Some(("Плеер не подтвердил открытие записи", true))
            }
            _ => None,
        };
        if let Some((message, error)) = result {
            self.pending = None;
            self.state.action = message.into();
            if error {
                self.state.error = message.into();
            }
        }
    }
    fn history(&mut self) {
        if !self.state.enabled("history") {
            self.recorder.reset();
            return;
        }
        let mut id = String::new();
        let mut info = Listening::default();
        let mut playing = false;
        if let Some(p) = &self.state.player {
            let load = self
                .state
                .adapter
                .as_ref()
                .filter(|a| a.supported)
                .map(|a| &a.load);
            if let Some(load) = load {
                if load.serial == 0
                    || matches!(
                        load.state.as_str(),
                        "loading" | "fallback_pending" | "ended" | "error"
                    )
                {
                    self.finish_history();
                    return;
                }
                id = format!("{}:{}:{}", p.owner, load.serial, self.repeat);
                if load.state == "applied" {
                    info.replacement = load
                        .target
                        .as_deref()
                        .and_then(|s| Source::parse(s).ok())
                        .map(|s| s.label());
                }
            } else if !p.track.uri.is_empty()
                || !p.track.object.is_empty() && !p.track.object.ends_with("/NoTrack")
            {
                id = format!(
                    "{}:{}:{}:{}",
                    p.owner, p.track.object, p.track.uri, self.repeat
                );
            }
            if p.status == "Stopped" {
                id.clear();
            }
            info.title = p.track.title.chars().take(320).collect();
            info.artist = p.track.artist.chars().take(200).collect();
            info.started = core::now();
            playing = p.status == "Playing" && !info.title.is_empty();
        }
        if let Some(record) = self.recorder.update(&id, info, playing) {
            self.add_history(record);
        }
    }
    fn finish_history(&mut self) {
        if let Some(record) = self.recorder.finish() {
            self.add_history(record);
        }
    }
    fn add_history(&mut self, record: Listening) {
        self.state.history.insert(0, record);
        self.state.history.truncate(core::HISTORY_LIMIT);
        if core::write_json(&core::data_dir().join("history.json"), &self.state.history).is_err() {
            self.state.error =
                "История не записана. Проверьте свободное место и права каталога.".into();
        }
    }
    fn tick(&mut self) {
        self.confirm_open();
        if let Some(timer) = &self.state.timer
            && core::now() >= timer.deadline
        {
            let timer = self.state.timer.take().expect("active timer");
            self.refresh(false);
            let selected = self
                .state
                .player
                .as_ref()
                .map(|p| (p.name.as_str(), p.owner.as_str()));
            self.state.action = if !timer.due(core::now(), selected).unwrap_or(false) {
                "Таймер завершён · выбранный экземпляр плеера сменился".into()
            } else if !self
                .state
                .player
                .as_ref()
                .is_some_and(|p| p.status == "Playing")
            {
                "Таймер завершён · плеер уже остановлен или на паузе".into()
            } else if self
                .player()
                .and_then(|p| self.bus.action(p, "Pause", None))
                .is_ok()
            {
                "Таймер завершён · отправлена команда паузы".into()
            } else {
                self.state.error = "Плеер отклонил паузу по таймеру".into();
                self.state.error.clone()
            };
        }
        if self.settings.catalog_minutes > 0
            && !self.downloading
            && !self.settings.catalog_url.is_empty()
            && core::now() >= self.catalog_due()
        {
            self.download(self.settings.catalog_url.clone(), None);
        }
    }
    fn catalog_due(&self) -> u64 {
        self.retry_at.max(
            self.settings
                .catalog_checked
                .saturating_add(self.settings.catalog_minutes as u64 * 60),
        )
    }
    fn download(&mut self, url: String, answer: Option<Sender<Response>>) {
        self.downloading = true;
        let tx = self.send.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Event::Download(fetch(&url), answer));
        });
    }
    fn import(&mut self, bytes: &[u8], remote: bool, background: bool) -> Result<Response> {
        let (summary, rules) = core::preview_import(bytes, &self.settings, remote)?;
        if background {
            if rules.is_empty() {
                bail!("Обновление пропущено · в полученном каталоге нет подходящих правил");
            }
            let mut settings = self.settings.clone();
            settings.catalog = rules
                .into_iter()
                .map(|mut rule| {
                    if let Some(old) = self.state.catalog.iter().find(|old| old.from == rule.from) {
                        rule.enabled = old.enabled && rule.enabled;
                    }
                    rule
                })
                .collect();
            settings.catalog_checked = core::now();
            self.save(settings)?;
            self.publish_map();
            self.state.action = format!(
                "Каталог обновлён · принято {}, пропущено {}, конфликтов {}",
                summary.accepted, summary.skipped, summary.conflicts
            );
            return Ok(Response::default());
        }
        self.present += 1;
        self.staged = Some((self.present, rules));
        Ok(Response {
            import: Some(summary),
            ticket: Some(self.present),
            ..Response::default()
        })
    }
    fn request(&mut self, request: Request) -> Result<Response> {
        match request {
            Request::Status => {
                return Ok(Response {
                    state: Some(self.state.clone()),
                    ..Response::default()
                });
            }
            Request::Rescan => self.rescan(),
            Request::Select { name } => {
                if !name.is_empty() && !self.state.players.iter().any(|p| p.name == name) {
                    bail!("Плеер больше не подключён");
                }
                let mut settings = self.settings.clone();
                settings.selected = name;
                self.save(settings)?;
                self.refresh(true);
            }
            Request::Toggle { id, enabled } => {
                if !core::plugins().iter().any(|p| p.id == id) {
                    bail!("Неизвестный плагин");
                }
                let mut settings = self.settings.clone();
                settings.plugins.insert(id.clone(), enabled);
                self.save(settings)?;
                match id.as_str() {
                    "timer" if !enabled => self.state.timer = None,
                    "history" if !enabled => self.recorder.reset(),
                    "versions" | "history" => {
                        self.detach_mpv();
                        self.attach_mpv();
                        self.publish_map();
                        self.history();
                    }
                    _ => (),
                }
            }
            Request::PlayPause => self.bus.action(self.player()?, "PlayPause", None)?,
            Request::Next => self.bus.action(self.player()?, "Next", None)?,
            Request::Timer { seconds } => {
                if !self.state.enabled("timer") || !(1..=86400).contains(&seconds) {
                    bail!("Таймер выключен или интервал вне диапазона 1 с – 24 ч.");
                }
                let p = self.player()?;
                if !p.control || !p.pause {
                    bail!("Плеер пока не объявил поддержку паузы");
                }
                self.state.timer = Some(Timer {
                    name: p.name.clone(),
                    owner: p.owner.clone(),
                    deadline: core::now().saturating_add(seconds),
                });
            }
            Request::CancelTimer => self.state.timer = None,
            Request::SaveRule { from, to, enabled } => {
                let rule = Rule {
                    from: Source::parse(&from)?,
                    to: to.as_deref().map(Source::parse).transpose()?,
                    enabled,
                }
                .normalized()?;
                let mut settings = self.settings.clone();
                if settings.manual.len() >= core::MAX_RULES
                    && !settings.manual.iter().any(|r| r.from == rule.from)
                {
                    bail!("Достигнут лимит ручных правил");
                }
                settings.manual.retain(|r| r.from != rule.from);
                settings.manual.push(rule);
                self.save(settings)?;
                self.publish_map();
            }
            Request::RuleEnable {
                from,
                enabled,
                catalog,
            } => {
                let from = Source::parse(&from)?;
                let mut settings = self.settings.clone();
                let rules = if catalog {
                    &mut settings.catalog
                } else {
                    &mut settings.manual
                };
                rules
                    .iter_mut()
                    .find(|r| r.from == from)
                    .context("Правило больше не существует")?
                    .enabled = enabled;
                self.save(settings)?;
                self.publish_map();
            }
            Request::DeleteRule { from } => {
                let from = Source::parse(&from)?;
                let mut settings = self.settings.clone();
                settings.manual.retain(|r| r.from != from);
                self.save(settings)?;
                self.publish_map();
            }
            Request::Preview { to } => {
                if !self.state.enabled("versions") {
                    bail!("Плагин версий выключен");
                }
                let adapter = self
                    .state
                    .adapter
                    .as_ref()
                    .filter(|a| a.supported)
                    .context("Предпрослушивание в очереди требует адаптер mpv")?;
                if adapter.load.serial == 0
                    || matches!(adapter.load.state.as_str(), "ended" | "error" | "loading")
                {
                    bail!("Сначала запустите исходный трек");
                }
                let target = to.as_deref().map(Source::parse).transpose()?;
                if matches!(target, Some(Source::Yandex(_))) {
                    bail!("mpv принимает локальные файлы и прямые аудиоссылки HTTPS");
                }
                self.mpv(json!([
                    "script-message-to",
                    "overtone",
                    "reload",
                    adapter.load.serial.to_string(),
                    target.as_ref().map(Source::value).unwrap_or(""),
                    "Предпрослушивание"
                ]))?;
            }
            Request::CloseMpv => {
                if self.state.adapter.is_none() {
                    bail!("Выберите отдельный mpv Overtone");
                }
                let p = self.player()?;
                mpv::send(&mpv::socket(&self.runtime, p.pid), p.pid, json!(["quit"]))?;
            }
            Request::LocalQueue { paths } => {
                if paths.is_empty() || paths.len() > 24 {
                    bail!("Выберите от 1 до 24 файлов");
                }
                let paths: Vec<_> = paths
                    .iter()
                    .map(|p| match Source::parse(p)? {
                        Source::Local(path) if std::path::Path::new(&path).is_file() => Ok(path),
                        _ => bail!("Требуется доступный локальный файл"),
                    })
                    .collect::<Result<_>>()?;
                if self.state.adapter.is_none() {
                    bail!("Сначала подключите отдельный mpv Overtone");
                }
                for (index, path) in paths.iter().enumerate() {
                    self.mpv(json!([
                        "loadfile",
                        path,
                        if index == 0 { "replace" } else { "append" }
                    ]))?;
                }
                self.state.action = "Локальная очередь передана mpv".into();
            }
            Request::Open { to } => {
                let uri = Source::parse(&to)?;
                let p = self.player()?;
                self.bus.action(p, "OpenUri", Some(&uri.uri()))?;
                self.pending = Some(Pending {
                    owner: p.owner.clone(),
                    before: p.track.uri.clone(),
                    uri,
                    deadline: Instant::now() + Duration::from_secs(5),
                });
                self.state.action =
                    "Ожидается подтверждение плеера · очередь может измениться".into();
                self.refresh(false);
            }
            Request::Import { path } => {
                let bytes = core::read_limited(&PathBuf::from(path), core::CATALOG_LIMIT)?;
                return self.import(&bytes, false, false);
            }
            Request::Fetch { .. } => unreachable!("download runs in its own thread"),
            Request::ApplyImport { ticket } => {
                let (_, rules) = self
                    .staged
                    .take_if(|(t, _)| *t == ticket)
                    .context("Создайте новый предпросмотр импорта")?;
                if rules.is_empty() {
                    bail!("Нет подходящих правил. Текущий каталог сохранён.");
                }
                let mut settings = self.settings.clone();
                settings.catalog = rules;
                self.save(settings)?;
                self.publish_map();
            }
            Request::Catalog { url, minutes } => {
                if !url.is_empty() {
                    validate_url(&url)?;
                }
                if minutes != 0 && (minutes < 15 || url.is_empty()) {
                    bail!("Интервал обновления должен быть от 15 минут, с URL");
                }
                let mut settings = self.settings.clone();
                if settings.catalog_url != url {
                    settings.catalog_checked = 0;
                }
                settings.catalog_url = url;
                settings.catalog_minutes = minutes;
                self.retry_at = 0;
                self.save(settings)?;
            }
            Request::ClearHistory => {
                core::write_json(
                    &core::data_dir().join("history.json"),
                    &Vec::<Listening>::new(),
                )?;
                self.recorder.reset();
                self.state.history.clear();
            }
            Request::Preferences {
                light,
                above,
                autostart,
            } => {
                let mut settings = self.settings.clone();
                if settings.autostart != autostart {
                    set_autostart(autostart)?;
                }
                settings.light = light;
                settings.above = above;
                settings.autostart = autostart;
                self.save(settings)?;
            }
            Request::Quit => {
                self.finish_history();
                self.detach_mpv();
            }
            Request::Subscribe => unreachable!("handled by listener"),
        }
        Ok(Response::default())
    }
    fn mpv(&self, args: serde_json::Value) -> Result<serde_json::Value> {
        let p = self.player()?;
        mpv::command(&mpv::socket(&self.runtime, p.pid), p.pid, args)
    }
    fn event(&mut self, event: Event) {
        match event {
            Event::Shutdown => self.running = false,
            Event::Signal(Signal::Names) => self.rescan(),
            Event::Signal(Signal::Property(owner)) => {
                if self.state.player.as_ref().is_some_and(|p| p.owner == owner) {
                    self.refresh(false);
                }
            }
            Event::Signal(Signal::Seek(owner, position)) => {
                if position <= 250_000
                    && self
                        .state
                        .player
                        .as_ref()
                        .is_some_and(|p| p.owner == owner && p.status == "Playing")
                    && self
                        .last_seek
                        .is_none_or(|t| t.elapsed() > Duration::from_secs(1))
                {
                    self.repeat += 1;
                    self.last_seek = Some(Instant::now());
                    self.history();
                }
            }
            Event::Mpv(pid, load) => {
                if !self.state.player.as_ref().is_some_and(|p| p.pid == pid) {
                    return;
                }
                if let Some(a) = &mut self.state.adapter {
                    if a.load == load {
                        return;
                    }
                    if load.state == "applied" {
                        self.state.action = "Адаптер mpv подтвердил запуск альтернативного аудиопотока · очередь сохранена".into();
                    } else if load.state == "fallback" {
                        self.state.action =
                            "Замена недоступна · mpv открывает исходный файл".into();
                        self.state.error.clone_from(&self.state.action);
                    } else if load.state == "original" {
                        self.state.action = "mpv воспроизводит оригинал".into();
                    } else if load.state == "error" {
                        self.state.error = "mpv не смог открыть дорожку".into();
                    }
                    if load.target.is_some()
                        && matches!(load.state.as_str(), "applied" | "fallback" | "error")
                    {
                        self.state.last_replacement = Some(load.clone());
                    }
                    a.load = load;
                }
                self.refresh(false);
            }
            Event::Subscribe(client) => {
                if client.try_send(self.wire_state()).is_ok() {
                    self.clients.push(client);
                }
            }
            Event::Client(Request::Fetch { url }, answer) => {
                if self.downloading {
                    let _ = answer.send(Response {
                        error: Some("Обновление уже выполняется".into()),
                        ..Response::default()
                    });
                } else {
                    self.download(url, Some(answer));
                }
            }
            Event::Client(req, answer) => {
                let result = self.request(req);
                let _ = answer.send(match result {
                    Ok(response) => response,
                    Err(err) => Response {
                        error: Some(err.to_string()),
                        ..Response::default()
                    },
                });
            }
            Event::Download(result, answer) => {
                self.downloading = false;
                let result = result.and_then(|bytes| self.import(&bytes, true, answer.is_none()));
                if result.is_err() {
                    self.retry_at = core::now() + 900;
                }
                let response = match result {
                    Ok(response) => response,
                    Err(err) => {
                        self.state.error =
                            "Каталог не обновился · сохранена предыдущая копия".into();
                        Response {
                            error: Some(err.to_string()),
                            ..Response::default()
                        }
                    }
                };
                if let Some(answer) = answer {
                    let _ = answer.send(response);
                }
            }
        }
    }
}

fn proc_start(pid: u32) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(") ")?
        .1
        .split_whitespace()
        .nth(19)
        .map(str::to_owned)
}
fn client_version(p: &Player) -> String {
    if p.identity == "YandexMusic"
        && fs::read_link(format!("/proc/{}/exe", p.pid))
            .ok()
            .is_some_and(|p| p == std::path::Path::new("/opt/yandex-music/yandexmusic"))
        && let Ok(output) = std::process::Command::new("pacman")
            .args(["-Q", "yandex-music"])
            .output()
        && output.status.success()
        && let Some(version) = String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .nth(1)
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".+-:".contains(&b))
    {
        return version.into();
    }
    "версия через MPRIS недоступна".into()
}
fn validate_url(url: &str) -> Result<()> {
    match Source::parse(url)? {
        Source::Audio(_) if url.starts_with("https://") => Ok(()),
        _ => bail!("Для каталога укажите прямой URL HTTPS"),
    }
}
fn fetch(url: &str) -> Result<Vec<u8>> {
    validate_url(url)?;
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .https_only(true)
        .max_redirects(0)
        .build();
    let mut response = ureq::Agent::new_with_config(config)
        .get(url)
        .call()
        .map_err(|_| anyhow!("Не удалось загрузить каталог по HTTPS"))?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(core::CATALOG_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > core::CATALOG_LIMIT {
        bail!("Каталог больше 1 МиБ");
    }
    Ok(bytes)
}
fn set_autostart(enabled: bool) -> Result<()> {
    let dir = core::config_dir().with_file_name("autostart");
    let path = dir.join(format!("{}.desktop", core::project().id));
    if enabled {
        fs::create_dir_all(&dir)?;
        let binary = std::env::current_exe()?;
        let escaped = binary
            .to_string_lossy()
            .replace('\\', "\\\\\\\\")
            .replace('"', "\\\\\"")
            .replace('$', "\\\\$")
            .replace('`', "\\\\`")
            .replace('%', "%%");
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName={}\nExec=\"{escaped}\"\nTerminal=false\n",
            core::project().name
        );
        core::atomic_in_user_dir(&path, entry.as_bytes())?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
fn client(mut stream: UnixStream, tx: Sender<Event>) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let Some(value) = mpv::read_line(&mut reader, REQUEST_LIMIT)? else {
        return Ok(());
    };
    if matches!(
        serde_json::from_value::<Request>(value.clone()),
        Ok(Request::Subscribe)
    ) {
        let (send, receive) = mpsc::sync_channel::<Arc<Vec<u8>>>(8);
        tx.send(Event::Subscribe(send))?;
        // A separate read detects a closed UI even when no player events arrive.
        let mut monitor = stream.try_clone()?;
        monitor.set_read_timeout(None)?;
        std::thread::spawn(move || {
            let _ = monitor.read(&mut [0]);
            let _ = monitor.shutdown(Shutdown::Both);
        });
        loop {
            match receive.recv_timeout(Duration::from_secs(20)) {
                Ok(line) => stream.write_all(&line)?,
                Err(mpsc::RecvTimeoutError::Timeout) => stream.write_all(b"{}\n")?,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        return Ok(());
    }
    let (send, receive) = mpsc::channel();
    let request = serde_json::from_value::<Request>(value);
    let quit = matches!(request, Ok(Request::Quit));
    let answer = match request {
        Ok(request) => {
            tx.send(Event::Client(request, send))?;
            receive.recv_timeout(Duration::from_secs(12))?
        }
        Err(_) => Response {
            error: Some("Неизвестный запрос Overtone".into()),
            ..Response::default()
        },
    };
    writeln!(stream, "{}", serde_json::to_string(&answer)?)?;
    if quit {
        tx.send(Event::Shutdown)?;
    }
    Ok(())
}
fn run(engine: &mut Engine, rx: Receiver<Event>) {
    while engine.running {
        let mut wait = if engine.state.timer.is_some() || engine.pending.is_some() {
            Duration::from_secs(1)
        } else {
            Duration::from_secs(86_400)
        };
        if engine.settings.catalog_minutes > 0 && !engine.downloading {
            wait = wait.min(Duration::from_secs(
                engine.catalog_due().saturating_sub(core::now()),
            ));
        }
        match rx.recv_timeout(wait) {
            Ok(event) => engine.event(event),
            Err(mpsc::RecvTimeoutError::Timeout) => (),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        engine.tick();
        engine.broadcast();
    }
}
pub fn start() -> Result<()> {
    let runtime = core::runtime_dir()?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(runtime.join("background.lock"))?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Ok(());
    }
    let path = socket()?;
    let bus = Bus::connect().context("Не удалось подключиться к сессионной D-Bus")?;
    let (tx, rx) = mpsc::channel();
    let sender = tx.clone();
    overtone_player::listen(&bus, move |s| {
        let _ = sender.send(Event::Signal(s));
    })?;
    let mut engine = Engine::new(bus, runtime, tx.clone());
    engine.rescan();
    let _ = fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let _ = client(stream, tx);
            });
        }
    });
    run(&mut engine, rx);
    let _ = fs::remove_file(path);
    Ok(())
}
