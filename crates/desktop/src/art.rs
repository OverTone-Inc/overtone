use gtk::{gdk_pixbuf::Pixbuf, gio, glib, prelude::*};
use overtone_core::Source;
use std::{
    io::Read,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

const FALLBACK: &[u8] = include_bytes!("../../../assets/icons/no-cover.png");
const MAX: usize = 2 * 1024 * 1024;
pub struct Art {
    pub image: gtk::Picture,
    request: mpsc::Sender<(u64, String)>,
    generation: Arc<AtomicU64>,
    current: std::cell::RefCell<String>,
    fallback: Option<gtk::gdk::Texture>,
}
impl Art {
    pub fn new(size: i32) -> Self {
        let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_static(FALLBACK));
        let fallback =
            Pixbuf::from_stream_at_scale(&stream, size, size, true, gio::Cancellable::NONE)
                .ok()
                .map(|pixbuf| gtk::gdk::Texture::for_pixbuf(&pixbuf));
        let image = gtk::Picture::new();
        image.set_paintable(fallback.as_ref());
        image.set_content_fit(gtk::ContentFit::Contain);
        image.set_size_request(size, size);
        image.add_css_class("art");
        image.set_tooltip_text(Some("Обложка недоступна"));
        let (tx, rx) = mpsc::channel::<(u64, String)>();
        let (output, result) = async_channel::bounded(1);
        let generation = Arc::new(AtomicU64::new(0));
        let version = generation.clone();
        let weak = image.downgrade();
        let displayed = generation.clone();
        let scale = image.scale_factor().max(1);
        std::thread::spawn(move || {
            while let Ok(mut job) = rx.recv() {
                while let Ok(next) = rx.try_recv() {
                    job = next;
                }
                let (rev, url) = job;
                if let Some(bytes) = artwork(&url, &version, rev)
                    && version.load(Ordering::Relaxed) == rev
                {
                    let _ = output.try_send((rev, bytes));
                }
            }
        });
        glib::MainContext::default().spawn_local(async move {
            while let Ok((rev, bytes)) = result.recv().await {
                let Some(image) = weak.upgrade() else { break };
                if displayed.load(Ordering::Relaxed) != rev {
                    continue;
                }
                let bytes = glib::Bytes::from_owned(bytes);
                let stream = gio::MemoryInputStream::from_bytes(&bytes);
                if let Ok(pixbuf) = Pixbuf::from_stream_at_scale(
                    &stream,
                    size * scale,
                    size * scale,
                    true,
                    gio::Cancellable::NONE,
                ) {
                    image.set_paintable(Some(&gtk::gdk::Texture::for_pixbuf(&pixbuf)));
                    image.set_tooltip_text(Some("Обложка записи"));
                }
            }
        });
        Self {
            image,
            request: tx,
            generation,
            current: Default::default(),
            fallback,
        }
    }
    pub fn update(&self, url: &str) {
        if *self.current.borrow() == url {
            return;
        }
        self.current.replace(url.into());
        let rev = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.image.set_paintable(self.fallback.as_ref());
        self.image.set_tooltip_text(Some("Обложка недоступна"));
        if !url.is_empty() {
            let _ = self.request.send((rev, url.into()));
        }
    }
}
impl Drop for Art {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
}
fn artwork(url: &str, generation: &AtomicU64, rev: u64) -> Option<Vec<u8>> {
    match Source::parse(url).ok()? {
        Source::Local(path) => overtone_core::read_limited(Path::new(&path), MAX)
            .ok()
            .filter(|b| raster(b)),
        Source::Audio(url) => {
            let config = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(3)))
                .https_only(true)
                .max_redirects(0)
                .build();
            let mut response = ureq::Agent::new_with_config(config).get(url).call().ok()?;
            let mut reader = response.body_mut().as_reader().take(MAX as u64 + 1);
            let mut bytes = Vec::new();
            let mut buf = [0u8; 8192];
            while generation.load(Ordering::Relaxed) == rev {
                let len = reader.read(&mut buf).ok()?;
                if len == 0 {
                    return (bytes.len() <= MAX && raster(&bytes)).then_some(bytes);
                }
                bytes.extend_from_slice(&buf[..len]);
            }
            None
        }
        _ => None,
    }
}
fn raster(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(b"\xff\xd8\xff")
        || bytes.get(..4) == Some(b"RIFF") && bytes.get(8..12) == Some(b"WEBP")
}
