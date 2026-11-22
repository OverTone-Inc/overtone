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

pub fn present(app: &gtk::Application) {
    if let Some(window) = app
        .windows()
        .into_iter()
        .find(|window| window.widget_name() == "overtone-launcher")
    {
        window.present();
        return;
    }
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Overtone · Яндекс Музыка")
        .default_width(690)
        .default_height(485)
        .build();
    window.set_widget_name("overtone-launcher");
    window.set_icon_name(Some(&overtone_core::project().id));
    window.add_css_class("mod-launcher");

    let screen = gtk::Box::new(gtk::Orientation::Vertical, 0);
    screen.add_css_class("launcher-screen");

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 9);
    top.add_css_class("launcher-top");
    let logo = crate::branded_icon();
    let name = label("Overtone", "launcher-wordmark");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let version = label(&format!("Версия {VERSION}"), "launcher-version");
    top.append(&logo);
    top.append(&name);
    top.append(&spacer);
    top.append(&version);
    screen.append(&top);
    let stack = gtk::Stack::builder()
        .vexpand(true)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    let tabs = gtk::StackSwitcher::new();
    tabs.set_halign(gtk::Align::Center);
    tabs.add_css_class("launcher-tabs");
    tabs.set_stack(Some(&stack));
    screen.append(&tabs);
    let overview = gtk::Box::new(gtk::Orientation::Vertical, 0);
    stack.add_titled(&overview, Some("overview"), "Обзор");
    let addons = plugins::page(plugins::Updates::new(&version));
    stack.add_titled(&addons, Some("plugins"), "Плагины и обновления");
    screen.append(&stack);

    let hero = gtk::Box::new(gtk::Orientation::Vertical, 13);
    hero.add_css_class("launcher-hero");
    hero.set_vexpand(true);
    hero.set_valign(gtk::Align::Center);
    let status = label("Поиск Яндекс Музыки…", "launcher-status");
    status.set_halign(gtk::Align::Center);
    let title = label("Яндекс Музыка\nс Overtone", "launcher-title");
    title.set_justify(gtk::Justification::Center);
    let subtitle = label(
        "Discord RPC и тексты из LRCLIB.\nНастройки доступны в боковом меню Яндекс Музыки.",
        "launcher-subtitle",
    );
    subtitle.set_justify(gtk::Justification::Center);
    subtitle.set_wrap(true);
    hero.append(&status);
    hero.append(&title);
    hero.append(&subtitle);

    let launch = gtk::Button::new();
    let button = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let button_text = gtk::Label::new(Some("Открыть Яндекс Музыку"));
    let arrow = label("→", "launcher-arrow");
    button.append(&button_text);
    button.append(&arrow);
    launch.set_child(Some(&button));
    launch.set_halign(gtk::Align::Center);
    launch.set_sensitive(false);
    launch.add_css_class("launcher-launch");
    hero.append(&launch);
    overview.append(&hero);

    let message = label("", "launcher-message");
    message.set_wrap(true);
    message.set_justify(gtk::Justification::Center);
    message.set_selectable(true);
    overview.append(&message);

    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    controls.add_css_class("launcher-controls");
    controls.set_halign(gtk::Align::Center);
    let check = gtk::Button::with_label("Проверить ещё раз");
    check.add_css_class("launcher-check");
    check.set_visible(false);
    let restore = gtk::Button::with_label("Удалить мод");
    restore.add_css_class("launcher-check");
    restore.set_visible(false);
    controls.append(&check);
    controls.append(&restore);
    overview.append(&controls);
    window.set_child(Some(&screen));

    let refresh = {
        let launch = launch.downgrade();
        let button_text = button_text.downgrade();
        let check = check.downgrade();
        let restore = restore.downgrade();
        let status = status.downgrade();
        let message = message.downgrade();
        Rc::new(move || {
            let Some(launch) = launch.upgrade() else {
                return;
            };
            let Some(button_text) = button_text.upgrade() else {
                return;
            };
            let Some(check) = check.upgrade() else { return };
            let Some(restore) = restore.upgrade() else {
                return;
            };
            let Some(status) = status.upgrade() else {
                return;
            };
            let Some(message) = message.upgrade() else {
                return;
            };
            launch.set_sensitive(false);
            check.set_visible(false);
            restore.set_visible(false);
            status.set_label("Проверяем Яндекс Музыку…");
            message.set_label("");
            glib::MainContext::default().spawn_local(async move {
                match run("status").await {
                    Ok(value) => {
                        let (version, state) = value.split_once(' ').unwrap_or((&value, "ready"));
                        let (heading, action) = match state {
                            "installed" => {
                                ("Overtone установлен".to_owned(), "Открыть Яндекс Музыку")
                            }
                            "update" => {
                                ("Доступно обновление мода".to_owned(), "Обновить и открыть")
                            }
                            _ => (
                                format!("Яндекс Музыка {version} найдена"),
                                "Установить и открыть",
                            ),
                        };
                        status.set_label(&heading);
                        button_text.set_label(action);
                        launch.set_sensitive(true);
                        restore.set_visible(state != "ready");
                        restore.set_sensitive(true);
                    }
                    Err(error) => {
                        status.set_label("Требуется проверка клиента");
                        check.set_visible(true);
                        message.set_label(&error);
                    }
                }
            });
        })
    };
    refresh();
    let check_again = refresh.clone();
    check.connect_clicked(move |_| check_again());

    let weak = window.downgrade();
    let launch_status = status.clone();
    let launch_message = message.clone();
    let restore_control = restore.clone();
    launch.connect_clicked(move |launch| {
        launch.set_sensitive(false);
        restore_control.set_sensitive(false);
        button_text.set_label("Открываем…");
        launch_status.set_label("Подключаем Overtone");
        launch_message.set_label("");
        let (launch, button_text, status, message, restore, weak) = (
            launch.clone(),
            button_text.clone(),
            launch_status.clone(),
            launch_message.clone(),
            restore_control.clone(),
            weak.clone(),
        );
        glib::MainContext::default().spawn_local(async move {
            match run("launch").await {
                Ok(_) => {
                    if let Some(window) = weak.upgrade() {
                        window.close();
                    }
                }
                Err(error) => {
                    status.set_label("Запуск не подтверждён");
                    message.set_label(&error);
                    button_text.set_label("Повторить запуск");
                    launch.set_sensitive(true);
                    restore.set_sensitive(true);
                }
            }
        });
    });
    let launch_control = launch.downgrade();
    restore.connect_clicked(move |restore| {
        let Some(launch) = launch_control.upgrade() else {
            return;
        };
        launch.set_sensitive(false);
        restore.set_sensitive(false);
        status.set_label("Удаляем мод и возвращаем исходный архив…");
        message.set_label("");
        let (restore, status, message, refresh) = (
            restore.clone(),
            status.clone(),
            message.clone(),
            refresh.clone(),
        );
        glib::MainContext::default().spawn_local(async move {
            match run("restore").await {
                Ok(_) => refresh(),
                Err(error) => {
                    status.set_label("Исходный архив не восстановлен");
                    message.set_label(&error);
                    launch.set_sensitive(true);
                    restore.set_sensitive(true);
                }
            }
        });
    });
    window.present();
}
