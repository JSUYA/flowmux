// SPDX-License-Identifier: GPL-3.0-or-later
//! Minimal shared color type.
//!
//! The GTK layer renders with VTE, which owns palette/selection colors; this
//! crate only needs a plain 8-bit-per-channel RGB carrier to pass a theme into
//! `GhosttyPane::apply_colors` without depending on GTK in the headless crate.

/// An 8-bit-per-channel RGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}
