use serde::Deserialize;
use std::{
    ffi::CString,
    io::{self, BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, Shutdown, SocketAddr, SocketAddrV4, TcpListener, TcpStream, ToSocketAddrs},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::Path,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

const RULES: &str = include_str!("../../../../integration/vpn/music-domains.json");
const TIMEOUT: Duration = Duration::from_secs(2);
const FORBIDDEN: &[u8] = b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n";
const BAD_GATEWAY: &[u8] = b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n";

#[derive(Deserialize)]
struct Domains {
    suffixes: Vec<String>,
    hosts: Vec<String>,
}

impl Domains {
    fn matches(&self, host: &str) -> bool {
        self.hosts.iter().any(|name| host == name)
            || self.suffixes.iter().any(|name| {
                host == name
                    || host
                        .strip_suffix(name)
                        .is_some_and(|prefix| prefix.ends_with('.'))
            })
    }
}

#[derive(Deserialize)]
struct Route {
    dev: String,
    gateway: Option<String>,
    metric: Option<u32>,
}

fn physical_interface() -> Option<CString> {
    let output = Command::new("ip")
        .args(["-j", "-4", "route", "show", "default"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut routes: Vec<Route> = serde_json::from_slice(&output.stdout).ok()?;
    routes.sort_by_key(|route| route.metric.unwrap_or_default());
    let route = routes.into_iter().find(|route| {
        route.gateway.is_some()
            && !route.dev.contains('/')
            && Path::new("/sys/class/net")
                .join(&route.dev)
                .join("device")
                .exists()
    })?;
    CString::new(route.dev).ok()
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    a != 0
        && a < 224
        && !ip.is_private()
        && !ip.is_loopback()
        && !ip.is_link_local()
        && !(a == 100 && (64..128).contains(&b))
        && !(a == 198 && (18..20).contains(&b))
}

fn bind_connect(interface: &CString, address: SocketAddrV4) -> io::Result<TcpStream> {
    // Own the descriptor immediately so each failed syscall closes it.
    let raw = unsafe {
        libc::socket(
            libc::AF_INET,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let name = interface.as_bytes_with_nul();
    let code = unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            name.as_ptr().cast(),
            name.len() as libc::socklen_t,
        )
    };
    if code != 0 {
        return Err(io::Error::last_os_error());
    }
    let addr = libc::sockaddr_in {
        sin_family: libc::AF_INET as libc::sa_family_t,
        sin_port: address.port().to_be(),
        sin_addr: libc::in_addr {
            s_addr: u32::from_ne_bytes(address.ip().octets()),
        },
        sin_zero: [0; 8],
    };
    // SAFETY: addr is a populated IPv4 sockaddr and fd owns a nonblocking IPv4 socket.
    let code = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&addr as *const libc::sockaddr_in).cast(),
            std::mem::size_of_val(&addr) as libc::socklen_t,
        )
    };
    if code < 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(error);
        }
        let mut poll = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        if unsafe { libc::poll(&mut poll, 1, TIMEOUT.as_millis() as libc::c_int) } <= 0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Direct connection timed out",
            ));
        }
    }
    let stream = TcpStream::from(fd);
    if let Some(error) = stream.take_error()? {
        return Err(error);
    }
    stream.set_nonblocking(false)?;
    Ok(stream)
}

fn upstream(interface: &CString, host: &str) -> io::Result<TcpStream> {
    let addresses: Vec<_> = (host, 443).to_socket_addrs()?.take(8).collect();
    for address in &addresses {
        if let SocketAddr::V4(v4) = address
            && public_v4(*v4.ip())
            && let Ok(stream) = bind_connect(interface, *v4)
        {
            return Ok(stream);
        }
    }
    for address in addresses {
        let usable = match address.ip() {
            std::net::IpAddr::V4(ip) => {
                let [a, b, _, _] = ip.octets();
                public_v4(ip) || (a == 198 && (18..20).contains(&b))
            }
            std::net::IpAddr::V6(ip) => {
                !ip.is_loopback()
                    && !ip.is_unspecified()
                    && !ip.is_unique_local()
                    && !ip.is_unicast_link_local()
                    && !ip.is_multicast()
                    && ip.to_ipv4_mapped().is_none()
            }
        };
        if usable && let Ok(stream) = TcpStream::connect_timeout(&address, TIMEOUT) {
            return Ok(stream);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotConnected,
        "No route for Music host",
    ))
}

