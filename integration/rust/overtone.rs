// SPDX-License-Identifier: GPL-3.0-only
//! Settings shared with the Overtone desktop application.
use crate::api::{Api, Lyrics, Track};
use serde_json::Value;
use std::{collections::BTreeMap, time::Duration};

#[path = "overtone_options.rs"]
pub mod options;

pub use options::{current, directory};

pub fn presence_enabled(default: bool) -> bool {
    current().discord.unwrap_or(default)
}

pub fn family() -> String {
    let plugins = current();
    if !plugins.fonts {
        return "YS Text".into();
    }
    registered_family().unwrap_or(plugins.font)
}

fn registered_family() -> Option<String> {
    use slint::fontique_011::{fontique, shared_collection};
    use std::{cell::RefCell, io::Read, time::SystemTime};

    thread_local! {
        static IMPORT: RefCell<Option<(SystemTime, Option<String>)>> = const { RefCell::new(None) };
    }
    let path = directory().join("custom-font");
    let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
    IMPORT.with_borrow_mut(|cached| {
        if let Some((_, family)) = cached.as_ref().filter(|(time, _)| *time == modified) {
            return family.clone();
        }
        let register = || {
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .ok()?
                .take(10 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .ok()?;
            if bytes.len() > 10 * 1024 * 1024 {
                return None;
            }
            let data = if bytes.starts_with(b"wOFF") {
                crate::assets::woff_to_sfnt(&bytes)?
            } else {
                bytes
            };
            let mut collection = shared_collection();
            let fonts =
                collection.register_fonts(fontique::Blob::new(std::sync::Arc::new(data)), None);
            collection.family_name(fonts.first()?.0).map(str::to_owned)
        };
        let result = register();
        *cached = Some((modified, result.clone()));
        result
    })
}

pub fn lyrics_available() -> bool {
    let settings = current();
    settings.lyrics != 0 || settings.spicy
}

pub fn lyrics_key(id: &str) -> String {
    let settings = current();
    // Different sources get different entries in the client's existing lyrics cache.
    format!(
        "lyrics:{}:{}:{id}",
        settings.lyrics,
        u8::from(settings.spicy)
    )
}

pub fn lyrics(api: &Api, request: &str) -> Result<Lyrics, String> {
    let (source, rest) = request.split_once(':').unwrap_or(("0", request));
    let (spicy, id) = rest.split_once(':').unwrap_or(("0", rest));
    let original = || api.lyrics(id);
    let external = || {
        let track = api.tracks(&[id.to_owned()]).ok()?.into_iter().next()?;
        lrclib(&track)
    };
    match (source, spicy) {
        ("1", _) => Ok(external().unwrap_or_default()),
        ("3", _) => external().map_or_else(original, Ok),
        ("2", _) | ("0", "1") => match original() {
            Ok(lyrics) if !lyrics.synced.is_empty() || !lyrics.text.is_empty() => Ok(lyrics),
            result => external().map_or(result, Ok),
        },
        _ => original(),
    }
}

fn lrclib(track: &Track) -> Option<Lyrics> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(6))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Overtone Rust (https://github.com/OverTone-Inc/overtone)")
        .build()
        .ok()?;
    let seconds = (track.duration_ms as f64 / 1000.0).round().to_string();
    let body = client
        .get("https://lrclib.net/api/get")
        .query(&[
            ("track_name", track.title.as_str()),
            ("artist_name", track.artists.as_str()),
            ("album_name", track.album_title.as_str()),
            ("duration", seconds.as_str()),
        ])
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .text()
        .ok()?;
    if body.len() > 512_000 {
        return None;
    }
    let payload: Value = serde_json::from_str(&body).ok()?;
    let synced = payload["syncedLyrics"]
        .as_str()
        .map(crate::api::parse_lrc)
        .unwrap_or_default();
    let text: Vec<String> = payload["plainLyrics"]
        .as_str()
        .unwrap_or("")
        .lines()
        .map(str::to_owned)
        .collect();
    (!synced.is_empty() || !text.is_empty()).then_some(Lyrics {
        synced,
        text,
        source: "LRCLIB".into(),
        writers: String::new(),
    })
}

#[derive(Default)]
struct Listening {
    start: Option<std::time::Instant>,
    days: BTreeMap<String, u64>,
    loaded: bool,
}
thread_local! {
    static LISTENING: std::cell::RefCell<Listening> = std::cell::RefCell::new(Listening::default());
}

pub fn observe(playing: bool) -> bool {
    let playing = playing && options::time_enabled();
    LISTENING.with_borrow_mut(|state| {
        let Some(start) = state.start else {
            if playing {
                if !state.loaded {
                    state.loaded = true;
                    state.days = serde_json::from_value(
                        options::read("rust-listening-time.json")["days"].clone(),
                    )
                    .unwrap_or_default();
                }
                state.start = Some(std::time::Instant::now());
            }
            return false;
        };
        let elapsed = start.elapsed();
        if playing && elapsed < Duration::from_secs(30) {
            return false;
        }
        state.start = playing.then(std::time::Instant::now);
        let seconds = elapsed.as_secs().min(35);
        if seconds == 0 {
            return false;
        }
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let day = state.days.entry(date).or_default();
        *day = day.saturating_add(seconds).min(86_400);
        let dir = directory();
        if std::fs::create_dir_all(&dir).is_ok() {
            let _ = options::write(
                &dir.join("rust-listening-time.json"),
                &serde_json::json!({ "days": state.days }),
            );
        }
        true
    })
}

pub fn listening_summary() -> String {
    use chrono::Datelike;
    let today = chrono::Local::now().date_naive();
    let day = today.format("%Y-%m-%d").to_string();
    let monday = (today - chrono::Duration::days(today.weekday().num_days_from_monday().into()))
        .format("%Y-%m-%d")
        .to_string();
    let (mut current, mut week, mut total) = (0_u64, 0_u64, 0_u64);
    for (file, scale) in [
        ("rust-listening-time.json", 1_u64),
        ("time-addon.json", 1),
        ("listening-time.json", 1000),
    ] {
        let config = options::read(file);
        let Some(days) = config["days"].as_object() else {
            continue;
        };
        for (date, value) in days {
            let Some(seconds) = value
                .as_f64()
                .filter(|value| (0.0..=86_400.0 * scale as f64).contains(value))
            else {
                continue;
            };
            let seconds = (seconds / scale as f64) as u64;
            total = total.saturating_add(seconds);
            if date == &day {
                current += seconds;
            }
            if date >= &monday && date <= &day {
                week = week.saturating_add(seconds);
            }
        }
    }
    fn format(seconds: u64) -> String {
        format!("{} ч {:02} мин", seconds / 3600, seconds % 3600 / 60)
    }
    format!(
        "Сегодня {}   ·   Неделя {}   ·   Всего {}",
        format(current),
        format(week),
        format(total)
    )
}
