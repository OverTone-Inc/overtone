use super::{label, run, run_with};
use gtk::{glib, prelude::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    fckcensor: bool,
    time: bool,
    fonts: bool,
    font: String,
    direct: bool,
    auto_update: bool,
    replacements: BTreeMap<String, String>,
    #[serde(default, skip_serializing)]
    stats: Stats,
}

#[derive(Default, Deserialize)]
struct Stats {
    today: u64,
    week: u64,
    total: u64,
}

fn minutes(seconds: u64) -> String {
    format!("{} ч {} мин", seconds / 3600, (seconds / 60) % 60)
}

fn heading(text: &str) -> gtk::Label {
    let heading = label(text, "launcher-plugin-title");
    heading.set_halign(gtk::Align::Start);
    heading
}

fn note(text: &str) -> gtk::Label {
    let note = label(text, "launcher-plugin-note");
    note.set_wrap(true);
    note.set_xalign(0.0);
    note
}

fn option(container: &gtk::Box, text: &str, caption: &str) -> gtk::Switch {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.append(&heading(text));
    copy.append(&note(caption));
    let toggle = gtk::Switch::new();
    toggle.set_valign(gtk::Align::Center);
    row.append(&copy);
    row.append(&toggle);
    container.append(&row);
    toggle
}

fn pairs(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut result = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let items = line.split_whitespace().collect::<Vec<_>>();
        if items.is_empty() {
            continue;
        }
        if items.len() != 2
            || result
                .insert(items[0].to_owned(), items[1].to_owned())
                .is_some()
        {
            return Err(format!(
                "Строка {} должна содержать одну уникальную пару ID.",
                index + 1
            ));
        }
    }
    Ok(result)
}

#[derive(Clone)]
pub(super) struct Updates {
    status: gtk::Label,
    button: gtk::Button,
    version: gtk::Label,
}

#[derive(Deserialize)]
struct Update {
    available: Option<String>,
    supported: bool,
}

impl Updates {
    pub(super) fn new(version: &gtk::Label) -> Self {
        Self {
            status: note("Проверяем GitHub Releases…"),
            button: gtk::Button::with_label("Проверить обновления"),
            version: version.clone(),
        }
    }

    fn install(&self) {
        self.button.set_sensitive(false);
        self.status.set_label("Загружаем и проверяем пакет…");
        let page = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let result = run("update-install").await.and_then(|json| {
                serde_json::from_str::<Update>(&json).map_err(|error| error.to_string())
            });
            match result {
                Ok(Update { available: Some(version), .. }) => {
                    page.version.set_label(&format!("Версия {version}"));
                    page.status.set_label("Лаунчер обновлён. Новая версия мода применится после закрытия Яндекс Музыки.");
                }
                Ok(_) => page.status.set_label("Установлена актуальная версия."),
                Err(error) => page.status.set_label(&error),
            }
            page.button.set_sensitive(true);
        });
    }

    fn check(&self, automatic: bool) {
        self.button.set_sensitive(false);
        let page = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let result = run("update-check").await.and_then(|json| {
                serde_json::from_str::<Update>(&json).map_err(|error| error.to_string())
            });
            match result {
                Ok(Update {
                    available: Some(_),
                    supported: true,
                }) if automatic => page.install(),
                Ok(Update {
                    available: Some(version),
                    supported: true,
                }) => {
                    page.status
                        .set_label(&format!("Доступна версия {version}."));
                    page.button.set_label("Установить обновление");
                    page.button.set_sensitive(true);
                }
                Ok(Update {
                    available: Some(version),
                    supported: false,
                }) => {
                    page.status.set_label(&format!(
                        "Версия {version} доступна в GitHub Releases. Обновите системный пакет."
                    ));
                    page.button.set_sensitive(true);
                }
                Ok(_) => {
                    page.status.set_label("Установлена актуальная версия.");
                    page.button.set_label("Проверить обновления");
                    page.button.set_sensitive(true);
                }
                Err(_) => {
                    page.status
                        .set_label("Сейчас не удалось проверить обновления.");
                    page.button.set_sensitive(true);
                }
            }
        });
    }
}

