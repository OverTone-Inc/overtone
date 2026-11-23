mod plugins;

use gtk::{glib, prelude::*};
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    rc::Rc,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn helper() -> PathBuf {
    let installed = std::env::current_exe().ok().and_then(|path| {
        path.parent()?
            .parent()
            .map(|prefix| prefix.join("share/overtone/yandex/launch.py"))
    });
    installed.filter(|path| path.is_file()).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../integration/yandex/launch.py")
    })
}

async fn run(action: &'static str) -> Result<String, String> {
    run_with(action, None).await
}

async fn run_with(action: &'static str, input: Option<String>) -> Result<String, String> {
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = Command::new("python3")
            .arg(helper())
            .args([action, VERSION])
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
                    let _ = stdin.write_all(input.as_bytes());
                }
                child.wait_with_output()
            })
            .map_err(|error| format!("Не удалось запустить Python. {error}"))
            .and_then(|output| {
                let text = if output.status.success() {
                    String::from_utf8_lossy(&output.stdout).trim().to_owned()
                } else {
                    String::from_utf8_lossy(&output.stderr).trim().to_owned()
                };
                if output.status.success() {
                    Ok(text)
                } else {
                    Err(text)
                }
            });
        let _ = sender.send_blocking(result);
    });
    receiver
        .recv()
        .await
        .map_err(|_| "Операция прервана".to_owned())?
}

fn label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class(class);
    label
}

fn text(value: &str, class: &str) -> gtk::Label {
    let widget = label(value, class);
    widget.set_xalign(0.0);
    widget.set_wrap(true);
    widget
}

fn column(gap: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, gap)
}

fn row(gap: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Horizontal, gap)
}

fn button(value: &str, class: &str) -> gtk::Button {
    let button = gtk::Button::with_label(value);
    button.add_css_class(class);
    button
}

fn page(title: &str, caption: &str) -> (gtk::ScrolledWindow, gtk::Box) {
    let body = column(18);
    body.add_css_class("ot-page");
    let heading = column(6);
    heading.append(&text(title, "ot-page-title"));
    heading.append(&text(caption, "ot-muted"));
    body.append(&heading);
    let scroll = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&body)
        .build();
    (scroll, body)
}

#[derive(Clone, serde::Deserialize)]
struct RustStatus {
    supported: bool,
    installed: Option<String>,
    available: Option<String>,
}

fn artwork() -> gtk::DrawingArea {
    let art = gtk::DrawingArea::new();
    art.set_content_width(225);
    art.set_content_height(200);
    art.set_can_target(false);
    art.set_draw_func(|_, context, width, height| {
        let center = (f64::from(width) * 0.55, f64::from(height) * 0.46);
        let radius = f64::from(width.min(height)) * 0.32;
        context.set_line_width(1.0);
        for stripe in 0..26 {
            let y = f64::from(stripe) * radius * 2.0 / 25.0 - radius;
            let half = (radius * radius - y * y).sqrt();
            let alpha = 0.33 + 0.64 * ((y + radius) / (radius * 2.0));
            context.set_source_rgba(1.0, 0.83, 0.72, alpha);
            context.move_to(center.0 - half, center.1 + y);
            for point in 1..=48 {
                let phase = f64::from(point) / 48.0;
                let x = half * (2.0 * phase - 1.0);
                let wave =
                    (phase * std::f64::consts::TAU).sin() * (1.0 - (y / radius).abs()) * 17.0;
                context.line_to(center.0 + x, center.1 + y + wave);
            }
            let _ = context.stroke();
        }
        context.set_source_rgba(1.0, 0.92, 0.87, 0.85);
        for (dx, dy) in [(0.41, -0.22), (-0.42, 0.31), (0.47, 0.17)] {
            context.arc(
                center.0 + dx * f64::from(width),
                center.1 + dy * f64::from(height),
                1.7,
                0.0,
                std::f64::consts::TAU,
            );
            let _ = context.fill();
        }
    });
    art
}

