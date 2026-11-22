use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use zbus::blocking::{Connection, Proxy, fdo::DBusProxy};
use zbus::names::BusName;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

pub const PREFIX: &str = "org.mpris.MediaPlayer2.";
pub const PATH: &str = "/org/mpris/MediaPlayer2";
pub const BASE: &str = "org.mpris.MediaPlayer2";
pub const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub object: String,
    pub uri: String,
    pub title: String,
    pub artist: String,
    pub art: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Player {
    pub name: String,
    pub owner: String,
    pub pid: u32,
    pub identity: String,
    pub status: String,
    pub control: bool,
    pub play: bool,
    pub pause: bool,
    pub next: bool,
    pub schemes: Vec<String>,
    pub mimes: Vec<String>,
    pub has_tracklist: bool,
    pub track: Track,
}

pub struct Bus(pub Connection);
impl Bus {
    pub fn connect() -> Result<Self> {
        Ok(Self(
            zbus::blocking::connection::Builder::session()?
                .method_timeout(std::time::Duration::from_secs(2))
                .build()?,
        ))
    }
    pub fn names(&self) -> Result<Vec<String>> {
        let dbus = DBusProxy::new(&self.0)?;
        let mut names: Vec<_> = dbus
            .list_names()?
            .into_iter()
            .map(|n| n.to_string())
            .filter(|n| n.starts_with(PREFIX) && n != "org.mpris.MediaPlayer2.playerctld")
            .collect();
        names.sort();
        Ok(names)
    }
    pub fn inspect(&self, name: &str) -> Result<Player> {
        let dbus = DBusProxy::new(&self.0)?;
        let owner = dbus.get_name_owner(BusName::try_from(name)?)?;
        let pid = dbus.get_connection_unix_process_id(BusName::try_from(owner.as_str())?)?;
        let root = Proxy::new(&self.0, owner.as_str(), PATH, BASE)?;
        let player = Proxy::new(&self.0, owner.as_str(), PATH, PLAYER)?;
        let properties = Proxy::new(
            &self.0,
            owner.as_str(),
            PATH,
            "org.freedesktop.DBus.Properties",
        )?;
        let metadata: HashMap<String, OwnedValue> = properties
            .call::<_, _, OwnedValue>("Get", &(PLAYER, "Metadata"))?
            .try_into()?;
        let string = |key| {
            metadata
                .get(key)
                .and_then(|v| <&str>::try_from(v).ok())
                .unwrap_or_default()
                .to_owned()
        };
        let object = metadata
            .get("mpris:trackid")
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| OwnedObjectPath::try_from(v).ok())
            .map(|v| v.to_string())
            .unwrap_or_default();
        let artist = metadata
            .get("xesam:artist")
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| Vec::<String>::try_from(v).ok())
            .unwrap_or_default()
            .join(", ");
        Ok(Player {
            name: name.into(),
            owner: owner.to_string(),
            pid,
            identity: root
                .get_property("Identity")
                .unwrap_or_else(|_| name.into()),
            status: player.get_property("PlaybackStatus")?,
            control: player.get_property("CanControl").unwrap_or(false),
            play: player.get_property("CanPlay").unwrap_or(false),
            pause: player.get_property("CanPause").unwrap_or(false),
            next: player.get_property("CanGoNext").unwrap_or(false),
            schemes: root.get_property("SupportedUriSchemes").unwrap_or_default(),
            mimes: root.get_property("SupportedMimeTypes").unwrap_or_default(),
            has_tracklist: root.get_property("HasTrackList").unwrap_or_default(),
            track: Track {
                object,
                title: string("xesam:title"),
                uri: string("xesam:url"),
                artist,
                art: string("mpris:artUrl"),
            },
        })
    }
    pub fn action(&self, player: &Player, action: &str, uri: Option<&str>) -> Result<()> {
        let proxy = Proxy::new(&self.0, player.owner.as_str(), PATH, PLAYER)?;
        if !player.control {
            bail!("Плеер не разрешил управление");
        }
        match action {
            "PlayPause"
                if if player.status == "Playing" {
                    player.pause
                } else {
                    player.play
                } =>
            {
                proxy.call::<_, _, ()>(action, &())?
            }
            "Pause" if player.pause => proxy.call::<_, _, ()>(action, &())?,
            "Next" if player.next => proxy.call::<_, _, ()>(action, &())?,
            "OpenUri" => {
                let uri = uri.ok_or_else(|| anyhow::anyhow!("Укажите URI записи"))?;
                let scheme = uri
                    .split_once(':')
                    .map(|(scheme, _)| scheme)
                    .unwrap_or_default();
                if !player.schemes.iter().any(|s| s == scheme) {
                    bail!("Плеер не объявил поддержку схемы {scheme}");
                }
                proxy.call::<_, _, ()>(action, &(uri,))?
            }
            _ => bail!("Плеер не объявил доступность операции"),
        }
        Ok(())
    }
}

pub mod mpv;

#[derive(Debug)]
pub enum Signal {
    Names,
    Property(String),
    Seek(String, i64),
}
pub fn listen(bus: &Bus, on_signal: impl Fn(Signal) + Clone + Send + 'static) -> Result<()> {
    use zbus::blocking::MessageIterator;
    for rule in [
        "type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',member='NameOwnerChanged',arg0namespace='org.mpris.MediaPlayer2'",
        "type='signal',path='/org/mpris/MediaPlayer2',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged'",
        "type='signal',path='/org/mpris/MediaPlayer2',interface='org.mpris.MediaPlayer2.Player',member='Seeked'",
    ] {
        let events = MessageIterator::for_match_rule(rule, &bus.0, Some(128))?;
        let send = on_signal.clone();
        std::thread::spawn(move || {
            for msg in events.flatten() {
                let header = msg.header();
                let Some(sender) = header.sender() else {
                    continue;
                };
                match header.member().map(|s| s.as_str()) {
                    Some("PropertiesChanged") => send(Signal::Property(sender.to_string())),
                    Some("Seeked") if let Ok((pos,)) = msg.body().deserialize::<(i64,)>() => {
                        send(Signal::Seek(sender.to_string(), pos))
                    }
                    Some("NameOwnerChanged") => send(Signal::Names),
                    _ => {}
                }
            }
        });
    }
    Ok(())
}
