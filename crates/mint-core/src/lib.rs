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
pub use model::{Model, Vec2};
