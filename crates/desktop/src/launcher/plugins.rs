use super::{button, column, page, row, run, run_with, text};
use gtk::{glib, prelude::*};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Addon {
    id: String,
    name: String,
    author: String,
    category: String,
    description: String,
    url: String,
    icon: String,
    external: bool,
}

fn load(raw: String) -> Result<Value, String> {
    serde_json::from_str(&raw).map_err(|error| error.to_string())
}

fn save(action: &'static str, value: Value, message: &gtk::Label) {
    let message = message.downgrade();
    glib::MainContext::default().spawn_local(async move {
        let result = run_with(action, Some(value.to_string())).await;
        if let Some(message) = message.upgrade() {
            match result {
                Ok(_) => message.set_label("Применится при следующем запуске клиента."),
                Err(error) => message.set_label(&error),
            }
        }
    });
}

fn toggle(action: &'static str, key: &str, value: bool, feedback: &gtk::Label) -> gtk::Switch {
    let widget = gtk::Switch::builder()
        .active(value)
        .valign(gtk::Align::Center)
        .build();
    let (key, weak) = (key.to_owned(), feedback.downgrade());
    widget.connect_active_notify(move |widget| {
        if let Some(feedback) = weak.upgrade() {
            save(action, json!({key.clone(): widget.is_active()}), &feedback);
        }
    });
    widget
}

fn font_controls(card: &gtk::Box, state: &Value, feedback: &gtk::Label) {
    let tools = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(7)
        .row_spacing(7)
        .max_children_per_line(3)
        .build();
    let dialog = gtk::FontDialog::builder()
        .modal(true)
        .title("Шрифт Яндекс Музыки")
        .build();
    let chooser = gtk::FontDialogButton::new(Some(dialog));
    chooser.set_level(gtk::FontLevel::Family);
    let mut description = gtk::pango::FontDescription::new();
    description.set_family(state["font"].as_str().unwrap_or("Sans"));
    chooser.set_font_desc(&description);
    chooser.set_tooltip_text(Some("Системный шрифт"));
    tools.append(&chooser);
    let import = button("Импорт шрифта", "ot-button");
    let clear = button("Сбросить файл", "ot-button");
    let file = text(
        &state["fontFile"]
            .as_str()
            .filter(|name| !name.is_empty())
            .map(|name| format!("Файл: {name}"))
            .unwrap_or_default(),
        "ot-muted",
    );
    file.set_visible(!file.label().is_empty());
    clear.set_sensitive(file.is_visible());
    tools.append(&import);
    tools.append(&clear);
    card.append(&tools);
    card.append(&file);

    let weak = feedback.downgrade();
    chooser.connect_font_desc_notify(move |chooser| {
        if let Some(message) = weak.upgrade()
            && let Some(family) = chooser.font_desc().and_then(|desc| desc.family())
        {
            save("plugins-save", json!({"font": family.as_str()}), &message);
        }
    });
    let (file_name, message, reset) = (file.downgrade(), feedback.downgrade(), clear.downgrade());
    import.connect_clicked(move |widget| {
        let parent = widget.root().and_downcast::<gtk::Window>();
        let (file_name, message, reset) = (file_name.clone(), message.clone(), reset.clone());
        glib::MainContext::default().spawn_local(async move {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Файлы шрифтов"));
            for ext in ["ttf", "otf", "woff", "woff2"] {
                filter.add_suffix(ext);
            }
            let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder()
                .title("Выбрать файл шрифта")
                .filters(&filters)
                .modal(true)
                .build();
            let Ok(selection) = dialog.open_future(parent.as_ref()).await else {
                return;
            };
            let Some(path) = selection.path() else {
                return;
            };
            let Ok(value) = serde_json::to_string(&path) else {
                return;
            };
            match run_with("font-import", Some(value)).await {
                Ok(name) => {
                    if let Some(file) = file_name.upgrade() {
                        file.set_label(&format!("Файл: {name}"));
                        file.set_visible(true);
                    }
                    if let Some(reset) = reset.upgrade() {
                        reset.set_sensitive(true);
                    }
                    if let Some(message) = message.upgrade() {
                        message.set_label("Шрифт загружен.");
                    }
                }
                Err(error) => {
                    if let Some(message) = message.upgrade() {
                        message.set_label(&error);
                    }
                }
            }
        });
    });
    let (file_name, message) = (file.downgrade(), feedback.downgrade());
    clear.connect_clicked(move |clear| {
        let (file_name, message, clear) = (file_name.clone(), message.clone(), clear.downgrade());
        glib::MainContext::default().spawn_local(async move {
            match run("font-clear").await {
                Ok(_) => {
                    if let Some(file) = file_name.upgrade() {
                        file.set_visible(false);
                    }
                    if let Some(clear) = clear.upgrade() {
                        clear.set_sensitive(false);
                    }
                    if let Some(message) = message.upgrade() {
                        message.set_label("Восстановлен системный шрифт.");
                    }
                }
                Err(error) => {
                    if let Some(message) = message.upgrade() {
                        message.set_label(&error);
                    }
                }
            }
        });
    });
}

