use anyhow::{Context, Result, bail};
use overtone_core::ADAPTER_VERSION;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};

const IPC_LIMIT: usize = 256 * 1024;
pub const SCRIPT: &[u8] = include_bytes!("../../../integration/overtone.lua");

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct Load {
    pub serial: u64,
    pub original: String,
    pub target: Option<String>,
    pub origin: Option<String>,
    pub state: String,
    pub time: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adapter {
    pub version: String,
    pub supported: bool,
    pub load: Load,
}
pub fn socket(runtime: &Path, pid: u32) -> PathBuf {
    runtime.join(format!("mpv-{pid}.sock"))
}
pub fn map(runtime: &Path, pid: u32) -> PathBuf {
    runtime.join(format!("mpv-{pid}.json"))
}
pub fn read_line<R: BufRead>(reader: &mut R, limit: usize) -> Result<Option<Value>> {
    let mut bytes = Vec::new();
    let count = std::io::Read::take(reader, limit as u64 + 1).read_until(b'\n', &mut bytes)?;
    if count == 0 {
        return Ok(None);
    }
    if count > limit || !bytes.ends_with(b"\n") {
        bail!("Некорректный размер сообщения");
    }
    Ok(Some(serde_json::from_slice(&bytes)?))
}
fn connect(path: &Path, pid: u32) -> Result<UnixStream> {
    let stream = UnixStream::connect(path)?;
    // The socket name and the kernel's peer PID must both match the MPRIS owner.
    use std::os::fd::AsRawFd;
    let mut creds: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of_val(&creds) as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut creds as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if result != 0 || creds.pid as u32 != pid || creds.uid != unsafe { libc::geteuid() } {
        bail!("Сокет принадлежит другому процессу");
    }
    stream.set_read_timeout(Some(Duration::from_millis(800)))?;
    stream.set_write_timeout(Some(Duration::from_millis(800)))?;
    Ok(stream)
}
pub fn send(path: &Path, pid: u32, args: Value) -> Result<()> {
    let mut stream = connect(path, pid)?;
    writeln!(stream, "{}", json!({"command":args}))?;
    Ok(())
}
pub fn command(path: &Path, pid: u32, args: Value) -> Result<Value> {
    let mut stream = connect(path, pid)?;
    writeln!(stream, "{}", json!({"command":args,"request_id":1}))?;
    let mut reader = BufReader::new(stream);
    while let Some(reply) = read_line(&mut reader, IPC_LIMIT)? {
        if reply["request_id"] != 1 {
            continue;
        }
        if reply["error"] == "success" {
            return Ok(reply["data"].clone());
        }
        bail!("mpv · {}", reply["error"].as_str().unwrap_or("ошибка IPC"));
    }
    bail!("mpv не вернул ответ")
}
pub fn detect(runtime: &Path, pid: u32) -> Option<Adapter> {
    let path = socket(runtime, pid);
    let protocol = command(
        &path,
        pid,
        json!(["get_property", "user-data/overtone-protocol"]),
    )
    .ok()?;
    let version = command(&path, pid, json!(["get_property", "mpv-version"]))
        .ok()?
        .as_str()?
        .to_owned();
    Some(Adapter {
        supported: protocol.as_u64() == Some(ADAPTER_VERSION) && tested(&version),
        version,
        load: Load::default(),
    })
}
pub fn tested(version: &str) -> bool {
    matches!(
        version.split_whitespace().nth(1),
        Some("0.41.0" | "v0.41.0")
    )
}

pub fn subscribe(
    runtime: &Path,
    pid: u32,
    on_change: impl Fn(Load) + Send + 'static,
) -> Result<UnixStream> {
    let mut stream = connect(&socket(runtime, pid), pid)?;
    writeln!(
        stream,
        "{}",
        json!({"command":["observe_property",1,"user-data/overtone"]})
    )?;
    stream.set_read_timeout(None)?;
    let thread = stream.try_clone()?;
    std::thread::Builder::new()
        .name("mpv-events".into())
        .spawn(move || {
            let mut reader = BufReader::new(thread);
            while let Ok(Some(value)) = read_line(&mut reader, IPC_LIMIT) {
                if value["event"] == "property-change"
                    && let Ok(load) = serde_json::from_value(value["data"].clone())
                {
                    on_change(load);
                }
            }
        })
        .context("Подписка mpv")?;
    Ok(stream)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_version_disables_hook_only() {
        assert!(tested("mpv 0.41.0"));
        assert!(!tested("mpv 0.42.0"));
        assert!(!tested("mpv 0.41.0-dirty"));
    }
}
