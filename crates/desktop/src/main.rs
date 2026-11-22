mod art;
mod launcher;
mod manager;
mod x11;

use art::Art;
use gtk::{gio, glib, prelude::*};
use overtone_core::{Source, project};
use overtone_service::{Request, Response, State, call, connect, ensure, receive};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::mpsc,
};

const ICON: &[u8] = include_bytes!("../../../assets/icons/overtone-app-64.png");

struct Job(Request, Box<dyn FnOnce(Result<Response, String>)>);
struct ClientJob(Request, async_channel::Sender<Result<Response, String>>);
struct Hub {
    state: RefCell<State>,
    manager: RefCell<Option<Rc<manager::Manager>>>,
    mini: RefCell<Option<Rc<Mini>>>,
    app: glib::WeakRef<gtk::Application>,
    jobs: mpsc::Sender<ClientJob>,
    started: Cell<bool>,
}
impl Hub {
    fn new(app: &gtk::Application) -> Rc<Self> {
        let (send, recv) = mpsc::channel::<ClientJob>();
        std::thread::spawn(move || {
            while let Ok(ClientJob(req, output)) = recv.recv() {
                let response = if matches!(&req, Request::Rescan) {
                    ensure().and_then(|()| call(&req))
                } else {
                    call(&req)
                }
                .map_err(|e| e.to_string());
                let _ = output.send_blocking(response);
            }
        });
        Rc::new(Self {
            state: Default::default(),
            manager: Default::default(),
            mini: Default::default(),
            app: app.downgrade(),
            jobs: send,
            started: Cell::new(false),
        })
    }
    fn start(self: &Rc<Self>) {
        if self.started.replace(true) {
            return;
        }
        let (send, recv) = async_channel::bounded::<Result<State, String>>(8);
        std::thread::spawn(move || {
            if let Err(e) = ensure() {
                let _ = send.send_blocking(Err(e.to_string()));
                return;
            }
            loop {
                match connect(&Request::Subscribe) {
                    Ok(mut reader) => {
                        while let Ok(Some(msg)) = receive(&mut reader) {
                            if let Some(state) = msg.state
                                && send.send_blocking(Ok(state)).is_err()
                            {
                                return;
                            }
                        }
                    }
                    Err(_) => {
                        let _ = send.send_blocking(Err(
                            "Фоновый процесс отключён. Закройте окно и запустите Overtone снова."
                                .into(),
                        ));
                        break;
                    }
                }
            }
        });
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            while let Ok(value) = recv.recv().await {
                let Some(hub) = weak.upgrade() else { break };
                match value {
                    Ok(s) => {
                        if let Some(view) = hub.manager.borrow().as_ref() {
                            view.update(&s, &hub.state.borrow());
                        }
                        if let Some(mini) = hub.mini.borrow().as_ref() {
                            mini.update(&s);
                        }
                        hub.state.replace(s);
                    }
                    Err(msg) => {
                        if let Some(view) = hub.manager.borrow().as_ref() {
                            view.feedback(&msg, true);
                        }
                        if let Some(view) = hub.mini.borrow().as_ref() {
                            view.title.set_label(&msg);
                        }
                        hub.started.set(false);
                    }
                }
            }
        });
    }
    fn job(&self, job: Job) {
        let Job(req, callback) = job;
        let (send, recv) = async_channel::bounded(1);
        if self.jobs.send(ClientJob(req, send)).is_err() {
            callback(Err("Очередь команд недоступна".into()));
            return;
        }
        glib::MainContext::default().spawn_local(async move {
            callback(
                recv.recv()
                    .await
                    .unwrap_or_else(|_| Err("Операция прервана".into())),
            );
        });
    }
    fn request(self: &Rc<Self>, req: Request) {
        if matches!(req, Request::Rescan) {
            self.start();
        }
        let weak = Rc::downgrade(self);
        self.job(Job(
            req,
            Box::new(move |r| {
                if let Some(h) = weak.upgrade()
                    && let Some(view) = h.manager.borrow().as_ref()
                {
                    match r {
                        Ok(_) => view.feedback("", false),
                        Err(msg) => view.feedback(&msg, true),
                    }
                }
            }),
        ));
    }
    fn open_manager(self: &Rc<Self>, page: Option<&str>) {
        self.start();
        if let Some(view) = self.manager.borrow().as_ref() {
            if let Some(page) = page {
                view.open(page);
            }
            view.window.present();
            return;
        }
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let view = manager::Manager::new(&app, self);
        let weak = Rc::downgrade(self);
        view.window.connect_close_request(move |_| {
            if let Some(hub) = weak.upgrade() {
                hub.manager.borrow_mut().take();
            }
            glib::Propagation::Proceed
        });
        view.update(&self.state.borrow(), &State::default());
        if let Some(page) = page {
            view.open(page);
        }
        view.window.present();
        self.manager.replace(Some(view));
    }
    fn open_mini(self: &Rc<Self>) {
        self.start();
        if let Some(view) = self.mini.borrow().as_ref() {
            view.window.present();
            return;
        }
        if !self.state.borrow().enabled("mini") {
            return;
        }
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let view = Mini::new(&app, self);
        let weak = Rc::downgrade(self);
        view.window.connect_close_request(move |w| {
            if x11::supported(w) {
                let _ = x11::save(w);
            }
            if let Some(hub) = weak.upgrade() {
                hub.mini.borrow_mut().take();
            }
            glib::Propagation::Proceed
        });
        view.update(&self.state.borrow());
        view.window.present();
        self.mini.replace(Some(view));
    }
    fn save_mini(&self) {
        if let Some(mini) = self.mini.borrow().as_ref()
            && x11::supported(&mini.window)
        {
            let _ = x11::save(&mini.window);
        }
    }
    fn quit(self: &Rc<Self>) {
        self.save_mini();
        let weak = self.app.clone();
        self.job(Job(
            Request::Quit,
            Box::new(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.quit();
                }
            }),
        ));
    }
}

