// SPDX-License-Identifier: GPL-3.0-only
use serde_json::{json, Map, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{OnceLock, RwLock},
};

pub const SOURCES: [&str; 4] = ["yandex", "external", "fallback", "prefer"];

#[derive(Clone)]
pub struct Options {
    pub discord: Option<bool>,
    pub lyrics: usize,
    pub fckcensor: bool,
    pub time: bool,
    pub spicy: bool,
    pub fonts: bool,
    pub font: String,
}

pub fn directory() -> PathBuf {
    crate::sys::config_base().join("overtone")
}

pub fn read(file: &str) -> Value {
    fs::read(directory().join(file))
        .ok()
        .and_then(|contents| serde_json::from_slice(&contents).ok())
        .unwrap_or(Value::Null)
}

fn load() -> Options {
    let mod_config = read("yandex-mod.json");
    let plugins = read("plugins.json");
    Options {
        discord: mod_config["enabled"].as_bool(),
        lyrics: SOURCES
            .iter()
            .position(|mode| Some(*mode) == mod_config["lyricsMode"].as_str())
            .unwrap_or(0),
        fckcensor: plugins["fckcensor"].as_bool().unwrap_or(false),
        time: plugins["time"].as_bool().unwrap_or(true),
        spicy: plugins["spicy"].as_bool().unwrap_or(false),
        fonts: plugins["fonts"].as_bool().unwrap_or(false),
        font: plugins["font"]
            .as_str()
            .unwrap_or("Sans")
            .chars()
            .take(100)
            .collect(),
    }
}

fn state() -> &'static RwLock<Options> {
    static OPTIONS: OnceLock<RwLock<Options>> = OnceLock::new();
    OPTIONS.get_or_init(|| RwLock::new(load()))
}

pub fn current() -> Options {
    state().read().unwrap().clone()
}

pub fn fckcensor() -> bool {
    state().read().unwrap().fckcensor
}

pub fn time_enabled() -> bool {
    state().read().unwrap().time
}

pub fn write(path: &Path, value: &Value) -> io::Result<()> {
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        serde_json::to_writer(&mut file, value)?;
        file.write_all(b"\n")?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn change(key: &str, value: &str) -> io::Result<()> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Некорректное значение настройки",
        )
    };
    let boolean = || {
        value
            .parse::<bool>()
            .map(Value::Bool)
            .map_err(|_| invalid())
    };
    let (file, key, value) = match key {
        "discord" => ("yandex-mod.json", "enabled", boolean()?),
        "lyrics" => {
            let index = value.parse::<usize>().map_err(|_| invalid())?;
            (
                "yandex-mod.json",
                "lyricsMode",
                json!(SOURCES.get(index).ok_or_else(invalid)?),
            )
        }
        "fckcensor" | "time" | "spicy" | "fonts" => ("plugins.json", key, boolean()?),
        "font" => {
            let font = value.trim();
            if font.is_empty() || font.chars().count() > 100 || font.chars().any(char::is_control) {
                return Err(invalid());
            }
            ("plugins.json", key, json!(font))
        }
        _ => return Err(invalid()),
    };

    let dir = directory();
    fs::create_dir_all(&dir)?;
    let lock: File = OpenOptions::new()
        .append(true)
        .create(true)
        .open(dir.join("plugins.lock"))?;
    lock.lock()?;
    let mut data = read(file).as_object().cloned().unwrap_or_else(Map::new);
    data.insert(key.to_owned(), value);
    write(&dir.join(file), &Value::Object(data))?;
    let settings = load();
    *state().write().unwrap() = settings;
    Ok(())
}
