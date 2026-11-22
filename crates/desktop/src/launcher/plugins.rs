use super::{label, run, run_with};
use gtk::{glib, prelude::*};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    fckcensor: bool,
    time: bool,
    fonts: bool,
    font: String,
    direct: bool,
    auto_update: bool,
    #[serde(default, skip_serializing)]
    font_file: String,
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

    let fckcensor = option(
        &outer,
        "FckCensor",
        "Автоматически загружает каталог и воспроизводит полную замену, если она есть.",
    );

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
        "Системный шрифт или файл TTF, OTF, WOFF и WOFF2.",
    );
    let dialog = gtk::FontDialog::builder()
        .modal(true)
        .title("Шрифт Яндекс Музыки")
        .build();
    let font = gtk::FontDialogButton::new(Some(dialog));
    font.set_level(gtk::FontLevel::Family);
    font.set_halign(gtk::Align::Start);
    outer.append(&font);
    let file_controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let import = gtk::Button::with_label("Выбрать файл шрифта");
    let clear = gtk::Button::with_label("Использовать системный");
    file_controls.append(&import);
    file_controls.append(&clear);
    outer.append(&file_controls);
    let font_status = note("Используется выбранное системное семейство.");
    outer.append(&font_status);

    let imported = font_status.clone();
    import.connect_clicked(move |button| {
        let parent = button.root().and_downcast::<gtk::Window>();
        let status = imported.clone();
        glib::MainContext::default().spawn_local(async move {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Файлы шрифтов"));
            for ext in ["ttf", "otf", "woff", "woff2"] {
                filter.add_suffix(ext);
            }
            let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder()
                .title("Выберите файл шрифта")
                .filters(&filters)
                .modal(true)
                .build();
            let Ok(file) = dialog.open_future(parent.as_ref()).await else {
                return;
            };
            let Some(path) = file.path() else {
                status.set_label("Выберите файл на этом компьютере.");
                return;
            };
            let Ok(json) = serde_json::to_string(&path) else {
                return;
            };
            match run_with("font-import", Some(json)).await {
                Ok(name) => {
                    status.set_label(&format!("Файл {name} применится при следующем запуске."))
                }
                Err(error) => status.set_label(&error),
            }
        });
    });
    let cleared = font_status.clone();
    clear.connect_clicked(move |_| {
        let status = cleared.clone();
        glib::MainContext::default().spawn_local(async move {
            match run("font-clear").await {
                Ok(_) => status.set_label("При следующем запуске применится системное семейство."),
                Err(error) => status.set_label(&error),
            }
        });
    });

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
    let widgets = (fckcensor, time, stats, fonts, font, direct, auto);
    {
        let outer = outer.clone();
        let (fckcensor, time, stats, fonts, font, direct, auto) = widgets.clone();
        let updates = updates.clone();
        let result = result.clone();
        glib::MainContext::default().spawn_local(async move {
            let loaded = run("plugins").await.and_then(|json| {
                serde_json::from_str::<Settings>(&json).map_err(|error| error.to_string())
            });
            match loaded {
                Ok(config) => {
                    fckcensor.set_active(config.fckcensor);
                    if !config.font_file.is_empty() {
                        font_status.set_label(&format!("Загружен файл {}", config.font_file));
                    }
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
        let auto = widgets.6.clone();
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
        let (fckcensor, time, _, fonts, font, direct, auto) = &widgets;
        let family = font
            .font_desc()
            .and_then(|description| description.family())
            .unwrap_or_else(|| "Sans".into());
        let config = Settings {
            fckcensor: fckcensor.is_active(),
            font_file: String::new(),
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