fn panel(orientation: gtk::Orientation, spacing: i32) -> gtk::Box {
    gtk::Box::new(orientation, spacing)
}
fn h(spacing: i32) -> gtk::Box {
    panel(gtk::Orientation::Horizontal, spacing)
}
fn v(spacing: i32) -> gtk::Box {
    panel(gtk::Orientation::Vertical, spacing)
}
fn text(s: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(s));
    l.set_xalign(0.);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    if !class.is_empty() {
        l.add_css_class(class);
    }
    l
}
fn clipped(s: &str, class: &str) -> gtk::Label {
    let l = text(s, class);
    l.set_wrap(false);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_max_width_chars(1);
    l
}
fn button(label: &str) -> gtk::Button {
    gtk::Button::with_label(label)
}
fn action(parent: &gtk::Box, label: &str, hub: &Rc<Hub>, req: Request) -> gtk::Button {
    let b = button(label);
    let hub = Rc::downgrade(hub);
    b.connect_clicked(move |_| {
        if let Some(h) = hub.upgrade() {
            h.request(req.clone());
        }
    });
    parent.append(&b);
    b
}
fn branded_icon() -> gtk::Image {
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_static(ICON));
    let pic = gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale(
        &stream,
        24,
        24,
        true,
        gio::Cancellable::NONE,
    )
    .ok();
    gtk::Image::from_pixbuf(pic.as_ref())
}
fn setup_window(
    app: &gtk::Application,
    title: &str,
    size: (i32, i32),
) -> (gtk::ApplicationWindow, gtk::HeaderBar) {
    let size = gtk::gdk::Display::default()
        .and_then(|d| d.monitors().item(0).and_downcast::<gtk::gdk::Monitor>())
        .map(|m| {
            (
                size.0.min(m.geometry().width() - 36).max(360),
                size.1.min((m.geometry().height() - 98).max(240)),
            )
        })
        .unwrap_or(size);
    let w = gtk::ApplicationWindow::builder()
        .application(app)
        .title(title)
        .default_width(size.0)
        .default_height(size.1)
        .build();
    w.set_icon_name(Some(&project().id));
    w.add_css_class("overtone");
    let bar = gtk::HeaderBar::new();
    bar.set_decoration_layout(Some(":minimize,maximize,close"));
    let brand = h(8);
    brand.append(&branded_icon());
    let title_label = text(title, "strong");
    title_label.set_wrap(false);
    brand.append(&title_label);
    bar.set_title_widget(Some(&brand));
    w.set_titlebar(Some(&bar));
    (w, bar)
}
fn playback(p: Option<&overtone_player::Player>) -> (&'static str, bool, bool) {
    let Some(p) = p else {
        return ("Воспроизвести", false, false);
    };
    if p.status == "Playing" {
        ("Пауза", p.control && p.pause, p.control && p.next)
    } else {
        ("Воспроизвести", p.control && p.play, p.control && p.next)
    }
}
fn friendly_status(status: &str) -> &str {
    match status {
        "Playing" => "Воспроизводится",
        "Paused" => "На паузе",
        "Stopped" => "Остановлено",
        _ => "Состояние неизвестно",
    }
}
fn sibling(name: &str) -> PathBuf {
    std::env::current_exe()
        .unwrap_or_default()
        .with_file_name(name)
}
fn drawable(window: &gtk::ApplicationWindow) -> bool {
    window.is_mapped()
        && window
            .surface()
            .and_then(|surface| surface.dynamic_cast::<gtk::gdk::Toplevel>().ok())
            .is_some_and(|toplevel| {
                !toplevel
                    .state()
                    .contains(gtk::gdk::ToplevelState::MINIMIZED)
            })
}
fn watch_visibility(window: &gtk::ApplicationWindow, update: impl Fn() + 'static) {
    let update = Rc::new(update);
    let mapped = update.clone();
    window.connect_map(move |_| mapped());
    window.connect_realize(move |w| {
        if let Some(toplevel) = w
            .surface()
            .and_then(|surface| surface.dynamic_cast::<gtk::gdk::Toplevel>().ok())
        {
            let update = update.clone();
            toplevel.connect_state_notify(move |_| update());
        }
    });
}
fn choose_files(
    window: &gtk::ApplicationWindow,
    title: &str,
    many: bool,
    callback: impl Fn(Vec<PathBuf>) + 'static,
) {
    let dialog = gtk::FileDialog::builder()
        .title(title)
        .modal(true)
        .accept_label("Выбрать")
        .build();
    if many {
        dialog.open_multiple(Some(window), gio::Cancellable::NONE, move |result| {
            if let Ok(files) = result {
                let paths = (0..files.n_items())
                    .filter_map(|i| files.item(i).and_downcast::<gio::File>()?.path())
                    .collect();
                callback(paths);
            }
        });
    } else {
        dialog.open(Some(window), gio::Cancellable::NONE, move |result| {
            if let Some(path) = result.ok().and_then(|file| file.path()) {
                callback(vec![path]);
            }
        });
    }
}

