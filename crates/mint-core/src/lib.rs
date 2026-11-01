//! MINT core: MOC3 parsing, pose evaluation, rasterization,
//! and emitters for editable formats (PSD, ORA, cmo3, INP).
//!
//! Clean-room implementation — no Live2D SDK code is used.

pub mod emit;
pub mod moc3;
pub mod model;
pub mod raster;
pub mod render;
pub mod inp;

pub use moc3::{Moc3, Canvas};

static GUESS_PAIRS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Opt in to combining X/Y parameter pairs beyond Cubism's standard template
/// (a guess from naming; off by default).
pub fn set_guess_pairs(on: bool) {
    GUESS_PAIRS.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn guess_pairs() -> bool {
    GUESS_PAIRS.load(std::sync::atomic::Ordering::Relaxed)
}
pub use model::{Model, Vec2};
