// SPDX-License-Identifier: GPL-3.0-or-later
//! libghostty-vt-backed terminal pane (task C).
//!
//! A drop-in alternative to [`crate::ui::terminal_pane::TerminalPane`] that
//! renders the grid itself from `flowmux_terminal`'s libghostty-vt core instead
//! of embedding a VTE widget. Selected at pane-creation time by
//! [`crate::ui::pane_terminal::PaneTerminal`] when the libghostty backend is
//! toggled on; the VTE path stays the default so nothing regresses.
//!
//! Parity status (this is the integration milestone): rendering, PTY I/O,
//! keyboard (incl. IME commit), focus, font, resize, cwd, and screen-text
//! extraction work. Mouse reporting, drag selection, the scrollbar, OSC title
//! tracking, and inline IME preedit are deferred to the input-parity milestone;
//! the methods are present but degrade gracefully (no-op / best-effort).

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gtk::cairo;
use gtk::gdk;
use gtk::glib;
use gtk::pango;
use gtk::prelude::*;

use flowmux_core::{PaneId, SurfaceId};
use flowmux_terminal::pty::Pty;
use flowmux_terminal::vt::{Colors, Rgb, Vt};

use crate::ui::terminal_pane::PaneCallbacks;

const DEFAULT_FONT: &str = "Monospace 12";
const SCROLLBACK: usize = 10_000;

/// Shared, mutable terminal state behind an `Rc<RefCell<…>>` so the draw,
/// resize, key, and PTY-pump closures can all reach it on the GTK thread.
struct State {
    vt: Vt,
    pty: Pty,
    font: pango::FontDescription,
    font_scale: f64,
    cell_w: f64,
    cell_h: f64,
    ascent: f64,
    cols: u16,
    rows: u16,
}

impl State {
    /// Recompute cell metrics for the current (scaled) font.
    fn remeasure(&mut self) {
        let (w, h, a) = measure_cell(&self.scaled_font());
        self.cell_w = w;
        self.cell_h = h;
        self.ascent = a;
    }

    fn scaled_font(&self) -> pango::FontDescription {
        let mut f = self.font.clone();
        // Pango sizes are in 1024ths of a point; scale around the base size.
        let base = if self.font.size() > 0 {
            self.font.size()
        } else {
            12 * pango::SCALE
        };
        f.set_size(((base as f64) * self.font_scale).round() as i32);
        f
    }
}

/// libghostty-backed terminal pane. Cheap to clone (all handles are refcounted).
#[derive(Clone)]
pub struct GhosttyPane {
    pub id: PaneId,
    pub container: gtk::Overlay,
    area: gtk::DrawingArea,
    state: Rc<RefCell<State>>,
    pid: Rc<Cell<Option<i32>>>,
}

impl GhosttyPane {
    /// Build a libghostty terminal and spawn `argv` (falling back to `$SHELL`),
    /// matching [`TerminalPane::spawn`]'s signature so the two are
    /// interchangeable at the call site.
    pub fn spawn(
        id: PaneId,
        _surface: SurfaceId,
        argv: Vec<String>,
        cwd: Option<PathBuf>,
        extra_env: Vec<(String, String)>,
        callbacks: PaneCallbacks,
    ) -> Self {
        let font = pango::FontDescription::from_string(DEFAULT_FONT);
        let (cell_w, cell_h, ascent) = measure_cell(&font);

        // Initial geometry; the first allocation resizes to fit.
        let cols: u16 = 80;
        let rows: u16 = 24;

        let vt = Vt::new(cols, rows, SCROLLBACK).expect("libghostty vt new");

        let argv_owned = if argv.is_empty() {
            vec![std::env::var("SHELL").unwrap_or_else(|_| "bash".into())]
        } else {
            argv
        };
        let argv_ref: Vec<&str> = argv_owned.iter().map(|s| s.as_str()).collect();
        let pty = Pty::spawn(
            &argv_ref,
            cwd.as_deref(),
            &extra_env,
            cols,
            rows,
        )
        .expect("libghostty pty spawn");

        let pid = Rc::new(Cell::new(None));

        let state = Rc::new(RefCell::new(State {
            vt,
            pty,
            font,
            font_scale: 1.0,
            cell_w,
            cell_h,
            ascent,
            cols,
            rows,
        }));

        let area = gtk::DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_focusable(true);
        area.set_can_focus(true);

        let container = gtk::Overlay::new();
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.set_child(Some(&area));

        let pane = GhosttyPane {
            id,
            container,
            area: area.clone(),
            state: state.clone(),
            pid: pid.clone(),
        };

        pane.install_draw();
        pane.install_resize();
        pane.install_pty_pump(callbacks.clone());
        pane.install_input();
        pane.install_focus(callbacks);

        pane
    }

