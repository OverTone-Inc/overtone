#!/usr/bin/env python3
"""Apply the Overtone changes to the pinned GPL-3.0-only yamusic-rs source."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[2]
UPSTREAM = "https://github.com/SpolzerMod/yamusic-rs.git"
REVISION = "a145a4f9cf26b5287ceb23e95a316ec9dc52338c"
PREPARED_FILES = {
    "overtone.rs": "src/overtone.rs",
    "options.rs": "src/overtone_options.rs",
    "fckcensor.rs": "src/overtone_fckcensor.rs",
    "discord_cover.rs": "src/overtone_discord_cover.rs",
    "overtone.slint": "ui/overtone.slint",
    "../yandex/licenses/FckCensor-MIT.txt": "FckCensor-MIT.txt",
}


def prepare(target):
    if not target.exists():
        subprocess.run(["git", "clone", "--branch", "v1.1.4", "--depth", "1", UPSTREAM, target], check=True)
    revision = subprocess.check_output(["git", "-C", target, "rev-parse", "HEAD"], text=True).strip()
    if revision != REVISION:
        raise ValueError("Неожиданная ревизия yamusic-rs.")

    def edit(name, *changes):
        file = target / name
        baseline = subprocess.check_output(["git", "-C", target, "show", f"HEAD:{name}"], text=True)
        for before, after in changes:
            if before not in baseline:
                raise ValueError(f"Строка для патча не найдена в {name}: {before}")
            baseline = baseline.replace(before, after)
        file.write_text(baseline)

    runtime = ROOT / "integration/rust"
    app_id = json.loads((ROOT / "integration/yandex/discord-application.json").read_text())["applicationId"]
    edit("Cargo.toml", ("panic = \"abort\"", "panic = \"abort\"\n\n[workspace]"))
    edit("build.rs", ('"Яндекс Музыка RS"', '"Яндекс Музыка (Rust)"'))
    edit("src/main.rs", ("mod net;", "mod net;\nmod overtone;\nmod overtone_fckcensor;"),
         ('set_xdg_app_id("yamusic-rs")', 'set_xdg_app_id("overtone-yamusic-rust")'))
    for path in ("src/sys/linux.rs", "src/mpris.rs", "src/secret.rs"):
        edit(path, ('"yamusic-rs"', '"overtone-yamusic-rust"'))
    baseline = subprocess.check_output(
        ["git", "-C", target, "show", "HEAD:src/config.rs"], text=True)
    first = baseline.index("        let base = crate::sys::config_base();")
    last = baseline.index("\n    })", first)
    edit("src/config.rs", (baseline[first:last], '        crate::sys::config_base().join("overtone-yamusic-rust")'))
    discord = subprocess.check_output(["git", "-C", target, "show", "HEAD:src/discord.rs"], text=True)
    old_run = discord[discord.index("    fn run(&self, player: Player)"):discord.index("\n}\n\n#[cfg(test)]")]
    edit("src/discord.rs", ('"1570815647133474856"', f'"{app_id}"'),
         ('env!("CARGO_PKG_REPOSITORY")', '"https://github.com/OverTone-Inc/overtone"'),
         ("use crate::api::Track;", '#[path = "overtone_discord_cover.rs"]\nmod covers;\n\nuse crate::api::Track;'),
         ("            let mut c = Conn { pipe, nonce: 0 };", """            #[cfg(unix)]
            {
                let deadline = Some(Duration::from_secs(8));
                pipe.set_read_timeout(deadline).map_err(|e| e.to_string())?;
                pipe.set_write_timeout(deadline).map_err(|e| e.to_string())?;
            }
            let mut c = Conn { pipe, nonce: 0 };"""),
         ("""    /// Ok(None) — принято, Ok(Some(msg)) — Discord вернул ошибку на эту активность.
    fn set_activity(&mut self, activity: Value) -> Result<Option<String>, String> {""",
          "    fn set_activity(&mut self, activity: Value) -> Result<Value, String> {"),
         ("""                    if v.get("evt").and_then(|e| e.as_str()) == Some("ERROR") {
                        let msg = v.pointer("/data/message").and_then(|m| m.as_str()).unwrap_or("?");
                        return Ok(Some(msg.to_string()));
                    }
                    return Ok(None);""", "                    return Ok(v);"),
         ("fn activity(t: &Track, start: i64, end: Option<i64>, with_display_type: bool)",
          "fn activity(t: &Track, image: &str, start: i64, end: Option<i64>, with_display_type: bool)"),
         ('"timestamps": { "start": start }', '"timestamps": { "start": start / 1000 }'),
         ('a["timestamps"]["end"] = json!(end);', 'a["timestamps"]["end"] = json!((end + 999) / 1000);'),
         ('"large_image": t.cover_url(400).unwrap_or_else(|| "logo".into()),', '"large_image": image,'),
         (old_run, (runtime / "discord_run.inc.rs").read_text().rstrip()))
    edit("src/api.rs", ("fn parse_lrc(", "pub(crate) fn parse_lrc("),
         ("    pub duration_ms: u64,\n    pub cover: Option<String>,\n    pub explicit: bool,",
          "    pub duration_ms: u64,\n    pub cover: Option<String>,\n    pub album_cover: Option<String>,\n    pub explicit: bool,"),
         ('        cover: v.get("coverUri").and_then(|c| c.as_str()).map(String::from),\n        explicit:',
          '''        cover: v.get("coverUri").and_then(Value::as_str).map(str::to_owned),
        album_cover: album.and_then(|a| a.get("coverUri").or_else(|| a.pointer("/cover/uri")))
            .and_then(Value::as_str).map(str::to_owned),
        explicit:'''))
    edit("src/net.rs", ("api.lyrics(arg).map(Data::Lyrics)", "crate::overtone::lyrics(api, arg).map(Data::Lyrics)"))
    edit("src/stream.rs",
         ("impl Buf {", """impl Buf {
    pub fn complete(data: Vec<u8>) -> Arc<Self> {
        let len = Some(data.len() as u64);
        Arc::new(Self { state: Mutex::new(State { data, len, done: true, err: None }), cv: Condvar::new() })
    }
"""))
    edit("src/player.rs",
         ("fn load_track(api: &Api, t: &Track) -> Res<Audio> {\n    let url = api.stream_url(&t.id)?;",
          """fn load_track(api: &Api, t: &Track, has_plus: bool) -> Res<Audio> {
    let url = api.stream_url(&t.id)?;
    if has_plus && crate::overtone::options::fckcensor() {
        if let Some(data) = crate::overtone_fckcensor::replacement(&t.id) {
            let buf = Buf::complete(data);
            if let Ok(decoder) = Decoder::new(buf.reader()) {
                return Ok((buf, decoder));
            }
        }
    }"""),
         ("let res = load_track(&api, &t);", "let res = load_track(&api, &t, has_plus);"),
         ("let res = load_track(&api, &next);", "let res = load_track(&api, &next, has_plus);"),
         ("        let tx = self.tx.clone();\n        thread::spawn(move || {\n            let res = load_track",
          "        let tx = self.tx.clone();\n        let has_plus = self.sh().has_plus;\n        thread::spawn(move || {\n            let res = load_track"))
    edit("src/gui.rs",
         ("    start_updates(&ui);", "    // This fork is updated by Overtone."),
         ("let playing_now = started && s.playing;", """let playing_now = started && s.playing;
        if crate::overtone::observe(playing_now && !s.loading) && app.get_overtone_open() {
            app.set_overtone_time_summary(crate::overtone::listening_summary().into());
        }"""),
         ("    let ui = AppWindow::new()?;", "    let ui = AppWindow::new()?;\n    ui.set_overtone_font(crate::overtone::family().into());"),
         ("app.set_has_lyrics(t.lyrics);", "app.set_has_lyrics(t.lyrics || crate::overtone::lyrics_available());"),
         ("if t.lyrics && current {", "if (t.lyrics || crate::overtone::lyrics_available()) && current {"),
         ("let discord = crate::discord::Discord::spawn(",
          "let discord_enabled = crate::overtone::presence_enabled(cfg.discord_enabled);\n    let discord = crate::discord::Discord::spawn("),
         ("enabled: cfg.discord_enabled, app_id: cfg.discord_app_id.clone()", "enabled: discord_enabled, app_id: String::new()"),
         ("app.set_discord_on(cfg.discord_enabled);", """app.set_discord_on(discord_enabled);
    let overtone = crate::overtone::current();
    app.set_overtone_spicy(overtone.spicy);
    app.set_overtone_fonts(overtone.fonts);
    app.set_overtone_display_font(ui.get_overtone_font());"""),
         ('let key = format!("lyrics:{}", t.id);', 'let key = crate::overtone::lyrics_key(&t.id);'),
         ("fn quit() {", "fn quit() {\n    crate::overtone::observe(false);"),
         ("fn notifier() -> Arc<dyn Fn() + Send + Sync> {", """thread_local! {
    static RESTORE_REPAINT: Timer = Timer::default();
}

fn repaint_after_restore() {
    // Skia can reuse a partial frame after the compositor replaces the window surface.
    // Let the native show and focus events settle before invalidating the whole window.
    RESTORE_REPAINT.with(|timer| {
        timer.start(TimerMode::SingleShot, Duration::from_millis(16), || {
            with_ctx(|c| full_repaint(&c.ui()));
        });
    });
}

fn notifier() -> Arc<dyn Fn() + Send + Sync> {"""),
         ("    sys::raise(ui.window());\n}",
          "    sys::raise(ui.window());\n    if !first {\n        repaint_after_restore();\n    }\n}"),
         ("            WindowEvent::Moved(_) => {",
          """            WindowEvent::Focused(true) | WindowEvent::Occluded(false) => {
                repaint_after_restore();
                EventResult::Propagate
            }
            WindowEvent::Moved(_) => {"""),
         ("    fn setting(self: &Rc<Self>, id: &str) {",
          (runtime / "gui.inc.rs").read_text() + "\n    fn setting(self: &Rc<Self>, id: &str) {"),
         ('            "about" => sys::open_url(env!("CARGO_PKG_REPOSITORY")),',
          """            "about" => sys::open_url(env!("CARGO_PKG_REPOSITORY")),
            "overtone-open" => {
                if !app.get_overtone_open() {
                    self.sync_overtone();
                    app.set_overtone_feedback(SharedString::new());
                }
                app.set_overtone_open(!app.get_overtone_open());
            }"""),
         ("""                app.set_discord_on(on);
                self.discord.cfg.lock().unwrap().enabled = on;
                let mut st = self.st.borrow_mut();
                st.cfg.discord_enabled = on;
                st.cfg.save();""", '                self.overtone_change("discord", if on { "true" } else { "false" });'),
         ('if app.get_page() == Page::Settings {', 'if app.get_page() == Page::Settings || app.get_overtone_open() {'),
         ("    app.on_setting(|id| with_ctx(|c| c.setting(&id)));",
          '    app.on_setting(|id| with_ctx(|c| c.setting(&id)));\n'
          '    app.on_overtone_change(|key, value| with_ctx(|c| c.overtone_change(&key, &value)));'))
    edit("ui/state.slint", ("    callback setting(string);", """    callback setting(string);
    in-out property <bool> overtone-open: false;
    in property <int> overtone-source: 0;
    in property <bool> overtone-fck: false;
    in property <bool> overtone-time: true;
    in property <bool> overtone-spicy: false;
    in property <bool> overtone-fonts: false;
    in property <string> overtone-family: "Sans";
    in property <string> overtone-display-font: "YS Text";
    in property <string> overtone-time-summary;
    in property <string> overtone-feedback;
    callback overtone-change(string, string);"""))
    edit("ui/fullscreen.slint",
         ("    in property <bool> clickable;\n    in property <float> fade: 1;",
          "    in property <bool> clickable;\n    in property <float> progress;\n    in property <float> fade: 1;"),
         ("""        font-family: "YS Music Headline";
        font-size: root.font-size;
        font-weight: 700;
        color: #e6e6e6.with-alpha(root.alpha * root.fade);""",
          """        font-family: App.overtone-fonts ? App.overtone-display-font : "YS Music Headline";
        font-size: root.font-size;
        font-weight: 700;
        color: (App.overtone-spicy && root.active ? #ffd7b2 : #e6e6e6).with-alpha(root.alpha * root.fade);"""),
         ("""    TouchArea {
        enabled: root.clickable;
        mouse-cursor: root.clickable ? MouseCursor.pointer""",
          """    if App.overtone-spicy && root.active: Rectangle {
        x: parent.width * 0.1;
        y: parent.height + 7px;
        width: parent.width * 0.8;
        height: 3px;
        border-radius: 2px;
        background: #ffffff28;
        Rectangle {
            x: 0;
            width: parent.width * root.progress;
            border-radius: 2px;
            background: #ffd7b2;
            animate width { duration: App.motion ? 80ms : 0ms; }
        }
    }
    TouchArea {
        enabled: root.clickable;
        mouse-cursor: root.clickable ? MouseCursor.pointer"""),
         ("            clickable: root.scrolled;",
          """            clickable: l.time >= 0 && (App.overtone-spicy || root.scrolled);
            progress: i != root.cur || l.time < 0 ? 0 : clamp(
                (App.position - l.time) / max(0.1, (i + 1 < App.lyrics.length ? App.lyrics[i + 1].time : App.duration) - l.time), 0, 1);"""),
         ("    background: App.tint;\n    property <bool> side:", """    background: App.overtone-spicy && App.fs-mode == 2 ?
        @linear-gradient(125deg, #1b1118, App.tint, #2d1923) : App.tint;
    property <bool> side:"""),
         ('    // Текст песни справа от обложки, как SyncLyrics оригинала.',
          """    if App.overtone-spicy && App.fs-mode == 2: Text {
        x: 40px;
        y: 52px;
        text: "SP Lyrics";
        color: #ffd7b2;
        font-size: 16px;
        font-weight: 700;
    }
    // Текст песни справа от обложки, как SyncLyrics оригинала."""))
    edit("ui/app.slint",
         ('import { FullscreenPlayer } from "fullscreen.slint";',
          'import { FullscreenPlayer } from "fullscreen.slint";\nimport { OvertonePanel } from "overtone.slint";'),
         ("    HorizontalLayout {\n        x: parent.width - 4 * 46px;", """    if !App.mini: Rectangle {
        x: parent.width - 4 * 46px - self.width - 10px;
        y: 5px;
        width: 86px;
        height: 24px;
        border-radius: 12px;
        background: ot-button.has-hover || App.overtone-open ?
            Theme.controls-color-secondary-default-hovered : Theme.controls-color-secondary-default-enabled;
        Text {
            text: "Overtone";
            font-size: 12px;
            font-weight: 650;
            color: Theme.controls-color-primary-text-enabled;
            vertical-alignment: center;
            horizontal-alignment: center;
        }
        accessible-role: button;
        accessible-label: "Настройки Overtone";
        accessible-action-default => { App.setting("overtone-open"); }
        ot-button := TouchArea {
            mouse-cursor: MouseCursor.pointer;
            clicked => { App.setting("overtone-open"); }
        }
    }

    HorizontalLayout {
        x: parent.width - 4 * 46px;"""),
         ('    default-font-family: "YS Text";', '    in property <string> overtone-font: "YS Text";\n    default-font-family: overtone-font;'),
         ("    if App.mini: MiniPlayer {",
          """    if !App.mini: Rectangle {
        x: parent.width - self.width;
        y: parent.height - self.height;
        width: overtone-label.preferred-width + 16px;
        height: 22px;
        border-top-left-radius: 8px;
        background: Theme.background-color-primary-enabled-content;
        overtone-label := Text {
            text: "Overtone · Rust";
            font-size: 11px;
            color: Theme.controls-color-secondary-on-outline-enabled;
        }
    }

    if App.overtone-open && !App.mini: OvertonePanel {
        x: 0;
        y: 32px;
        width: parent.width;
        height: parent.height - 32px;
    }

    if App.mini: MiniPlayer {"""))
    for source, dest in PREPARED_FILES.items():
        shutil.copy2(runtime / source, target / dest)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("checkout", type=Path)
    prepare(parser.parse_args().checkout.resolve())
