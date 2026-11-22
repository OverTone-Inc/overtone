use crate::*;
use overtone_core::{Listening, Rule, compatibility, plugins};

const PAGES: &[(&str, &str)] = &[
    ("overview", "Обзор"),
    ("plugins", "Плагины"),
    ("versions", "Замены"),
    ("timer", "Таймер"),
    ("history", "История"),
    ("settings", "Настройки"),
    ("diagnostics", "Диагностика"),
];

pub struct Manager {
    pub window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    nav: Vec<gtk::Button>,
    hub: std::rc::Weak<Hub>,
    players: gtk::DropDown,
    player_names: RefCell<Vec<String>>,
    rendering: Rc<Cell<bool>>,
    title: gtk::Label,
    artist: gtk::Label,
    playback: gtk::Label,
    art: Art,
    play: gtk::Button,
    next: gtk::Button,
    replace: gtk::Button,
    mini: gtk::Button,
    connection: gtk::Label,
    replacement: gtk::Label,
    original: gtk::Button,
    action: gtk::Label,
    result: gtk::Label,
    error: gtk::Label,
    plugin_rows: Vec<(String, gtk::Label, gtk::Switch)>,
    from: gtk::Entry,
    to: gtk::Entry,
    exclude: gtk::CheckButton,
    compatibility: gtk::Label,
    last_replacement: gtk::Label,
    local_queue: gtk::Button,
    close_mpv: gtk::Button,
    rule_search: gtk::SearchEntry,
    rule_rows: gtk::Box,
    history_search: gtk::SearchEntry,
    history_rows: gtk::Box,
    timer_info: gtk::Label,
    cancel_timer: gtk::Button,
    timer_starts: Vec<gtk::Button>,
    light: gtk::CheckButton,
    above: gtk::CheckButton,
    autostart: gtk::CheckButton,
    catalog_url: gtk::Entry,
    catalog_minutes: gtk::SpinButton,
    catalog_state: gtk::Label,
    report: gtk::Label,
    current_timer: RefCell<Option<overtone_core::Timer>>,
}
impl Manager {
    pub fn new(app: &gtk::Application, hub: &Rc<Hub>) -> Rc<Self> {
        let (window, bar) = setup_window(app, &project().name, (1000, 660));
        window.set_widget_name("overtone-manager");
        let top_mini = button("Мини-плеер");
        bar.pack_end(&top_mini);
        let weak = Rc::downgrade(hub);
        top_mini.connect_clicked(move |_| {
            if let Some(hub) = weak.upgrade() {
                hub.open_mini();
            }
        });

        let root = h(0);
        let side = v(6);
        side.add_css_class("side");
        side.set_size_request(164, -1);
        let nav: Vec<_> = PAGES
            .iter()
            .map(|(_, name)| {
                let nav = button(name);
                nav.set_halign(gtk::Align::Fill);
                side.append(&nav);
                nav
            })
            .collect();
        let spacer = v(0);
        spacer.set_vexpand(true);
        side.append(&spacer);
        let version = text(&format!("Версия {}", env!("CARGO_PKG_VERSION")), "subtle");
        version.set_halign(gtk::Align::Center);
        side.append(&version);
        let quit = button("Выйти полностью");
        side.append(&quit);
        let weak = Rc::downgrade(hub);
        quit.connect_clicked(move |_| {
            if let Some(h) = weak.upgrade() {
                h.quit();
            }
        });
        root.append(&side);

        let content = v(0);
        content.set_hexpand(true);
        let stack = gtk::Stack::builder()
            .hexpand(true)
            .vexpand(true)
            .hhomogeneous(false)
            .vhomogeneous(false)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(140)
            .build();
        content.append(&stack);
        let foot = v(4);
        foot.add_css_class("foot");
        let action_label = clipped("", "subtle");
        let result = clipped("", "error");
        let error = clipped("", "error");
        foot.append(&action_label);
        foot.append(&result);
        foot.append(&error);
        content.append(&foot);
        root.append(&content);
        window.set_child(Some(&root));

        let overview = section(&stack, "overview", "Обзор");
        let rack = card();
        let art = Art::new(115);
        let now = h(18);
        now.append(&art.image);
        let titles = v(5);
        titles.set_valign(gtk::Align::Center);
        titles.set_hexpand(true);
        let playback = text("Нет подключения", "accent-text");
        let title = clipped("Нет текущей записи", "track-title");
        let artist = clipped("Выберите плеер, чтобы начать", "subtle");
        titles.append(&playback);
        titles.append(&title);
        titles.append(&artist);
        now.append(&titles);
        rack.append(&now);
        let transport = h(8);
        let play = action(&transport, "Воспроизвести", hub, Request::PlayPause);
        play.add_css_class("primary");
        let next = action(&transport, "Далее", hub, Request::Next);
        let replace = button("Заменить этот трек");
        transport.append(&replace);
        rack.append(&transport);
        overview.append(&rack);
        let replacement = text("", "accent-text");
        let original = action(
            &overview,
            "Вернуть оригинал сейчас",
            hub,
            Request::Preview { to: None },
        );
        original.set_halign(gtk::Align::Start);
        overview.append(&replacement);
        let connected = card();
        connected.append(&text("Выбранный плеер", "strong"));
        let players = gtk::DropDown::from_strings(&["Выбрать плеер"]);
        players.set_hexpand(true);
        let player_line = h(8);
        player_line.append(&players);
        action(&player_line, "Перепроверить", hub, Request::Rescan);
        connected.append(&player_line);
        let connection = text("Подключите плеер с MPRIS", "subtle");
        connected.append(&connection);
        overview.append(&connected);
        let local = h(8);
        let start = button("Запустить mpv с адаптером");
        local.append(&start);
        let local_queue = button("Открыть локальную очередь");
        local.append(&local_queue);
        let close_mpv = action(&local, "Закрыть отдельный mpv", hub, Request::CloseMpv);
        overview.append(&local);

        let plugin_page = section(&stack, "plugins", "Плагины");
        let plugin_card = card();
        let rendering = Rc::new(Cell::new(false));
        let plugin_rows = plugins()
            .into_iter()
            .map(|plugin| {
                let row = h(14);
                row.add_css_class("row");
                let column = v(4);
                column.set_hexpand(true);
                column.append(&text(
                    &format!("{} · {}", plugin.name, plugin.version),
                    "strong",
                ));
                column.append(&text(&plugin.description, "subtle"));
                let status = text("Ждёт плеер", "accent-text");
                column.append(&status);
                row.append(&column);
                let options = button("Настройки");
                options.set_valign(gtk::Align::Center);
                let weak = Rc::downgrade(hub);
                let id = plugin.id.clone();
                options.connect_clicked(move |_| {
                    if let Some(h) = weak.upgrade() {
                        h.open_manager(Some(if id == "mini" { "settings" } else { &id }));
                    }
                });
                row.append(&options);
                let toggle = gtk::Switch::builder().valign(gtk::Align::Center).build();
                toggle.set_tooltip_text(Some(&format!("Включить {}", plugin.name)));
                toggle.update_property(&[gtk::accessible::Property::Label(&plugin.name)]);
                let weak = Rc::downgrade(hub);
                let guard = rendering.clone();
                let id = plugin.id.clone();
                toggle.connect_state_set(move |_, enabled| {
                    if !guard.get()
                        && let Some(h) = weak.upgrade()
                    {
                        h.request(Request::Toggle {
                            id: id.clone(),
                            enabled,
                        });
                        return glib::Propagation::Stop;
                    }
                    glib::Propagation::Proceed
                });
                row.append(&toggle);
                plugin_card.append(&row);
                (plugin.id, status, toggle)
            })
            .collect();
        plugin_page.append(&plugin_card);

        let versions = section(&stack, "versions", "Замены");
        let edit = card();
        let from = entry("Оригинал", "Ссылка на запись или абсолютный путь");
        let to = entry("Другая версия", "Ссылка HTTPS или абсолютный путь");
        edit.append(&from.0);
        edit.append(&to.0);
        let from = from.1;
        let to = to.1;
        let file_row = h(7);
        let from_current = button("Текущий трек");
        let file_from = button("Файл оригинала");
        let file_to = button("Файл замены");
        for b in [&from_current, &file_from, &file_to] {
            file_row.append(b);
        }
        edit.append(&file_row);
        let exclude = gtk::CheckButton::with_label("Всегда играть оригинал");
        edit.append(&exclude);
        let controls = h(7);
        let save = button("Сохранить правило");
        save.add_css_class("primary");
        let preview = button("Предпрослушать в mpv");
        let open = button("Открыть другую версию");
        for b in [&save, &preview, &open] {
            controls.append(b);
        }
        edit.append(&controls);
        let compat = text("", "subtle");
        edit.append(&compat);
        let last_replacement = text("", "subtle");
        edit.append(&last_replacement);
        versions.append(&edit);
        let import_row = h(7);
        let import_local = button("Импорт JSON");
        let rule_search = gtk::SearchEntry::builder()
            .placeholder_text("Найти правило")
            .hexpand(true)
            .build();
        import_row.append(&rule_search);
        import_row.append(&import_local);
        versions.append(&import_row);
        let rule_rows = v(0);
        versions.append(&rule_rows);

        let timer_page = section(&stack, "timer", "Таймер сна");
        let timer_card = card();
        let timer_info = text("Таймер выключен", "track-title");
        timer_card.append(&timer_info);
        timer_card.append(&text(
            "Пауза действует только для экземпляра плеера, выбранного при запуске таймера.",
            "subtle",
        ));
        let presets = h(8);
        let mut timer_starts = Vec::new();
        for minutes in [15, 30, 45, 60] {
            timer_starts.push(action(
                &presets,
                &format!("{minutes} мин"),
                hub,
                Request::Timer {
                    seconds: minutes * 60,
                },
            ));
        }
        timer_card.append(&presets);
        let custom_row = h(8);
        let custom = gtk::SpinButton::with_range(1., 1440., 1.);
        custom.set_value(25.);
        custom.update_property(&[gtk::accessible::Property::Label(
            "Пользовательский интервал в минутах",
        )]);
        custom_row.append(&custom);
        let start_timer = button("Поставить таймер, мин");
        custom_row.append(&start_timer);
        let weak = Rc::downgrade(hub);
        start_timer.connect_clicked(move |_| {
            if let Some(h) = weak.upgrade() {
                h.request(Request::Timer {
                    seconds: custom.value_as_int() as u64 * 60,
                });
            }
        });
        timer_starts.push(start_timer);
        let cancel_timer = action(&custom_row, "Отменить", hub, Request::CancelTimer);
        timer_card.append(&custom_row);
        timer_page.append(&timer_card);

        let history_page = section(&stack, "history", "История");
        history_page.append(&text(
            "До 600 завершённых прослушиваний на этом компьютере.",
            "subtle",
        ));
        let history_nav = h(7);
        let history_search = gtk::SearchEntry::builder()
            .placeholder_text("Трек или исполнитель")
            .hexpand(true)
            .build();
        let export = button("Экспорт JSON");
        let clear = button("Очистить");
        for widget in [
            history_search.clone().upcast::<gtk::Widget>(),
            export.clone().upcast(),
            clear.clone().upcast(),
        ] {
            history_nav.append(&widget);
        }
        history_page.append(&history_nav);
        let history_rows = v(0);
        history_page.append(&history_rows);

        let prefs = section(&stack, "settings", "Настройки");
        let appearance = card();
        appearance.append(&text("Окна и запуск", "strong"));
        let light = gtk::CheckButton::with_label("Светлая тема");
        let above = gtk::CheckButton::with_label("Мини-плеер поверх окон в X11");
        above.set_sensitive(x11::supported(&window));
        let autostart = gtk::CheckButton::with_label("Запускать фон при входе в систему");
        for control in [&light, &above, &autostart] {
            appearance.append(control);
        }
        prefs.append(&appearance);
        let catalogs = card();
        catalogs.append(&text("Каталог правил Overtone JSON v1", "strong"));
        let (url_field, catalog_url) = entry("Адрес каталога", "https://…");
        catalogs.append(&url_field);
        let controls = h(7);
        let catalog_minutes = gtk::SpinButton::with_range(0., 10080., 15.);
        catalog_minutes.update_property(&[gtk::accessible::Property::Label(
            "Интервал в минутах. Ноль выключает расписание.",
        )]);
        controls.append(&catalog_minutes);
        let save_catalog = button("Сохранить интервал, мин");
        let fetch = button("Загрузить и проверить");
        controls.append(&save_catalog);
        controls.append(&fetch);
        catalogs.append(&controls);
        catalogs.append(&text(
            "0 мин — вручную. От 15 мин — по расписанию для указанного URL.",
            "subtle",
        ));
        let catalog_state = text("", "subtle");
        catalogs.append(&catalog_state);
        prefs.append(&catalogs);

        let diagnostics = section(&stack, "diagnostics", "Диагностика");
        let report_card = card();
        let report = text("Подключение к фоновому процессу…", "");
        report.set_selectable(true);
        report_card.append(&report);
        let actions = h(8);
        action(&actions, "Перепроверить плееры", hub, Request::Rescan);
        let copy = button("Копировать отчёт");
        actions.append(&copy);
        report_card.append(&actions);
        diagnostics.append(&report_card);
        for row in compatibility() {
            let box_ = card();
            box_.append(&text(
                &format!("{} · {}", row.client, row.version),
                "strong",
            ));
            box_.append(&text(
                &format!(
                    "{} · {} · {} · {}",
                    row.control, row.replacement, row.queue, row.date
                ),
                "subtle",
            ));
            diagnostics.append(&box_);
        }

        let view = Rc::new(Self {
            window,
            stack,
            nav,
            hub: Rc::downgrade(hub),
            players,
            player_names: Default::default(),
            rendering,
            title,
            artist,
            playback,
            art,
            play,
            next,
            replace: replace.clone(),
            mini: top_mini,
            connection,
            replacement,
            original,
            action: action_label,
            result,
            error,
            plugin_rows,
            from,
            to,
            exclude,
            compatibility: compat,
            last_replacement,
            local_queue: local_queue.clone(),
            close_mpv,
            rule_search,
            rule_rows,
            history_search,
            history_rows,
            timer_info,
            cancel_timer,
            timer_starts,
            light,
            above,
            autostart,
            catalog_url,
            catalog_minutes,
            catalog_state,
            report,
            current_timer: Default::default(),
        });
        for (idx, (id, _)) in PAGES.iter().enumerate() {
            let weak = Rc::downgrade(&view);
            view.nav[idx].connect_clicked(move |_| {
                if let Some(v) = weak.upgrade() {
                    v.open(id);
                }
            });
        }
        {
            let weak = Rc::downgrade(&view);
            view.players.connect_selected_notify(move |list| {
                if let Some(v) = weak.upgrade()
                    && !v.rendering.get()
                    && let Some(h) = v.hub.upgrade()
                    && let Some(name) = v.player_names.borrow().get(list.selected() as usize)
                {
                    h.request(Request::Select { name: name.clone() });
                }
            });
        }
        let weak = Rc::downgrade(&view);
        start.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                match std::process::Command::new(sibling("overtone-ctl"))
                    .arg("mpv")
                    .spawn()
                {
                    Ok(mut child) => {
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                        v.feedback("Выберите новый экземпляр mpv в списке плееров.", false);
                    }
                    Err(_) => v.feedback(
                        "Не удалось запустить mpv. Проверьте пакеты mpv и mpv-mpris.",
                        true,
                    ),
                }
            }
        });
        let weak = Rc::downgrade(&view);
        local_queue.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                let hub = v.hub.clone();
                choose_files(
                    &v.window,
                    "Открыть локальные записи в выбранном mpv",
                    true,
                    move |files| {
                        if let Some(h) = hub.upgrade() {
                            h.request(Request::LocalQueue {
                                paths: files
                                    .iter()
                                    .map(|p| p.to_string_lossy().into_owned())
                                    .collect(),
                            });
                        }
                    },
                );
            }
        });
        for (b, switch_page) in [(replace, true), (from_current, false)] {
            let weak = Rc::downgrade(&view);
            b.connect_clicked(move |_| {
                if let Some(v) = weak.upgrade() && let Some(h) = v.hub.upgrade() {
                    if let Some(from) = h.state.borrow().original() {
                        v.from.set_text(from.value());
                        if switch_page { v.open("versions"); v.to.grab_focus(); }
                    } else { v.feedback("Плеер пока не передал идентичность трека. Вставьте ссылку на оригинал вручную.",true); }
                }
            });
        }
        for (b, field) in [(file_from, view.from.clone()), (file_to, view.to.clone())] {
            let w = view.window.downgrade();
            b.connect_clicked(move |_| {
                if let Some(w) = w.upgrade() {
                    let field = field.clone();
                    choose_files(
                        &w,
                        "Выбрать свой аудиофайл",
                        false,
                        move |files| {
                            if let Some(p) = files.first() {
                                field.set_text(&p.to_string_lossy());
                            }
                        },
                    );
                }
            });
        }
        let weak = Rc::downgrade(&view);
        view.exclude.connect_toggled(move |e| {
            if let Some(v) = weak.upgrade() {
                v.to.set_sensitive(!e.is_active());
            }
        });
        for (b, kind) in [(save, "save"), (preview, "preview"), (open, "open")] {
            let weak = Rc::downgrade(&view);
            b.connect_clicked(move |_| {
                if let Some(v) = weak.upgrade() && let Some(h) = v.hub.upgrade() {
                    let from = v.from.text().to_string();
                    let to = v.to.text().to_string();
                    if to.trim().is_empty() && !(kind == "save" && v.exclude.is_active()) {
                        v.feedback("Укажите альтернативный источник.",true);
                        return;
                    }
                    if kind == "preview" && h.state.borrow().original().as_ref() != Source::parse(&from).ok().as_ref() {
                        v.feedback("Предпрослушивание возможно для текущего оригинала. Нажмите «Текущий трек».",true);
                        return;
                    }
                    let request = match kind {
                        "save" => Request::SaveRule { from,to:(!v.exclude.is_active()).then_some(to),enabled:true },
                        "preview" => Request::Preview { to:Some(to) },
                        _ => Request::Open { to },
                    };
                    h.request(request);
                }
            });
        }
        let weak = Rc::downgrade(&view);
        import_local.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                let weak = Rc::downgrade(&v);
                choose_files(
                    &v.window,
                    "Выбрать каталог Overtone",
                    false,
                    move |files| {
                        if let Some(v) = weak.upgrade()
                            && let Some(p) = files.first()
                        {
                            v.import(Request::Import {
                                path: p.to_string_lossy().into_owned(),
                            });
                        }
                    },
                );
            }
        });
        let weak = Rc::downgrade(&view);
        view.rule_search.connect_search_changed(move |_| {
            if let Some(v) = weak.upgrade()
                && let Some(h) = v.hub.upgrade()
            {
                v.rules(&h.state.borrow());
            }
        });
        let weak = Rc::downgrade(&view);
        view.history_search.connect_search_changed(move |_| {
            if let Some(v) = weak.upgrade()
                && let Some(h) = v.hub.upgrade()
            {
                v.history(&h.state.borrow());
            }
        });
        let weak = Rc::downgrade(&view);
        clear.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade()
                && let Some(h) = v.hub.upgrade()
            {
                confirm(
                    &v.window,
                    "Очистить историю?",
                    "Сохранённые прослушивания будут удалены.",
                    "Очистить",
                    move || h.request(Request::ClearHistory),
                );
            }
        });
        let weak = Rc::downgrade(&view);
        export.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade()
                && let Some(h) = v.hub.upgrade()
            {
                v.export(&h.state.borrow().history);
            }
        });
        for preference in [&view.light, &view.above, &view.autostart] {
            let weak = Rc::downgrade(&view);
            preference.connect_toggled(move |_| {
                if let Some(v) = weak.upgrade()
                    && !v.rendering.get()
                    && let Some(h) = v.hub.upgrade()
                {
                    h.request(Request::Preferences {
                        light: v.light.is_active(),
                        above: v.above.is_active(),
                        autostart: v.autostart.is_active(),
                    });
                }
            });
        }
        let weak = Rc::downgrade(&view);
        save_catalog.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade()
                && let Some(h) = v.hub.upgrade()
            {
                h.request(Request::Catalog {
                    url: v.catalog_url.text().trim().into(),
                    minutes: v.catalog_minutes.value_as_int() as u32,
                });
            }
        });
        let weak = Rc::downgrade(&view);
        fetch.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.import(Request::Fetch {
                    url: v.catalog_url.text().trim().into(),
                });
            }
        });
        let weak = Rc::downgrade(&view);
        copy.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                gtk::prelude::WidgetExt::display(&v.window)
                    .clipboard()
                    .set_text(&v.report.text());
                v.feedback("Отчёт скопирован.", false);
            }
        });
        let weak = Rc::downgrade(&view);
        glib::timeout_add_seconds_local(1, move || {
            if let Some(v) = weak.upgrade() {
                v.refresh_timer();
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
        {
            let weak = Rc::downgrade(&view);
            watch_visibility(&view.window, move || {
                if let Some(v) = weak.upgrade()
                    && let Some(h) = v.hub.upgrade()
                {
                    v.update_art(&h.state.borrow());
                }
            });
        }
        view.open("overview");
        view
    }

    pub fn open(&self, id: &str) {
        let Some(idx) = PAGES.iter().position(|(page, _)| *page == id) else {
            return;
        };
        self.stack.set_visible_child_name(id);
        for b in &self.nav {
            b.remove_css_class("active");
        }
        self.nav[idx].add_css_class("active");
        if let Some(h) = self.hub.upgrade() {
            self.update_art(&h.state.borrow());
        }
    }
    fn update_art(&self, s: &State) {
        let art = if self.stack.visible_child_name().as_deref() == Some("overview")
            && drawable(&self.window)
        {
            s.player
                .as_ref()
                .map(|p| p.track.art.as_str())
                .unwrap_or("")
        } else {
            ""
        };
        self.art.update(art);
    }
    pub fn feedback(&self, msg: &str, bad: bool) {
        self.result.set_label(msg);
        self.result.set_visible(!msg.is_empty());
        if bad {
            self.result.add_css_class("error");
        } else {
            self.result.remove_css_class("error");
        }
        self.result
            .set_tooltip_text((!msg.is_empty()).then_some(msg));
    }
    pub fn update(&self, s: &State, previous: &State) {
        self.rendering.set(true);
        let p = s.player.as_ref();
        self.window.set_css_classes(if s.light {
            &["overtone", "light"]
        } else {
            &["overtone"]
        });
        self.title.set_label(
            p.filter(|p| !p.track.title.is_empty())
                .map(|p| p.track.title.as_str())
                .unwrap_or("Нет текущей записи"),
        );
        self.title
            .set_tooltip_text(p.map(|p| p.track.title.as_str()).filter(|t| !t.is_empty()));
        self.artist.set_label(
            p.filter(|p| !p.track.artist.is_empty())
                .map(|p| p.track.artist.as_str())
                .unwrap_or(if p.is_some() {
                    "Исполнитель не указан"
                } else {
                    "Выберите плеер, чтобы начать"
                }),
        );
        self.playback.set_label(
            p.map(|p| friendly_status(&p.status))
                .unwrap_or("Нет подключения"),
        );
        self.update_art(s);
        let (label, play, next) = playback(p);
        self.play.set_label(label);
        self.play.set_sensitive(play);
        self.next.set_sensitive(next);
        self.replace.set_sensitive(s.original().is_some());
        self.mini.set_sensitive(s.enabled("mini"));
        self.local_queue.set_sensitive(s.adapter.is_some());
        self.close_mpv.set_sensitive(s.adapter.is_some());
        let mut names = self.player_names.borrow_mut();
        let new_names: Vec<_> = std::iter::once(String::new())
            .chain(s.players.iter().map(|item| item.name.clone()))
            .collect();
        if *names != new_names {
            let titles: Vec<_> = std::iter::once("Автовыбор одного плеера".into())
                .chain(s.players.iter().map(|i| {
                    let duplicate = s.players.iter().filter(|p| p.title == i.title).count() > 1;
                    if duplicate {
                        format!("{} · {}", i.title, i.name.rsplit('.').next().unwrap_or(""))
                    } else {
                        i.title.clone()
                    }
                }))
                .collect();
            let strings: Vec<_> = titles.iter().map(String::as_str).collect();
            self.players
                .set_model(Some(&gtk::StringList::new(&strings)));
            *names = new_names;
        }
        let selected = names.iter().position(|n| n == &s.selected).unwrap_or(0) as u32;
        self.players.set_selected(selected);
        self.connection.set_label(&if let Some(p) = p {
            format!(
                "{} · {}\n{} · {}",
                p.identity,
                s.version,
                if p.control {
                    "Управление MPRIS"
                } else {
                    "Наблюдение MPRIS"
                },
                if s.adapted() {
                    "Автозамена до старта"
                } else if p.schemes.is_empty() {
                    "Клиент не объявил схемы OpenUri"
                } else {
                    "Открытие ссылки зависит от URI"
                }
            )
        } else if !s.selected.is_empty() {
            "Выбранный экземпляр отключён. Выберите плеер в списке.".into()
        } else if s.players.len() > 1 {
            "Подключено несколько плееров. Выберите нужный.".into()
        } else {
            "Ожидается плеер с MPRIS.".into()
        });
        let applied = s.adapter.as_ref().filter(|a| a.load.state == "applied");
        self.original.set_visible(applied.is_some());
        self.original.set_sensitive(s.enabled("versions"));
        self.replacement.set_visible(applied.is_some());
        if let Some(a) = applied {
            self.replacement.set_label(&format!(
                "{} → {} · {} · {}",
                label_source(&a.load.original),
                a.load
                    .target
                    .as_deref()
                    .map(label_source)
                    .unwrap_or_default(),
                a.load.origin.as_deref().unwrap_or_default(),
                date(a.load.time)
            ));
        }
        for (id, status, toggle) in &self.plugin_rows {
            let enabled = s.enabled(id);
            toggle.set_active(enabled);
            let status_text = if !enabled {
                "Выключен пользователем"
            } else if p.is_none() {
                "Ждёт плеер"
            } else if id == "versions" && !s.adapted() {
                "Автозамена несовместима с выбранным плеером"
            } else if id == "timer" && !p.is_some_and(|p| p.control && p.pause) {
                "Плеер пока не разрешает паузу"
            } else {
                "Готов к работе"
            };
            status.set_label(status_text);
        }
        self.compatibility.set_label(if s.adapted() {
            "Звук выдаёт выбранный mpv. Автозамена локального файла или прямой ссылки HTTPS сохраняет очередь."
        } else if p.is_some_and(|p| !p.schemes.is_empty()) {
            "Открытие ссылки использует MPRIS и может изменить очередь. Для замены до старта подключите mpv с адаптером."
        } else {
            "Автозамена недоступна. Плеер не подтвердил открытие URI. Правила можно сохранить для совместимого плеера."
        });
        self.last_replacement
            .set_visible(s.last_replacement.is_some());
        if let Some(load) = &s.last_replacement {
            let outcome = match load.state.as_str() {
                "applied" => "Воспроизведена другая версия",
                "fallback" => "Замена недоступна, воспроизведён оригинал",
                _ => "Ошибка открытия",
            };
            self.last_replacement.set_label(&format!(
                "{} → {} · {} · {} · {outcome}",
                label_source(&load.original),
                load.target.as_deref().map(label_source).unwrap_or_default(),
                load.origin.as_deref().unwrap_or_default(),
                date(load.time)
            ));
        }
        self.action.set_label(&s.action);
        self.action.set_visible(!s.action.is_empty());
        self.action
            .set_tooltip_text((!s.action.is_empty()).then_some(&s.action));
        self.error.set_label(&s.error);
        self.error.set_visible(!s.error.is_empty());
        self.error
            .set_tooltip_text((!s.error.is_empty()).then_some(&s.error));
        if s.manual != previous.manual
            || s.catalog != previous.catalog
            || s.last_replacement != previous.last_replacement
            || self.rule_rows.first_child().is_none()
        {
            self.rules(s);
        }
        if s.history != previous.history
            || s.enabled("history") != previous.enabled("history")
            || self.history_rows.first_child().is_none()
        {
            self.history(s);
        }
        self.current_timer.replace(s.timer.clone());
        self.refresh_timer();
        for b in &self.timer_starts {
            b.set_sensitive(s.enabled("timer") && p.is_some_and(|p| p.control && p.pause));
        }
        self.light.set_active(s.light);
        self.above.set_active(s.above);
        self.autostart.set_active(s.autostart);
        if s.catalog_url != previous.catalog_url {
            self.catalog_url.set_text(&s.catalog_url);
        }
        if s.catalog_minutes != previous.catalog_minutes {
            self.catalog_minutes.set_value(s.catalog_minutes as f64);
        }
        self.catalog_state.set_label(&format!(
            "Правил в кеше · {}. Последнее обновление · {}.",
            s.catalog.len(),
            date(s.catalog_checked)
        ));
        self.report.set_label(&s.diagnostic());
        self.rendering.set(false);
    }

    fn refresh_timer(&self) {
        let timer = self.current_timer.borrow();
        self.cancel_timer.set_sensitive(timer.is_some());
        self.timer_info.set_label(&if let Some(t) = timer.as_ref() {
            let remain = t.deadline.saturating_sub(overtone_core::now());
            format!(
                "{:02}:{:02}:{:02} осталось",
                remain / 3600,
                remain / 60 % 60,
                remain % 60
            )
        } else {
            "Таймер выключен".into()
        });
    }
    fn rules(&self, s: &State) {
        clear_box(&self.rule_rows);
        let query = self.rule_search.text().to_lowercase();
        let mut count = 0;
        for (rules, catalog) in [(&s.manual, false), (&s.catalog, true)] {
            for rule in rules {
                let label = format!(
                    "{} → {}",
                    rule.from.label(),
                    rule.to
                        .as_ref()
                        .map(Source::label)
                        .unwrap_or_else(|| "Всегда оригинал".into())
                );
                if !label.to_lowercase().contains(&query) {
                    continue;
                }
                count += 1;
                if count > 100 {
                    continue;
                }
                self.rule_row(rule, catalog, &label, s);
            }
        }
        if count == 0 {
            self.rule_rows
                .append(&text("Правил по этому запросу нет", "subtle"));
        }
        if count > 100 {
            self.rule_rows.append(&text(
                &format!("Показано 100 правил из {count}. Уточните поиск."),
                "subtle",
            ));
        }
    }
    fn rule_row(&self, rule: &Rule, catalog: bool, label: &str, state: &State) {
        let row = h(8);
        row.add_css_class("row");
        let words = v(2);
        words.set_hexpand(true);
        let summary = clipped(label, "strong");
        summary.set_tooltip_text(Some(label));
        words.append(&summary);
        words.append(&text(
            if catalog {
                "Импортированный каталог"
            } else {
                "Ручное правило"
            },
            "subtle",
        ));
        if let Some(load) = &state.last_replacement
            && matches!(load.state.as_str(), "fallback" | "error")
            && (load.origin.as_deref() == Some("Импортированный каталог")) == catalog
            && Source::parse(&load.original).ok().as_ref() == Some(&rule.from)
            && load
                .target
                .as_deref()
                .and_then(|target| Source::parse(target).ok())
                .as_ref()
                == rule.to.as_ref()
        {
            words.append(&text("Не удалось открыть другую версию", "error"));
        }
        row.append(&words);
        let edit = button("Править");
        let from = self.from.clone();
        let to = self.to.clone();
        let exclude = self.exclude.clone();
        let origin = rule.from.value().to_string();
        let target = rule.to.as_ref().map(|s| s.value().to_string());
        edit.connect_clicked(move |_| {
            from.set_text(&origin);
            to.set_text(target.as_deref().unwrap_or_default());
            exclude.set_active(target.is_none());
            to.grab_focus();
        });
        row.append(&edit);
        let Some(h) = self.hub.upgrade() else { return };
        if !catalog {
            action(
                &row,
                "Удалить",
                &h,
                Request::DeleteRule {
                    from: rule.from.value().into(),
                },
            );
        }
        action(
            &row,
            if rule.enabled {
                "Выключить"
            } else {
                "Включить"
            },
            &h,
            Request::RuleEnable {
                from: rule.from.value().into(),
                enabled: !rule.enabled,
                catalog,
            },
        );
        self.rule_rows.append(&row);
    }
    fn history(&self, s: &State) {
        clear_box(&self.history_rows);
        if !s.enabled("history") {
            self.history_rows.append(&text(
                "Запись выключена. Включите Listening History в разделе «Плагины».",
                "subtle",
            ));
        }
        let query = self.history_search.text().to_lowercase();
        let rows = s.history.iter().filter(|e| {
            e.title.to_lowercase().contains(&query) || e.artist.to_lowercase().contains(&query)
        });
        for e in rows.take(100) {
            let row = v(3);
            row.add_css_class("row");
            row.append(&clipped(&format!("{} · {}", e.title, e.artist), "strong"));
            let replaced = e
                .replacement
                .as_ref()
                .map(|r| format!(" · замена {r}"))
                .unwrap_or_default();
            row.append(&text(
                &format!(
                    "{} · {} мин {:02} с{replaced}",
                    date(e.started),
                    e.seconds / 60,
                    e.seconds % 60
                ),
                "subtle",
            ));
            self.history_rows.append(&row);
        }
        if s.history.is_empty() {
            self.history_rows
                .append(&text("Сохранённых прослушиваний пока нет.", "subtle"));
        }
    }
    fn import(&self, request: Request) {
        let Some(h) = self.hub.upgrade() else { return };
        let weak = Rc::downgrade(&h);
        let w = self.window.downgrade();
        self.feedback("Проверяем каталог…", false);
        h.job(Job(
            request,
            Box::new(move |reply| {
                if let Some(h) = weak.upgrade()
                    && let Some(v) = h.manager.borrow().as_ref()
                {
                    match reply {
                        Ok(response) => {
                            if let (Some(summary), Some(ticket), Some(window)) =
                                (response.import, response.ticket, w.upgrade())
                            {
                                v.feedback("", false);
                                let h = h.clone();
                                confirm(
                                    &window,
                                    "Применить каталог?",
                                    &format!(
                                        "Принято · {}\nПропущено · {}\nКонфликты · {}",
                                        summary.accepted, summary.skipped, summary.conflicts
                                    ),
                                    "Применить",
                                    move || h.request(Request::ApplyImport { ticket }),
                                );
                            }
                        }
                        Err(msg) => v.feedback(&msg, true),
                    }
                }
            }),
        ));
    }
    fn export(&self, history: &[Listening]) {
        let bytes = serde_json::to_vec_pretty(history).unwrap_or_default();
        let dialog = gtk::FileDialog::builder()
            .title("Экспорт истории")
            .modal(true)
            .accept_label("Сохранить")
            .initial_name("overtone-history.json")
            .build();
        let weak = self.hub.clone();
        dialog.save(Some(&self.window), gio::Cancellable::NONE, move |result| {
            let Ok(file) = result else { return };
            let result = file.replace_contents(
                &bytes,
                None,
                false,
                gio::FileCreateFlags::PRIVATE,
                gio::Cancellable::NONE,
            );
            if let Some(h) = weak.upgrade()
                && let Some(v) = h.manager.borrow().as_ref()
            {
                v.feedback(
                    if result.is_ok() {
                        "История экспортирована."
                    } else {
                        "Не удалось записать JSON."
                    },
                    result.is_err(),
                );
            }
        });
    }
}

fn clear_box(list: &gtk::Box) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}
fn card() -> gtk::Box {
    let box_ = v(9);
    box_.add_css_class("card");
    box_
}
fn section(stack: &gtk::Stack, id: &str, title: &str) -> gtk::Box {
    let body = v(12);
    body.add_css_class("body");
    body.append(&text(title, "screen-title"));
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&body)
        .build();
    stack.add_named(&scroll, Some(id));
    body
}
fn entry(prompt: &str, placeholder: &str) -> (gtk::Box, gtk::Entry) {
    let row = v(4);
    let l = text(prompt, "strong");
    let field = gtk::Entry::builder()
        .placeholder_text(placeholder)
        .max_length(2048)
        .build();
    field.update_property(&[gtk::accessible::Property::Label(prompt)]);
    row.append(&l);
    row.append(&field);
    (row, field)
}
fn date(timestamp: u64) -> String {
    if timestamp == 0 {
        return "ещё не было".into();
    }
    glib::DateTime::from_unix_local(timestamp as i64)
        .and_then(|t| t.format("%d.%m.%Y · %H:%M"))
        .map(|s| s.to_string())
        .unwrap_or_default()
}
fn label_source(input: &str) -> String {
    Source::parse(input)
        .map(|s| s.label())
        .unwrap_or_else(|_| "Источник".into())
}
#[allow(deprecated)]
fn confirm(
    w: &gtk::ApplicationWindow,
    title: &str,
    detail: &str,
    label: &str,
    action: impl Fn() + 'static,
) {
    let dialog = gtk::MessageDialog::builder()
        .transient_for(w)
        .modal(true)
        .text(title)
        .secondary_text(detail)
        .build();
    dialog.add_buttons(&[
        ("Отмена", gtk::ResponseType::Cancel),
        (label, gtk::ResponseType::Accept),
    ]);
    dialog.set_default_response(gtk::ResponseType::Cancel);
    dialog.connect_response(move |d, r| {
        if r == gtk::ResponseType::Accept {
            action();
        }
        d.close();
    });
    dialog.present();
}