fn overview() -> gtk::ScrolledWindow {
    let (scroll, body) = page("Обзор", "Клиенты Яндекс Музыки и настройки Overtone.");
    let hero = row(12);
    hero.add_css_class("ot-hero");
    let details = column(11);
    details.set_hexpand(true);
    details.set_valign(gtk::Align::Center);
    let platform = text("RUST / LINUX X86_64", "ot-eyebrow");
    let title = text("Яндекс Музыка\nна Rust", "ot-hero-title");
    let info = text(
        "Отдельный нативный клиент с Discord RPC и текстами LRCLIB.",
        "ot-hero-note",
    );
    let actions = row(8);
    let primary = button("Проверяем сборку…", "ot-primary");
    primary.set_sensitive(false);
    let update = button("Обновить", "ot-hero-alt");
    update.set_visible(false);
    actions.append(&primary);
    actions.append(&update);
    let status = text("", "ot-hero-note");
    details.append(&platform);
    details.append(&title);
    details.append(&info);
    details.append(&actions);
    details.append(&status);
    hero.append(&details);
    hero.append(&artwork());
    body.append(&hero);

    let section = row(10);
    let name = text("Настольный клиент", "ot-section");
    name.set_hexpand(true);
    section.append(&name);
    body.append(&section);
    let card = column(11);
    card.add_css_class("ot-card");
    let stock_head = row(12);
    let icon = gtk::Image::from_icon_name("audio-x-generic-symbolic");
    icon.add_css_class("ot-tile-icon");
    icon.set_pixel_size(22);
    stock_head.append(&icon);
    let copy = column(4);
    copy.set_hexpand(true);
    copy.append(&text("Яндекс Музыка + Overtone", "ot-row-title"));
    let stock_info = text("Проверяем установленный клиент…", "ot-muted");
    copy.append(&stock_info);
    stock_head.append(&copy);
    let stock_open = button("Открыть", "ot-button");
    stock_open.set_sensitive(false);
    stock_head.append(&stock_open);
    card.append(&stock_head);
    let stock_status = text("", "ot-muted");
    stock_status.set_visible(false);
    card.append(&stock_status);
    body.append(&card);

    let rust = Rc::new(std::cell::RefCell::new(None::<RustStatus>));
    let refresh_rust = {
        let (state, primary, update, status) = (
            rust.clone(),
            primary.downgrade(),
            update.downgrade(),
            status.downgrade(),
        );
        Rc::new(move || {
            let (state, primary, update, status) = (
                state.clone(),
                primary.clone(),
                update.clone(),
                status.clone(),
            );
            glib::MainContext::default().spawn_local(async move {
                let (Some(primary), Some(update), Some(status)) =
                    (primary.upgrade(), update.upgrade(), status.upgrade())
                else {
                    return;
                };
                match run("rust-status").await.and_then(|value| {
                    serde_json::from_str::<RustStatus>(&value).map_err(|e| e.to_string())
                }) {
                    Ok(current) => {
                        if let Some(version) = &current.installed {
                            primary.set_label("Открыть Rust-клиент");
                            status.set_label(&format!("Установлена версия {version}"));
                        } else {
                            primary.set_label("Скачать и установить");
                            status.set_label(if current.available.is_some() {
                                "Установка в профиль пользователя"
                            } else if current.supported {
                                "Пакет для этой сборки пока недоступен"
                            } else {
                                "Поддерживается Linux x86_64"
                            });
                        }
                        primary.set_sensitive(
                            current.installed.is_some() || current.available.is_some(),
                        );
                        update.set_visible(
                            current.installed.is_some()
                                && current.available.is_some()
                                && current.installed != current.available,
                        );
                        state.replace(Some(current));
                    }
                    Err(error) => status.set_label(&error),
                }
            });
        })
    };
    for (control, force_install) in [(&primary, false), (&update, true)] {
        let (state, status, refresh, primary, update) = (
            rust.clone(),
            status.downgrade(),
            refresh_rust.clone(),
            primary.downgrade(),
            update.downgrade(),
        );
        control.connect_clicked(move |_| {
            let (Some(primary), Some(update), Some(status)) =
                (primary.upgrade(), update.upgrade(), status.upgrade())
            else {
                return;
            };
            let installed = state
                .borrow()
                .as_ref()
                .is_some_and(|item| item.installed.is_some());
            let action = if installed && !force_install {
                "rust-launch"
            } else {
                "rust-install"
            };
            status.set_label(if action == "rust-install" {
                "Загружаем и проверяем сборку…"
            } else {
                "Открываем приложение…"
            });
            primary.set_sensitive(false);
            update.set_sensitive(false);
            let (status, refresh, primary, update) = (
                status.clone(),
                refresh.clone(),
                primary.clone(),
                update.clone(),
            );
            glib::MainContext::default().spawn_local(async move {
                match run(action).await {
                    Ok(_) => refresh(),
                    Err(error) => {
                        status.set_label(&error);
                        primary.set_sensitive(true);
                    }
                }
                update.set_sensitive(true);
            });
        });
    }
    let refresh_stock = {
        let (open, info, status) = (
            stock_open.downgrade(),
            stock_info.downgrade(),
            stock_status.downgrade(),
        );
        Rc::new(move || {
            let (Some(open), Some(info), Some(status)) =
                (open.upgrade(), info.upgrade(), status.upgrade())
            else {
                return;
            };
            glib::MainContext::default().spawn_local(async move {
                match run("status").await {
                    Ok(value) => {
                        let (version, state) = value.split_once(' ').unwrap_or((&value, "ready"));
                        info.set_label(&format!("Electron {version} · плагины и настройки мода"));
                        open.set_label(match state {
                            "installed" => "Открыть",
                            "update" => "Обновить и открыть",
                            _ => "Установить мод",
                        });
                        open.set_sensitive(true);
                        status.set_visible(false);
                    }
                    Err(error) => {
                        info.set_label("Клиент Electron не найден");
                        status.set_label(&error);
                        status.set_visible(true);
                        open.set_sensitive(false);
                    }
                }
            });
        })
    };
    refresh_stock();
    let (status, refresh) = (stock_status, refresh_stock);
    stock_open.connect_clicked(move |widget| {
        status.set_label("Открываем Яндекс Музыку…");
        status.set_visible(true);
        widget.set_sensitive(false);
        let (status, refresh, widget) = (status.clone(), refresh.clone(), widget.clone());
        glib::MainContext::default().spawn_local(async move {
            match run("launch").await {
                Ok(_) => refresh(),
                Err(error) => {
                    status.set_label(&error);
                    widget.set_sensitive(true);
                }
            }
        });
    });
    scroll.connect_map(move |_| refresh_rust());
    scroll
}