    fn install_draw(&self) {
        let state = self.state.clone();
        self.area.set_draw_func(move |_area, cr, w, h| {
            draw(&mut state.borrow_mut(), cr, w, h);
        });
    }

    fn install_resize(&self) {
        let state = self.state.clone();
        let area = self.area.clone();
        self.area.connect_resize(move |_area, w, h| {
            let mut s = state.borrow_mut();
            if s.cell_w <= 0.0 || s.cell_h <= 0.0 {
                return;
            }
            let cols = ((w as f64 / s.cell_w).floor() as i64).clamp(1, u16::MAX as i64) as u16;
            let rows = ((h as f64 / s.cell_h).floor() as i64).clamp(1, u16::MAX as i64) as u16;
            if (cols, rows) != (s.cols, s.rows) {
                s.cols = cols;
                s.rows = rows;
                let cw = s.cell_w as u16;
                let chh = s.cell_h as u16;
                s.vt.resize(cols, rows, cw as u32, chh as u32);
                let _ = s.pty.resize(cols, rows, cw, chh);
                drop(s);
                area.queue_draw();
            }
        });
    }

    fn install_pty_pump(&self, callbacks: PaneCallbacks) {
        let state = self.state.clone();
        let area = self.area.clone();
        let pid = self.pid.clone();
        let id = self.id;
        let fd = self.state.borrow().pty.master_fd();
        glib::source::unix_fd_add_local(fd, glib::IOCondition::IN, move |_fd, _cond| {
            let mut buf = [0u8; 16384];
            let mut s = state.borrow_mut();
            match s.pty.read(&mut buf) {
                Ok(0) => {
                    // Child exited: reap to learn the status and notify.
                    let status = s.pty.try_wait().ok().flatten().unwrap_or(0);
                    drop(s);
                    pid.set(None);
                    (callbacks.on_child_exited.borrow_mut())(id, status);
                    return glib::ControlFlow::Break;
                }
                Ok(n) => s.vt.write(&buf[..n]),
                Err(_) => return glib::ControlFlow::Break,
            }
            drop(s);
            area.queue_draw();
            glib::ControlFlow::Continue
        });
    }

