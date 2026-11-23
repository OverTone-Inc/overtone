// SPDX-License-Identifier: GPL-3.0-only
// Catalog format adapted from Haz's MIT-licensed FckCensor, https://github.com/Hazzz895/FckCensor.

use serde_json::Value;
use std::{
    collections::HashMap,
    io::Read,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

const ROOT: &str = "https://raw.githubusercontent.com/Hazzz895/FckCensorData/";
const INDEX: &str = "https://raw.githubusercontent.com/Hazzz895/FckCensorData/main/list_v2.json";
const MAX_INDEX: usize = 1024 * 1024;
const MAX_AUDIO: usize = 64 * 1024 * 1024;

#[derive(Default)]
struct Cache {
    entries: HashMap<String, String>,
    next_refresh: Option<Instant>,
}

fn numeric(value: &str) -> bool {
    !value.is_empty() && value.len() <= 24 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn id(value: &Value) -> Option<String> {
    let value = value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|number| number.to_string()))?;
    numeric(&value).then_some(value)
}

fn media_url(value: &str) -> Option<String> {
    let url = if value.starts_with("https://") {
        value.to_owned()
    } else {
        format!("https://{value}")
    };
    let path = url.strip_prefix(ROOT)?;
    let track = path
        .strip_prefix("main/tracks/")
        .or_else(|| path.strip_prefix("refs/heads/main/tracks/"))?;
    numeric(track).then_some(url)
}

fn entries(body: &[u8]) -> Option<HashMap<String, String>> {
    let root: Value = serde_json::from_slice(body).ok()?;
    let mut result = HashMap::new();
    for source in root["sources"].as_array()?.iter().rev() {
        let Some(storages) = source["tracksStorages"].as_array() else {
            continue;
        };
        for storage in storages.iter().rev() {
            let template = storage["urlTemplate"].as_str().unwrap_or("");
            if let Some(items) = storage["trackIds"].as_array() {
                for item in items {
                    let key = if item.is_object() {
                        id(&item["id"])
                    } else {
                        id(item)
                    };
                    if let Some(key) = key {
                        if let Some(url) = media_url(&template.replace("%%", &key)) {
                            result.insert(key, url);
                        }
                    }
                }
            }
            if let Some(items) = storage["tracks"].as_object() {
                for (key, item) in items.iter().filter(|(key, _)| numeric(key)) {
                    let value = if item.is_object() { &item["url"] } else { item };
                    let url = match id(value) {
                        Some(reference) => media_url(&template.replace("%%", &reference)),
                        None => value.as_str().and_then(media_url),
                    };
                    if let Some(url) = url {
                        result.insert(key.clone(), url);
                    }
                }
            }
        }
    }
    (!result.is_empty() && result.len() <= 10_000).then_some(result)
}

fn client() -> Option<&'static reqwest::blocking::Client> {
    static CLIENT: OnceLock<Option<reqwest::blocking::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::blocking::Client::builder()
                .connect_timeout(Duration::from_secs(4))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("Overtone Rust / FckCensor")
                .build()
                .ok()
        })
        .as_ref()
}

fn read_response(url: &str, limit: usize, timeout: Duration) -> Option<Vec<u8>> {
    let response = client()?
        .get(url)
        .timeout(timeout)
        .send()
        .ok()?
        .error_for_status()
        .ok()?;
    let length = response.content_length();
    if length.is_some_and(|length| length > limit as u64) {
        return None;
    }
    let mut data = Vec::new();
    response
        .take(limit as u64 + 1)
        .read_to_end(&mut data)
        .ok()?;
    (data.len() <= limit && length.is_none_or(|length| data.len() as u64 == length)).then_some(data)
}

fn replacement_url(track: &str) -> Option<String> {
    static CATALOG: OnceLock<Mutex<Cache>> = OnceLock::new();
    let mut cache = CATALOG.get_or_init(Mutex::default).lock().ok()?;
    let now = Instant::now();
    if cache.next_refresh.is_none_or(|deadline| deadline <= now) {
        // Accept the same local cache written by the Electron integration.
        let path = crate::overtone::directory().join("fckcensor-catalog.json");
        let cached = std::fs::metadata(&path)
            .ok()
            .filter(|file| file.is_file() && file.len() <= MAX_INDEX as u64);
        let recent = cached
            .as_ref()
            .and_then(|file| file.modified().ok())
            .and_then(|time| time.elapsed().ok())
            .is_some_and(|elapsed| elapsed < Duration::from_secs(21_600));
        let local = cached
            .and_then(|_| std::fs::File::open(path).ok())
            .and_then(|file| {
                let mut bytes = Vec::new();
                file.take(MAX_INDEX as u64 + 1)
                    .read_to_end(&mut bytes)
                    .ok()?;
                if bytes.len() > MAX_INDEX {
                    return None;
                }
                entries(&bytes)
            });
        let remote = if recent && local.is_some() {
            None
        } else {
            read_response(INDEX, MAX_INDEX, Duration::from_secs(5))
                .and_then(|bytes| entries(&bytes))
        };
        let fresh = remote.is_some() || recent && local.is_some();
        if let Some(entries) = remote.or(local) {
            cache.entries = entries;
        }
        cache.next_refresh = Some(
            now + if fresh {
                Duration::from_secs(21_600)
            } else {
                Duration::from_secs(90)
            },
        );
    }
    cache.entries.get(track).cloned()
}

fn audio_header(bytes: &[u8]) -> bool {
    bytes.starts_with(b"ID3")
        || bytes.starts_with(b"fLaC")
        || bytes.get(..4).is_some_and(|header| {
            header[0] == 0xff && header[1] & 0xe0 == 0xe0 && header[1] & 0x06 != 0
        })
}

pub fn replacement(track: &str) -> Option<Vec<u8>> {
    let track = track.split(':').next()?;
    let url = replacement_url(track)?;
    let bytes = read_response(&url, MAX_AUDIO, Duration::from_secs(24))?;
    (bytes.len() > 4096 && audio_header(&bytes)).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_catalog_entries_and_numeric_aliases_only() {
        let fixture = br#"{"sources":[{"tracksStorages":[{"urlTemplate":"raw.githubusercontent.com/Hazzz895/FckCensorData/refs/heads/main/tracks/%%","trackIds":[42,{"id":"43"}],"tracks":{"44":"42","45":"https://elsewhere.invalid/audio"}}]}]}"#;
        let index = entries(fixture).unwrap();
        assert_eq!(index.len(), 3);
        assert_eq!(index["42"], index["44"]);
        assert!(index["43"].ends_with("/43"));
    }

    #[test]
    fn rejects_non_audio_payloads() {
        assert!(audio_header(b"ID3\x04"));
        assert!(audio_header(b"fLaC"));
        assert!(!audio_header(b"<!doctype html>"));
    }
}
