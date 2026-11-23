// SPDX-License-Identifier: GPL-3.0-only
use reqwest::Url;
use serde_json::Value;

pub(super) fn candidates(covers: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<String> {
    let mut result = Vec::new();
    for cover in covers {
        let input = cover.as_ref().trim();
        if input.is_empty() || input.len() > 1024 {
            continue;
        }
        let input = input.replace("%25%25", "400x400").replace("%%", "400x400");
        let address = input
            .strip_prefix("https://")
            .or_else(|| input.strip_prefix("http://"))
            .unwrap_or(&input)
            .trim_start_matches("//");
        let Ok(mut url) = Url::parse(&format!("https://{address}")) else {
            continue;
        };
        let host = url.host_str().unwrap_or_default();
        let allowed = [
            "yandex.net",
            "yandex.ru",
            "yandex.com",
            "yandex.by",
            "yandex.kz",
            "yandex.uz",
            "yastatic.net",
        ];
        if !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.path() == "/"
            || !allowed.iter().any(|domain| {
                host == *domain
                    || host
                        .strip_suffix(*domain)
                        .is_some_and(|sub| sub.ends_with('.'))
            })
        {
            continue;
        }
        let alternate = matches!(host, "avatars.yandex.net" | "avatars.mds.yandex.net")
            && (url.path().starts_with("/get-music-") || url.path().starts_with("/get-music/"));
        url.set_fragment(None);
        if alternate {
            for cdn in ["avatars.mds.yandex.net", "avatars.yandex.net"] {
                if url.set_host(Some(cdn)).is_ok() {
                    push_unique(&mut result, url.as_str());
                }
            }
        } else {
            push_unique(&mut result, url.as_str());
        }
    }
    result
}

fn push_unique(result: &mut Vec<String>, url: &str) {
    if url.len() <= 512 && !result.iter().any(|entry| entry == url) {
        result.push(url.to_owned());
    }
}

pub(super) fn resolved(response: &Value) -> Option<&str> {
    response
        .pointer("/data/assets/large_image")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

pub(super) fn rejected(
    response: &Value,
    requested: &str,
    previous: Option<&(String, String)>,
) -> bool {
    if requested == "logo"
        || !response
            .pointer("/data/details")
            .is_some_and(Value::is_string)
    {
        return false;
    }
    let Some(image) = resolved(response) else {
        return true;
    };
    image == "logo"
        || response
            .pointer("/data/assets/small_image")
            .and_then(Value::as_str)
            == Some(image)
        || previous.is_some_and(|(url, asset)| url != requested && asset == image)
}
