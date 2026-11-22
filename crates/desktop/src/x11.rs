use gdk4_x11::X11Surface;
use gtk::prelude::*;
use std::path::PathBuf;
use x11rb::{
    connection::Connection,
    protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt, EventMask},
};

fn window(w: &gtk::ApplicationWindow) -> anyhow::Result<(impl Connection + use<>, u32, u32)> {
    let surface = w
        .surface()
        .ok_or_else(|| anyhow::anyhow!("Окно ещё не открыто"))?;
    let surface = surface
        .downcast::<X11Surface>()
        .map_err(|_| anyhow::anyhow!("Требуется X11"))?;
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    Ok((conn, surface.xid() as u32, root))
}
fn pos_path() -> PathBuf {
    overtone_core::data_dir().join("mini-position.json")
}
pub fn supported(w: &gtk::ApplicationWindow) -> bool {
    gtk::prelude::WidgetExt::display(w)
        .type_()
        .name()
        .contains("X11")
}
pub fn place(w: &gtk::ApplicationWindow, above: bool, restore: bool) -> anyhow::Result<()> {
    let (conn, xid, root) = window(w)?;
    let atom =
        |name: &[u8]| -> anyhow::Result<_> { Ok(conn.intern_atom(false, name)?.reply()?.atom) };
    let above_atom = atom(b"_NET_WM_STATE_ABOVE")?;
    let wm_state = atom(b"_NET_WM_STATE")?;
    let event = ClientMessageEvent::new(32, xid, wm_state, [u32::from(above), above_atom, 0, 1, 0]);
    conn.send_event(
        false,
        root,
        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
        event,
    )?
    .check()?;
    if restore && let Ok((x, y)) = overtone_core::read_json::<(i32, i32)>(&pos_path(), 128) {
        let size = conn.get_geometry(xid)?.reply()?;
        let monitors = gtk::prelude::WidgetExt::display(w).monitors();
        let scale = w.scale_factor().max(1);
        let mut best = None;
        for i in 0..monitors.n_items() {
            let Some(m) = monitors.item(i).and_downcast::<gtk::gdk::Monitor>() else {
                continue;
            };
            let m = m.geometry();
            let gx = m.x() * scale;
            let gy = m.y() * scale;
            let right = (m.x() + m.width()) * scale - i32::from(size.width) - 12;
            let bottom = (m.y() + m.height()) * scale - i32::from(size.height) - 40;
            let nx = x.clamp(gx + 4, right.max(gx + 4));
            let ny = y.clamp(gy + 40, bottom.max(gy + 40));
            let distance = i64::from(nx - x).abs() + i64::from(ny - y).abs();
            if best.is_none_or(|(_, _, d)| distance < d) {
                best = Some((nx, ny, distance));
            }
        }
        if let Some((x, y, _)) = best {
            // EWMH asks the window manager to move the frame, avoiding reparenting offsets.
            let moveresize = atom(b"_NET_MOVERESIZE_WINDOW")?;
            let event = ClientMessageEvent::new(
                32,
                xid,
                moveresize,
                [
                    1 | (1 << 8) | (1 << 9) | (1 << 12),
                    x as u32,
                    y as u32,
                    0,
                    0,
                ],
            );
            conn.send_event(
                false,
                root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                event,
            )?
            .check()?;
        }
    }
    conn.flush()?;
    Ok(())
}
pub fn save(w: &gtk::ApplicationWindow) -> anyhow::Result<()> {
    let (conn, xid, root) = window(w)?;
    let pos = conn.translate_coordinates(xid, root, 0, 0)?.reply()?;
    let extents = conn
        .intern_atom(false, b"_NET_FRAME_EXTENTS")?
        .reply()?
        .atom;
    let prop = conn
        .get_property(false, xid, extents, AtomEnum::CARDINAL, 0, 4)?
        .reply()?;
    let ext: Vec<_> = prop.value32().into_iter().flatten().collect();
    let x = i32::from(pos.dst_x) - ext.first().copied().unwrap_or(0) as i32;
    let y = i32::from(pos.dst_y) - ext.get(2).copied().unwrap_or(0) as i32;
    overtone_core::write_json(&pos_path(), &(x, y))
}