pub(super) fn catalog() -> gtk::ScrolledWindow {
    let (screen, body) = page("Плагины", "Каталог дополнений для Яндекс Музыки.");
    let feedback = text("", "ot-feedback");
    let list = column(9);
    body.append(&list);
    body.append(&feedback);
    let weak = (list.downgrade(), feedback.downgrade());
    screen.connect_map(move |_| {
        let (list, message) = weak.clone();
        glib::MainContext::default().spawn_local(async move {
            let state = run("plugins").await.and_then(load);
            let (Some(list), Some(message)) = (list.upgrade(), message.upgrade()) else {
                return;
            };
            match state {
                Ok(state) => populate(&list, &message, &state),
                Err(error) => message.set_label(&error),
            }
        });
    });
    screen
}

fn populate(list: &gtk::Box, message: &gtk::Label, state: &Value) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let entries: Vec<Addon> =
        serde_json::from_str(include_str!("../../../../data/yandex-addons.json"))
            .expect("valid built-in add-on catalog");
    for addon in entries {
        let card = column(12);
        card.add_css_class("ot-card");
        let line = row(12);
        let icon = gtk::Image::from_icon_name(&addon.icon);
        icon.set_pixel_size(19);
        icon.set_valign(gtk::Align::Start);
        icon.add_css_class("ot-tile-icon");
        line.append(&icon);
        let details = column(4);
        details.set_hexpand(true);
        let name = text(&addon.name, "ot-row-title");
        details.append(&name);
        let author = format!("{} · {}", addon.author, addon.category);
        if addon.url.is_empty() {
            details.append(&text(&author, "ot-meta"));
        } else {
            let link = gtk::LinkButton::with_label(&addon.url, &author);
            link.add_css_class("ot-source");
            link.set_halign(gtk::Align::Start);
            link.set_tooltip_text(Some("Исходный код"));
            details.append(&link);
        }
        details.append(&text(&addon.description, "ot-muted"));
        line.append(&details);
        let active = state[&addon.id].as_bool().unwrap_or(false);
        if !addon.external || state["addons"][&addon.id].is_string() {
            let control = toggle("plugins-save", &addon.id, active, message);
            control.update_property(&[gtk::accessible::Property::Label(&addon.name)]);
            line.append(&control);
        } else {
            let install = button("Установить", "ot-button");
            install.set_valign(gtk::Align::Center);
            let (card, list, msg) = (card.downgrade(), list.downgrade(), message.downgrade());
            let id = addon.id.clone();
            let name = addon.name.clone();
            install.connect_clicked(move |install| {
                install.set_sensitive(false);
                install.set_label("Сборка…");
                let (card, list, msg, install, id, name) = (
                    card.clone(),
                    list.clone(),
                    msg.clone(),
                    install.downgrade(),
                    id.clone(),
                    name.clone(),
                );
                glib::MainContext::default().spawn_local(async move {
                    let result = run_with("addon-install", Some(json!(id).to_string())).await;
                    if let Some(message) = msg.upgrade() {
                        match result {
                            Ok(version) => {
                                message.set_label(&format!(
                                    "{name} {version} установлен. Перезапусти Яндекс Музыку."
                                ));
                                if card.upgrade().is_some()
                                    && let Some(list) = list.upgrade()
                                    && let Ok(state) = run("plugins").await.and_then(load)
                                {
                                    populate(&list, &message, &state);
                                }
                            }
                            Err(error) => {
                                message.set_label(&error);
                                if let Some(install) = install.upgrade() {
                                    install.set_label("Повторить");
                                    install.set_sensitive(true);
                                }
                            }
                        }
                    }
                });
            });
            line.append(&install);
        }
        card.append(&line);
        if addon.id == "fonts" {
            font_controls(&card, state, message);
        }
        if addon.id == "spicy" {
            card.append(&text("Electron · Яндекс Музыка и LRCLIB", "ot-meta"));
        }
        if addon.id == "time" {
            let caption = if let Some(version) = state["addons"]["time"].as_str() {
                format!("Оригинальный аддон {version} · Electron")
            } else {
                "Electron · локальная сборка из исходников автора".to_owned()
            };
            card.append(&text(&caption, "ot-meta"));
        }
        list.append(&card);
    }
}