struct Mini {
    window: gtk::ApplicationWindow,
    title: gtk::Label,
    subtitle: gtk::Label,
    art: Art,
    play: gtk::Button,
    next: gtk::Button,
    above: Cell<bool>,
}
impl Mini {
    fn new(app: &gtk::Application, hub: &Rc<Hub>) -> Rc<Self> {
        let (window, bar) =
            setup_window(app, &format!("{} · Мини-плеер", project().name), (440, 100));
        window.set_resizable(false);
        let row = h(11);
        row.add_css_class("mini-shell");
        let art = Art::new(44);
        row.append(&art.image);
        let titles = v(2);
        titles.set_valign(gtk::Align::Center);
        titles.set_hexpand(true);
        let title = clipped("Плеер не подключён", "strong");
        let subtitle = clipped("Выберите плеер в менеджере", "subtle");
        titles.append(&title);
        titles.append(&subtitle);
        row.append(&titles);
        let play = action(&row, "Воспроизвести", hub, Request::PlayPause);
        let next = action(&row, "Далее", hub, Request::Next);
        let open = button("Менеджер");
        bar.pack_start(&open);
        let weak = Rc::downgrade(hub);
        open.connect_clicked(move |_| {
            if let Some(hub) = weak.upgrade() {
                hub.open_manager(None);
            }
        });
        window.set_child(Some(&row));
        let view = Rc::new(Self {
            window,
            title,
            subtitle,
            art,
            play,
            next,
            above: Cell::new(hub.state.borrow().above),
        });
        {
            let weak = Rc::downgrade(&view);
            view.window.connect_map(move |_| {
                let weak = weak.clone();
                glib::idle_add_local_once(move || {
                    if let Some(v) = weak.upgrade()
                        && x11::supported(&v.window)
                    {
                        let _ = x11::place(&v.window, v.above.get(), true);
                    }
                });
            });
        }
        {
            let weak = Rc::downgrade(&view);
            gtk::prelude::WidgetExt::display(&view.window)
                .monitors()
                .connect_items_changed(move |_, _, _, _| {
                    if let Some(v) = weak.upgrade()
                        && x11::supported(&v.window)
                    {
                        let _ = x11::place(&v.window, v.above.get(), true);
                    }
                });
        }
        {
            let v = Rc::downgrade(&view);
            let h = Rc::downgrade(hub);
            watch_visibility(&view.window, move || {
                if let (Some(v), Some(h)) = (v.upgrade(), h.upgrade()) {
                    v.update_art(&h.state.borrow());
                }
            });
        }
        view
    }
    fn update(&self, s: &State) {
        let p = s.player.as_ref();
        self.title.set_label(
            p.filter(|p| !p.track.title.is_empty())
                .map(|p| p.track.title.as_str())
                .unwrap_or("Нет текущей записи"),
        );
        self.subtitle.set_label(
            &p.map(|p| {
                let status = friendly_status(&p.status);
                if p.track.artist.is_empty() {
                    status.into()
                } else {
                    format!("{} · {status}", p.track.artist)
                }
            })
            .unwrap_or_else(|| "Плеер отключён".into()),
        );
        self.update_art(s);
        let (label, play, next) = playback(p);
        self.play.set_label(label);
        self.play.set_sensitive(play);
        self.next.set_sensitive(next);
        self.window.set_css_classes(if s.light {
            &["overtone", "light"]
        } else {
            &["overtone"]
        });
        if self.above.replace(s.above) != s.above && x11::supported(&self.window) {
            let _ = x11::place(&self.window, s.above, false);
        }
        if !s.enabled("mini") {
            let w = self.window.downgrade();
            glib::idle_add_local_once(move || {
                if let Some(w) = w.upgrade() {
                    w.close();
                }
            });
        }
    }
    fn update_art(&self, state: &State) {
        let uri = if drawable(&self.window) {
            state
                .player
                .as_ref()
                .map(|p| p.track.art.as_str())
                .unwrap_or("")
        } else {
            ""
        };
        self.art.update(uri);
    }
}