pub(super) fn page(updates: Updates) -> gtk::ScrolledWindow {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 12);
    outer.add_css_class("launcher-plugins");

    let replacements = option(
        &outer,
        "FckCensor",
        "Подмена аудио по ID треков Яндекс Музыки.",
    );
    let rules = gtk::TextView::new();
    rules.set_monospace(true);
    rules.set_tooltip_text(Some(
        "Одна пара в строке. Сначала ID оригинала, затем ID замены.",
    ));
    let rules_scroll = gtk::ScrolledWindow::builder()
        .min_content_height(64)
        .max_content_height(105)
        .propagate_natural_height(true)
        .child(&rules)
        .build();
    outer.append(&rules_scroll);
    outer.append(&note(
        "Одна пара в строке. Сначала ID исходного трека, затем ID замены.",
    ));
    let catalog = gtk::Button::with_label("Загрузить каталог метаданных");
    catalog.set_halign(gtk::Align::Start);
    outer.append(&catalog);
    let catalog_status = note("");
    outer.append(&catalog_status);
    catalog.connect_clicked(move |button| {
        let (button, status) = (button.clone(), catalog_status.clone());
        button.set_sensitive(false);
        status.set_label("Загружаем каталог FckCensor…");
        glib::MainContext::default().spawn_local(async move {
            match run("catalog").await {
                Ok(count) => status.set_label(&format!(
                    "Загружено записей {count}. Каталог применяется при следующем запуске."
                )),
                Err(error) => status.set_label(&error),
            }
            button.set_sensitive(true);
        });
    });

    let time = option(
        &outer,
        "YandexMusicTime",
        "Считает время воспроизведения на этом устройстве.",
    );
    let stats = note("Загрузка статистики…");
    outer.append(&stats);

    let fonts = option(
        &outer,
        "Custom Fonts",
        "Выбор установленного шрифта для интерфейса.",
    );
    let dialog = gtk::FontDialog::builder()
        .modal(true)
        .title("Шрифт Яндекс Музыки")
        .build();
    let font = gtk::FontDialogButton::new(Some(dialog));
    font.set_level(gtk::FontLevel::Family);
    font.set_halign(gtk::Align::Start);
    outer.append(&font);

    let direct = option(
        &outer,
        "Яндекс Музыка напрямую",
        "Локальный процесс запускается вместе с клиентом Linux.",
    );
    let auto = option(
        &outer,
        "Автоустановка обновлений",
        "Устанавливает релизы при открытии лаунчера.",
    );
    outer.append(&updates.status);
    updates.button.set_halign(gtk::Align::Start);
    outer.append(&updates.button);

    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let save = gtk::Button::with_label("Сохранить");
    save.add_css_class("suggested-action");
    let result = note("");
    result.set_hexpand(true);
    controls.append(&save);
    controls.append(&result);
    outer.append(&controls);

    outer.set_sensitive(false);
    let widgets = (replacements, rules, time, stats, fonts, font, direct, auto);
    {
        let outer = outer.clone();
        let (replacements, rules, time, stats, fonts, font, direct, auto) = widgets.clone();
        let updates = updates.clone();
        let result = result.clone();
        glib::MainContext::default().spawn_local(async move {
            let loaded = run("plugins").await.and_then(|json| {
                serde_json::from_str::<Settings>(&json).map_err(|error| error.to_string())
            });
            match loaded {
                Ok(config) => {
                    replacements.set_active(config.fckcensor);
                    let list = config
                        .replacements
                        .into_iter()
                        .map(|(a, b)| format!("{a} {b}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    rules.buffer().set_text(&list);
                    time.set_active(config.time);
                    stats.set_label(&format!(
                        "Сегодня {}   ·   Неделя {}   ·   Всего {}",
                        minutes(config.stats.today),
                        minutes(config.stats.week),
                        minutes(config.stats.total)
                    ));
                    fonts.set_active(config.fonts);
                    let mut description = gtk::pango::FontDescription::new();
                    description.set_family(&config.font);
                    font.set_font_desc(&description);
                    direct.set_active(config.direct);
                    auto.set_active(config.auto_update);
                    updates.check(config.auto_update);
                }
                Err(error) => result.set_label(&error),
            }
            outer.set_sensitive(true);
        });
    }

    {
        let status = updates.status.clone();
        let version = updates.version.clone();
        let auto = widgets.7.clone();
        updates.button.connect_clicked(move |button| {
            let control = Updates {
                status: status.clone(),
                button: button.clone(),
                version: version.clone(),
            };
            if button.label().as_deref() == Some("Установить обновление") {
                control.install();
            } else {
                control.check(auto.is_active());
            }
        });
    }
    save.connect_clicked(move |save| {
        let (replacements, rules, time, _, fonts, font, direct, auto) = &widgets;
        let buffer = rules.buffer();
        let map = match pairs(&buffer.text(&buffer.start_iter(), &buffer.end_iter(), false)) {
            Ok(values) => values,
            Err(error) => {
                result.set_label(&error);
                return;
            }
        };
        let family = font
            .font_desc()
            .and_then(|description| description.family())
            .unwrap_or_else(|| "Sans".into());
        let config = Settings {
            fckcensor: replacements.is_active(),
            replacements: map,
            time: time.is_active(),
            fonts: fonts.is_active(),
            font: family.to_string(),
            direct: direct.is_active(),
            auto_update: auto.is_active(),
            stats: Stats::default(),
        };
        let Ok(payload) = serde_json::to_string(&config) else {
            return;
        };
        save.set_sensitive(false);
        let (result, save) = (result.clone(), save.clone());
        glib::MainContext::default().spawn_local(async move {
            match run_with("plugins-save", Some(payload)).await {
                Ok(_) => result.set_label(
                    "Сохранено. Настройки применятся при следующем запуске Яндекс Музыки.",
                ),
                Err(error) => result.set_label(&error),
            }
            save.set_sensitive(true);
        });
    });

    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&outer)
        .build()
}