fn duration(seconds: u64) -> String {
    if seconds < 3600 {
        format!("{} мин", seconds / 60)
    } else {
        format!("{} ч {:02} мин", seconds / 3600, (seconds / 60) % 60)
    }
}

pub(super) fn activity() -> gtk::ScrolledWindow {
    let (screen, body) = page(
        "Статистика",
        "Время прослушивания Яндекс Музыки на этом компьютере.",
    );
    let totals = gtk::Grid::builder()
        .column_homogeneous(true)
        .column_spacing(10)
        .row_spacing(10)
        .build();
    let mut values = Vec::new();
    for (index, (key, title)) in [
        ("today", "Сегодня"),
        ("week", "Эта неделя"),
        ("month", "Этот месяц"),
        ("total", "Всё время"),
    ]
    .into_iter()
    .enumerate()
    {
        let card = column(9);
        card.add_css_class("ot-card");
        let value = text("…", "ot-count");
        card.append(&text(title, "ot-muted"));
        card.append(&value);
        totals.attach(&card, (index % 2) as i32, (index / 2) as i32, 1, 1);
        values.push((key, value));
    }
    body.append(&totals);
    let days = column(8);
    days.add_css_class("ot-card");
    body.append(&days);
    let controls = row(10);
    let refresh = button("Обновить статистику", "ot-button");
    let status = text("", "ot-feedback");
    controls.append(&refresh);
    controls.append(&status);
    body.append(&controls);

    let reload = {
        let (days, status) = (days.downgrade(), status.downgrade());
        let values: Vec<_> = values
            .into_iter()
            .map(|(name, widget)| (name, widget.downgrade()))
            .collect();
        move || {
            let (values, days, status) = (values.clone(), days.clone(), status.clone());
            glib::MainContext::default().spawn_local(async move {
                let data = run("plugins").await.and_then(load);
                let (Some(days), Some(status)) = (days.upgrade(), status.upgrade()) else {
                    return;
                };
                let data = match data {
                    Ok(data) => data,
                    Err(error) => {
                        status.set_label(&error);
                        return;
                    }
                };
                status.set_label("");
                while let Some(child) = days.first_child() {
                    days.remove(&child);
                }
                days.append(&text("Последние 7 дней", "ot-row-title"));
                let stats = &data["stats"];
                for (key, widget) in values {
                    if let Some(widget) = widget.upgrade() {
                        widget.set_label(&duration(stats[key].as_u64().unwrap_or(0)));
                    }
                }
                if let (Some(current), Some(best)) = (
                    stats["streak"]["current"].as_u64(),
                    stats["streak"]["best"].as_u64(),
                ) {
                    days.append(&text(
                        &format!("Серия входов: {current} дн. · Рекорд: {best} дн."),
                        "ot-muted",
                    ));
                }
                if let Some(items) = stats["days"].as_object() {
                    let max = items
                        .values()
                        .filter_map(Value::as_u64)
                        .max()
                        .unwrap_or(0)
                        .max(1);
                    for (date, seconds) in items {
                        let row = row(12);
                        let day = text(&date.get(5..).unwrap_or(date).replace('-', "."), "ot-meta");
                        day.set_width_chars(6);
                        let progress = gtk::ProgressBar::builder()
                            .hexpand(true)
                            .valign(gtk::Align::Center)
                            .fraction(seconds.as_u64().unwrap_or(0) as f64 / max as f64)
                            .build();
                        progress.update_property(&[gtk::accessible::Property::Label(date)]);
                        let count = text(&duration(seconds.as_u64().unwrap_or(0)), "ot-meta");
                        count.set_width_chars(12);
                        count.set_xalign(1.0);
                        row.append(&day);
                        row.append(&progress);
                        row.append(&count);
                        days.append(&row);
                    }
                }
            });
        }
    };
    let on_map = reload.clone();
    screen.connect_map(move |_| on_map());
    refresh.connect_clicked(move |_| reload());
    screen
}