fn main() -> glib::ExitCode {
    // This UI has no continuous drawing. Cairo avoids a separate GPU renderer's
    // buffers. Honor GSK_RENDERER when the user specifies it.
    if std::env::var_os("GSK_RENDERER").is_none() {
        // Safe here: the application has not started any threads.
        unsafe { std::env::set_var("GSK_RENDERER", "cairo") };
    }
    let id = project().id;
    let app = gtk::Application::builder()
        .application_id(&id)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    // GtkApplication registers on the user bus. Further launches open the existing window.
    let hub = Hub::new(&app);
    app.connect_startup(|_| {
        if let Some(display) = gtk::gdk::Display::default() {
            let css = gtk::CssProvider::new();
            css.load_from_data(include_str!("style.css"));
            gtk::style_context_add_provider_for_display(
                &display,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
    app.connect_command_line(move |app, args| {
        let args = args.arguments();
        let flag = args.get(1).and_then(|s| s.to_str());
        match flag {
            Some("--launcher") => launcher::present(app),
            Some("--mini") => hub.open_mini(),
            Some("--quit") => {
                hub.save_mini();
                if let Err(e) = call(&Request::Quit) {
                    eprintln!("{e}");
                }
                if let Some(app) = hub.app.upgrade() {
                    app.quit();
                }
            }
            Some("--version") => println!("{} {}", project().name, env!("CARGO_PKG_VERSION")),
            Some("--page") => hub.open_manager(args.get(2).and_then(|s| s.to_str())),
            _ => hub.open_manager(None),
        }
        glib::ExitCode::SUCCESS
    });
    app.run()
}