pub fn present(app: &gtk::Application, selected: Option<&str>) {
    if let Some(window) = app
        .windows()
        .into_iter()
        .find(|window| window.widget_name() == "overtone-launcher")
    {
        if let Some(name) = selected
            && let Some(screen) = window.child().and_downcast::<gtk::Box>()
            && let Some(stack) = screen.last_child().and_downcast::<gtk::Stack>()
            && stack.child_by_name(name).is_some()
        {
            stack.set_visible_child_name(name);
        }
        window.present();
        return;
    }
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Overtone")
        .default_width(1110)
        .default_height(690)
        .build();
    window.set_widget_name("overtone-launcher");
    window.set_icon_name(Some(&overtone_core::project().id));
    window.add_css_class("mod-launcher");
    let chrome = gtk::HeaderBar::new();
    chrome.set_title_widget(Some(&label("Overtone", "ot-chrome-label")));
    window.set_titlebar(Some(&chrome));

    let screen = row(0);
    let sidebar = column(8);
    sidebar.add_css_class("ot-sidebar");
    let brand = row(10);
    brand.add_css_class("ot-brand");
    brand.append(&crate::branded_icon());
    brand.append(&text("Overtone", "ot-brand-title"));
    sidebar.append(&brand);

    let stack = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(150)
        .build();
    let version = text(&format!("Версия {VERSION}"), "ot-version");
    stack.add_named(&overview(), Some("overview"));
    stack.add_named(&plugins::catalog(), Some("plugins"));
    stack.add_named(&plugins::activity(), Some("activity"));
    stack.add_named(&plugins::settings(&version), Some("settings"));
    let mut nav = Vec::new();
    for (id, title, icon) in [
        ("overview", "Обзор", "go-home-symbolic"),
        ("plugins", "Плагины", "application-x-addon-symbolic"),
        ("activity", "Статистика", "document-open-recent-symbolic"),
        ("settings", "Настройки", "preferences-system-symbolic"),
    ] {
        let action = gtk::Button::new();
        action.add_css_class("ot-nav");
        let content = row(9);
        let glyph = gtk::Image::from_icon_name(icon);
        glyph.set_valign(gtk::Align::Center);
        content.append(&glyph);
        content.append(&text(title, "ot-nav-label"));
        action.set_child(Some(&content));
        let weak = stack.downgrade();
        action.connect_clicked(move |_| {
            if let Some(stack) = weak.upgrade() {
                stack.set_visible_child_name(id);
            }
        });
        sidebar.append(&action);
        nav.push((id, action));
    }
    stack.connect_visible_child_name_notify(move |stack| {
        for (name, button) in &nav {
            if stack.visible_child_name().as_deref() == Some(*name) {
                button.add_css_class("current");
            } else {
                button.remove_css_class("current");
            }
        }
    });
    let fill = column(0);
    fill.set_vexpand(true);
    sidebar.append(&fill);
    let manager = button("Менеджер плееров", "ot-nav");
    manager.connect_clicked(|_| {
        let _ = Command::new(std::env::current_exe().unwrap_or_else(|_| "overtone".into()))
            .arg("--manager")
            .spawn();
    });
    sidebar.append(&manager);
    sidebar.append(&version);
    screen.append(&sidebar);
    screen.append(&stack);
    if let Some(name) = selected.filter(|name| stack.child_by_name(name).is_some()) {
        stack.set_visible_child_name(name);
    }
    stack.notify("visible-child-name");
    window.set_child(Some(&screen));
    window.present();
}