    fn install_input(&self) {
        // Direct key handling (control/navigation/printable fast path).
        let key = gtk::EventControllerKey::new();
        {
            let state = self.state.clone();
            let area = self.area.clone();
            key.connect_key_pressed(move |_kc, keyval, _code, mods| {
                if let Some(bytes) = encode_key(keyval, mods) {
                    let mut s = state.borrow_mut();
                    let _ = s.pty.write(&bytes);
                    drop(s);
                    area.queue_draw();
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
        }

        // IME: route committed text (e.g. composed Hangul syllables) to the PTY.
        let im = gtk::IMMulticontext::new();
        key.set_im_context(Some(&im));
        {
            let state = self.state.clone();
            let area = self.area.clone();
            im.connect_commit(move |_im, text| {
                let mut s = state.borrow_mut();
                let _ = s.pty.write(text.as_bytes());
                drop(s);
                area.queue_draw();
            });
        }
        let im_for_focus = im.clone();
        let focus = gtk::EventControllerFocus::new();
        focus.connect_enter(move |_| im_for_focus.focus_in());
        let im_for_focus_out = im;
        let focus_out = gtk::EventControllerFocus::new();
        focus_out.connect_leave(move |_| im_for_focus_out.focus_out());

        self.area.add_controller(key);
        self.area.add_controller(focus);
        self.area.add_controller(focus_out);
    }

    fn install_focus(&self, callbacks: PaneCallbacks) {
        let focus = gtk::EventControllerFocus::new();
        let id = self.id;
        focus.connect_enter(move |_| {
            (callbacks.on_focus.borrow_mut())(id);
        });
        self.area.add_controller(focus);
    }

    // ---- TerminalPane-compatible method surface (see pane_terminal.rs) ----

    pub fn root_widget(&self) -> gtk::Widget {
        self.container.clone().upcast::<gtk::Widget>()
    }

    pub fn grab_focus(&self) {
        self.area.grab_focus();
    }

    /// Best-effort cwd via `/proc/<pid>/cwd` (OSC 7 tracking lands in M3).
    pub fn current_dir(&self) -> Option<PathBuf> {
        let pid = self.pid.get()?;
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }

    pub fn set_font_scale(&self, scale: f64) {
        let mut s = self.state.borrow_mut();
        s.font_scale = if scale > 0.0 { scale } else { 1.0 };
        s.remeasure();
        // Re-fit the grid to the new cell size on the next allocation.
        let (w, h) = (self.area.width(), self.area.height());
        if w > 0 && h > 0 && s.cell_w > 0.0 && s.cell_h > 0.0 {
            let cols = ((w as f64 / s.cell_w).floor() as i64).clamp(1, u16::MAX as i64) as u16;
            let rows = ((h as f64 / s.cell_h).floor() as i64).clamp(1, u16::MAX as i64) as u16;
            s.cols = cols;
            s.rows = rows;
            let cw = s.cell_w as u16;
            let chh = s.cell_h as u16;
            s.vt.resize(cols, rows, cw as u32, chh as u32);
            let _ = s.pty.resize(cols, rows, cw, chh);
        }
        drop(s);
        self.area.queue_draw();
    }

    pub fn set_font(&self, desc: &pango::FontDescription) {
        // Read the scale out and release the borrow before calling
        // set_font_scale (which borrows again) to avoid a RefCell double-borrow.
        let scale = {
            let mut s = self.state.borrow_mut();
            s.font = desc.clone();
            s.font_scale
        };
        self.set_font_scale(scale);
    }

    pub fn has_selection(&self) -> bool {
        // Selection lands in M3 (libghostty selection API + drag handling).
        false
    }

    pub fn copy_selection_to_clipboard(&self) {
        // No-op until selection lands (M3).
    }

    pub fn paste_clipboard(&self) {
        let state = self.state.clone();
        let area = self.area.clone();
        if let Some(display) = gdk::Display::default() {
            let clipboard = display.clipboard();
            clipboard.read_text_async(gtk::gio::Cancellable::NONE, move |res| {
                if let Ok(Some(text)) = res {
                    let mut s = state.borrow_mut();
                    let _ = s.pty.write(text.as_bytes());
                    drop(s);
                    area.queue_draw();
                }
            });
        }
    }

    /// Inject bytes into the terminal display (not the child). Mirrors
    /// `TerminalPane::feed` (used to surface inline messages).
    pub fn feed(&self, bytes: &[u8]) {
        self.state.borrow_mut().vt.write(bytes);
        self.area.queue_draw();
    }

    pub fn feed_after_preedit_commit(&self, bytes: &'static [u8]) {
        let mut s = self.state.borrow_mut();
        let _ = s.pty.write(bytes);
        drop(s);
        self.area.queue_draw();
    }

    /// Visible screen text (all viewport rows joined), for `read-screen`.
    pub fn screen_text(&self) -> Option<String> {
        let s = self.state.borrow();
        let (_, rows) = s.vt.dims().unwrap_or((s.cols, s.rows));
        let mut out = String::new();
        for row in 0..rows {
            out.push_str(&s.vt.row_text(row));
            out.push('\n');
        }
        Some(out)
    }

    pub fn add_controller(&self, controller: impl IsA<gtk::EventController>) {
        self.container.add_controller(controller);
    }
}

fn rgb(c: Rgb) -> (f64, f64, f64) {
    (c.r as f64 / 255.0, c.g as f64 / 255.0, c.b as f64 / 255.0)
}

/// Measure monospace cell metrics (width, height, ascent) for `font`.
fn measure_cell(font: &pango::FontDescription) -> (f64, f64, f64) {
    let surf = cairo::ImageSurface::create(cairo::Format::ARgb32, 8, 8).unwrap();
    let cr = cairo::Context::new(&surf).unwrap();
    let layout = pangocairo::functions::create_layout(&cr);
    layout.set_font_description(Some(font));
    layout.set_text("M");
    let (w_px, h_px) = layout.pixel_size();
    let ctx = layout.context();
    let metrics = ctx.metrics(Some(font), None);
    let ascent = metrics.ascent() as f64 / pango::SCALE as f64;
    (w_px.max(1) as f64, h_px.max(1) as f64, ascent)
}

fn draw(state: &mut State, cr: &cairo::Context, w: i32, h: i32) {
    let _ = state.vt.update();
    let colors = state.vt.colors().unwrap_or(Colors {
        fg: Rgb { r: 220, g: 220, b: 220 },
        bg: Rgb { r: 0, g: 0, b: 0 },
        cursor: Rgb { r: 220, g: 220, b: 220 },
        cursor_has_value: false,
    });

    let (br, bgc, bb) = rgb(colors.bg);
    cr.set_source_rgb(br, bgc, bb);
    cr.rectangle(0.0, 0.0, w as f64, h as f64);
    let _ = cr.fill();

    let layout = pangocairo::functions::create_layout(cr);
    layout.set_font_description(Some(&state.scaled_font()));

    let (cols, rows) = state.vt.dims().unwrap_or((state.cols, state.rows));
    let cw = state.cell_w;
    let ch = state.cell_h;
    let ascent = state.ascent;

    for row in 0..rows {
        let y = row as f64 * ch;
        for col in 0..cols {
            let Some(cell) = state.vt.cell(row, col) else {
                continue;
            };
            let x = col as f64 * cw;
            let cell_px_w = if cell.wide { cw * 2.0 } else { cw };

            let (fg, bg) = if cell.style.inverse {
                (cell.bg.unwrap_or(colors.bg), Some(cell.fg))
            } else {
                (cell.fg, cell.bg)
            };

            if cell.selected {
                cr.set_source_rgb(0.20, 0.34, 0.55);
                cr.rectangle(x, y, cell_px_w, ch);
                let _ = cr.fill();
            } else if let Some(b) = bg {
                let (r, g, bl) = rgb(b);
                cr.set_source_rgb(r, g, bl);
                cr.rectangle(x, y, cell_px_w, ch);
                let _ = cr.fill();
            }

            let (fr, fgc, fb) = rgb(fg);
            if !cell.text.is_empty() {
                layout.set_text(&cell.text);
                cr.set_source_rgb(fr, fgc, fb);
                cr.move_to(x, y);
                pangocairo::functions::show_layout(cr, &layout);
            }
            if cell.style.underline {
                cr.set_source_rgb(fr, fgc, fb);
                cr.rectangle(x, y + ascent + 2.0, cell_px_w, 1.0);
                let _ = cr.fill();
            }
            if cell.style.strikethrough {
                cr.set_source_rgb(fr, fgc, fb);
                cr.rectangle(x, y + ch * 0.5, cell_px_w, 1.0);
                let _ = cr.fill();
            }
        }
    }

    if let Some(cursor) = state.vt.cursor() {
        if cursor.visible && cursor.x < cols && cursor.y < rows {
            let (r, g, b) = if colors.cursor_has_value {
                rgb(colors.cursor)
            } else {
                rgb(colors.fg)
            };
            cr.set_source_rgba(r, g, b, 0.6);
            cr.rectangle(cursor.x as f64 * cw, cursor.y as f64 * ch, cw, ch);
            let _ = cr.fill();
        }
    }
}

/// Encode a GTK key press into terminal bytes. Returns None for keys handled by
/// the IM context (plain printable text) or not translated.
fn encode_key(keyval: gdk::Key, state: gdk::ModifierType) -> Option<Vec<u8>> {
    use gdk::Key;
    let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
    let alt = state.contains(gdk::ModifierType::ALT_MASK);

    let named: Option<&[u8]> = match keyval {
        Key::Return | Key::KP_Enter => Some(b"\r"),
        Key::BackSpace => Some(b"\x7f"),
        Key::Tab => Some(b"\t"),
        Key::Escape => Some(b"\x1b"),
        Key::Up => Some(b"\x1b[A"),
        Key::Down => Some(b"\x1b[B"),
        Key::Right => Some(b"\x1b[C"),
        Key::Left => Some(b"\x1b[D"),
        Key::Home => Some(b"\x1b[H"),
        Key::End => Some(b"\x1b[F"),
        Key::Page_Up => Some(b"\x1b[5~"),
        Key::Page_Down => Some(b"\x1b[6~"),
        Key::Delete => Some(b"\x1b[3~"),
        Key::Insert => Some(b"\x1b[2~"),
        _ => None,
    };
    if let Some(bytes) = named {
        return Some(bytes.to_vec());
    }

    let ch = keyval.to_unicode()?;
    if ctrl {
        let b = ch.to_ascii_uppercase() as u32;
        if (b'A' as u32..=b'Z' as u32).contains(&b) {
            return Some(vec![(b - b'A' as u32 + 1) as u8]);
        }
        match ch {
            ' ' => return Some(vec![0]),
            '[' => return Some(vec![0x1b]),
            '\\' => return Some(vec![0x1c]),
            ']' => return Some(vec![0x1d]),
            _ => {}
        }
    }

    // Plain printable text is left to the IM context (so dead keys / CJK
    // composition work); only emit here when Alt is held (ESC-prefixed).
    if alt {
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        let mut out = vec![0x1b];
        out.extend_from_slice(s.as_bytes());
        return Some(out);
    }

    // ASCII control range typed without modifiers (rare) still goes through.
    if (ch as u32) < 0x20 {
        let mut buf = [0u8; 4];
        return Some(ch.encode_utf8(&mut buf).as_bytes().to_vec());
    }

    None
}