fn setting_row(parent: &gtk::Box, title: &str, description: &str, widget: &impl IsA<gtk::Widget>) {
    let line = row(14);
    let copy = column(4);
    copy.set_hexpand(true);
    copy.append(&text(title, "ot-row-title"));
    copy.append(&text(description, "ot-muted"));
    line.append(&copy);
    widget.as_ref().set_valign(gtk::Align::Center);
    line.append(widget);
    parent.append(&line);
}

fn action(parent: &gtk::Box, label: &str, command: &'static str, result: &gtk::Label) {
    let check = button(label, "ot-button");
    check.set_halign(gtk::Align::Start);
    parent.append(&check);
    let result = result.downgrade();
    check.connect_clicked(move |widget| {
        widget.set_sensitive(false);
        let (widget, result) = (widget.downgrade(), result.clone());
        glib::MainContext::default().spawn_local(async move {
            let response = run(command).await;
            if let Some(result) = result.upgrade() {
                result.set_label(&response.err().unwrap_or_else(|| "Готово.".to_owned()));
            }
            if let Some(widget) = widget.upgrade() {
                widget.set_sensitive(true);
            }
        });
    });
}

pub(super) fn settings(version: &gtk::Label) -> gtk::ScrolledWindow {
    let (screen, body) = page(
        "Настройки",
        "Подключение, обновления и обслуживание клиентов.",
    );
    let card = column(13);
    card.add_css_class("ot-card");
    let result = text("", "ot-feedback");
    body.append(&card);
    let updates = column(9);
    updates.add_css_class("ot-card");
    updates.append(&text("Обновления Overtone", "ot-row-title"));
    let check = button("Проверить обновления", "ot-button");
    check.set_halign(gtk::Align::Start);
    updates.append(&check);
    body.append(&updates);
    let clients = column(10);
    clients.add_css_class("ot-card");
    clients.append(&text("Управление установками", "ot-row-title"));
    let tools = row(8);
    action(&tools, "Восстановить Electron", "restore", &result);
    action(&tools, "Удалить Rust-клиент", "rust-remove", &result);
    clients.append(&tools);
    body.append(&clients);
    body.append(&result);

    let version = version.downgrade();
    let check_for_updates = {
        let (check, result) = (check.downgrade(), result.downgrade());
        move |install: bool| {
            let (Some(check), Some(result)) = (check.upgrade(), result.upgrade()) else {
                return;
            };
            check.set_sensitive(false);
            result.set_label(if install {
                "Проверяем пакет обновления…"
            } else {
                "Проверяем GitHub Releases…"
            });
            let version = version.clone();
            glib::MainContext::default().spawn_local(async move {
                match run(if install {
                    "update-install"
                } else {
                    "update-check"
                })
                .await
                .and_then(load)
                {
                    Ok(state) => match state["available"].as_str() {
                        Some(number) if install => {
                            if let Some(version) = version.upgrade() {
                                version.set_label(&format!("Версия {number}"));
                            }
                            result.set_label("Обновление установлено. Перезапусти Overtone.");
                            check.set_label("Проверить обновления");
                        }
                        Some(number) if state["supported"].as_bool() == Some(true) => {
                            result.set_label(&format!("Доступна версия {number}."));
                            check.set_label("Установить обновление");
                        }
                        Some(number) => result
                            .set_label(&format!("Версия {number} доступна в менеджере пакетов.")),
                        None => {
                            result.set_label("Установлена актуальная версия.");
                            check.set_label("Проверить обновления");
                        }
                    },
                    Err(error) => result.set_label(&error),
                }
                check.set_sensitive(true);
            });
        }
    };
    let manual = check_for_updates.clone();
    check.connect_clicked(move |button| {
        manual(button.label().as_deref() == Some("Установить обновление"))
    });
    let (card, result) = (card.downgrade(), result.downgrade());
    glib::MainContext::default().spawn_local(async move {
        let (Some(card), Some(result)) = (card.upgrade(), result.upgrade()) else {
            return;
        };
        match run("plugins").await.and_then(load) {
            Ok(state) => {
                let mod_settings = &state["mod"];
                let discord = toggle(
                    "mod-settings-save",
                    "enabled",
                    mod_settings["enabled"].as_bool().unwrap_or(false),
                    &result,
                );
                setting_row(
                    &card,
                    "Discord Rich Presence",
                    "Показывает текущий трек в профиле Discord.",
                    &discord,
                );
                let sources = ["yandex", "external", "fallback", "prefer"];
                let lyrics = gtk::DropDown::from_strings(&[
                    "Яндекс Музыка",
                    "LRCLIB",
                    "Сначала Яндекс Музыка",
                    "Сначала LRCLIB",
                ]);
                let index = sources
                    .iter()
                    .position(|mode| Some(*mode) == mod_settings["lyricsMode"].as_str())
                    .unwrap_or(0);
                lyrics.set_selected(index as u32);
                let weak = result.downgrade();
                lyrics.connect_selected_notify(move |control| {
                    if let Some(source) = sources.get(control.selected() as usize)
                        && let Some(result) = weak.upgrade()
                    {
                        save("mod-settings-save", json!({"lyricsMode": source}), &result);
                    }
                });
                setting_row(
                    &card,
                    "Источник текстов",
                    "Используется обоими клиентами при следующем запуске.",
                    &lyrics,
                );
                let direct = toggle(
                    "plugins-save",
                    "direct",
                    state["direct"].as_bool().unwrap_or(true),
                    &result,
                );
                setting_row(
                    &card,
                    "Прямой маршрут",
                    "Направляет соединения Electron-клиента через физический интерфейс Linux.",
                    &direct,
                );
                let automatic = state["autoUpdate"].as_bool().unwrap_or(true);
                let auto = toggle("plugins-save", "autoUpdate", automatic, &result);
                setting_row(
                    &card,
                    "Автообновление",
                    "Загружает стабильные сборки Overtone из GitHub Releases.",
                    &auto,
                );
                if automatic {
                    check_for_updates(true);
                }
            }
            Err(error) => result.set_label(&error),
        }
    });
    screen
}