struct Relay {
    rules: Domains,
    interface: CString,
    pac_path: String,
    pac: String,
    active: AtomicUsize,
}

impl Relay {
    fn handle(&self, mut client: TcpStream) -> io::Result<()> {
        client.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut incoming = BufReader::new(client.try_clone()?);
        let mut lines = (&mut incoming).take(16 * 1024);
        let mut first = String::new();
        lines.read_line(&mut first)?;
        let fields: Vec<_> = first.split_ascii_whitespace().collect();
        if fields.len() != 3 || !matches!(fields[2], "HTTP/1.1" | "HTTP/1.0") {
            return client.write_all(FORBIDDEN);
        }
        let mut header = String::new();
        loop {
            header.clear();
            if lines.read_line(&mut header)? == 0 {
                return Ok(());
            }
            if header == "\r\n" {
                break;
            }
        }
        if fields[0] == "GET" && fields[1] == self.pac_path {
            write!(
                client,
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-ns-proxy-autoconfig\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                self.pac.len(),
                self.pac
            )?;
            return Ok(());
        }
        let host = fields[1]
            .strip_suffix(":443")
            .unwrap_or_default()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if fields[0] != "CONNECT" || !self.rules.matches(&host) {
            return client.write_all(FORBIDDEN);
        }
        let Ok(mut server) = upstream(&self.interface, &host) else {
            return client.write_all(BAD_GATEWAY);
        };
        client.set_read_timeout(None)?;
        client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;

        let mut forward = server.try_clone()?;
        let mut client_back = client.try_clone()?;
        let upload = thread::Builder::new().spawn(move || {
            let result = io::copy(&mut incoming, &mut forward);
            let _ = forward.shutdown(if result.is_ok() {
                Shutdown::Write
            } else {
                Shutdown::Both
            });
        })?;
        let result = io::copy(&mut server, &mut client_back);
        let _ = client.shutdown(Shutdown::Both);
        let _ = upload.join();
        result.map(|_| ())
    }
}

fn main() -> io::Result<()> {
    let Some(interface) = physical_interface() else {
        return Ok(());
    };
    let rules: Domains = serde_json::from_str(RULES)?;
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let mut entropy = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut entropy)?;
    let token = entropy
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let pac_path = format!("/{token}.pac");
    let pac = format!(
        r#"function FindProxyForURL(url, host) {{
            host = host.toLowerCase();
            var suffix = {};
            var exact = {};
            var proxy = 'PROXY 127.0.0.1:{port}; DIRECT';
            for (var i=0;i<exact.length;i++) if (host === exact[i]) return proxy;
            for (var j=0;j<suffix.length;j++) if (host === suffix[j] || dnsDomainIs(host,'.'+suffix[j])) return proxy;
            return 'DIRECT';
        }}"#,
        serde_json::to_string(&rules.suffixes)?,
        serde_json::to_string(&rules.hosts)?,
    );
    let report = serde_json::json!({"pac": format!("http://127.0.0.1:{port}{pac_path}"),
        "interface": interface.to_string_lossy()});
    println!("{report}");
    io::stdout().flush()?;
    thread::Builder::new().spawn(|| {
        let _ = io::copy(&mut io::stdin().lock(), &mut io::sink());
        std::process::exit(0);
    })?;

    let relay = Arc::new(Relay {
        rules,
        interface,
        pac_path,
        pac,
        active: AtomicUsize::new(0),
    });
    for stream in listener.incoming().flatten() {
        if relay.active.fetch_add(1, Ordering::Relaxed) >= 48 {
            relay.active.fetch_sub(1, Ordering::Relaxed);
            continue;
        }
        let task = relay.clone();
        if thread::Builder::new()
            .spawn(move || {
                let _ = task.handle(stream);
                task.active.fetch_sub(1, Ordering::Relaxed);
            })
            .is_err()
        {
            relay.active.fetch_sub(1, Ordering::Relaxed);
        }
    }
    Ok(())
}
