//! Software triangle rasterizer: draws art meshes into RGBA pixel
//! buffers, sampling the texture atlas through each mesh's UVs.

use crate::model::Vec2;
use image::{Rgba, RgbaImage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    Normal,
    Add,
    Multiply,
    Darken,
    MultiplyNew,
    ColorBurn,
    LinearBurn,
    Lighten,
    Screen,
    ColorDodge,
    Overlay,
    SoftLight,
    HardLight,
    LinearLight,
}

impl Blend {
    /// Legacy drawable flags (add / multiply).
    pub fn from_mode(mode: u8) -> Blend {
        match mode {
            1 => Blend::Add,
            2 => Blend::Multiply,
            _ => Blend::Normal,
        }
    }

    /// Cubism 5.3 colour blend id (order of the editor's blend list).
    pub fn from_v6(id: u8) -> Blend {
        match id {
            1 | 2 | 16 => Blend::Add,
            3 => Blend::Darken,
            4 => Blend::MultiplyNew,
            5 => Blend::ColorBurn,
            6 => Blend::LinearBurn,
            7 => Blend::Lighten,
            8 => Blend::Screen,
            9 => Blend::ColorDodge,
            10 => Blend::Overlay,
            11 => Blend::SoftLight,
            12 => Blend::HardLight,
            13 => Blend::LinearLight,
            17 => Blend::Multiply,
            _ => Blend::Normal,
        }
    }

    /// Separable blend function on straight colours in 0..1.
    fn mix(self, cb: f32, cs: f32) -> f32 {
        fn screen(a: f32, b: f32) -> f32 {
            a + b - a * b
        }
        fn hard(cb: f32, cs: f32) -> f32 {
            if cs <= 0.5 { cb * 2.0 * cs } else { screen(cb, 2.0 * cs - 1.0) }
        }
        match self {
            Blend::Normal => cs,
            Blend::Add => (cb + cs).min(1.0),
            Blend::Multiply | Blend::MultiplyNew => cb * cs,
            Blend::Darken => cb.min(cs),
            Blend::Lighten => cb.max(cs),
            Blend::Screen => screen(cb, cs),
            Blend::LinearBurn => (cb + cs - 1.0).max(0.0),
            Blend::ColorBurn => {
                if cb >= 1.0 { 1.0 } else if cs <= 0.0 { 0.0 } else { 1.0 - ((1.0 - cb) / cs).min(1.0) }
            }
            Blend::ColorDodge => {
                if cb <= 0.0 { 0.0 } else if cs >= 1.0 { 1.0 } else { (cb / (1.0 - cs)).min(1.0) }
            }
            Blend::HardLight => hard(cb, cs),
            Blend::Overlay => hard(cs, cb),
            Blend::LinearLight => (cb + 2.0 * cs - 1.0).clamp(0.0, 1.0),
            Blend::SoftLight => {
                if cs <= 0.5 {
                    cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
                } else {
                    let d = if cb <= 0.25 { ((16.0 * cb - 12.0) * cb + 4.0) * cb } else { cb.sqrt() };
                    cb + (2.0 * cs - 1.0) * (d - cb)
                }
            }
        }
    }
}

/// Cubism 5.3 alpha blend (Porter-Duff style compositing of alpha).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaMode {
    Over,
    Atop,
    Out,
    Conjoint,
    Disjoint,
}

impl AlphaMode {
    pub fn from_v6(id: u8) -> AlphaMode {
        match id {
            1 => AlphaMode::Atop,
            2 => AlphaMode::Out,
            4 => AlphaMode::Conjoint,
            8 => AlphaMode::Disjoint,
            _ => AlphaMode::Over,
        }
    }
}

/// A single art mesh ready to rasterize: pixel-space triangle vertices,
/// UVs, triangle index list.
pub struct MeshGeom {
    pub verts_px: Vec<Vec2>,
    pub uvs: Vec<Vec2>,
    pub indices: Vec<u32>,
    pub opacity: f32,
    pub blend: Blend,
    pub double_sided: bool,
    /// Optional mask alpha buffer (canvas size), applied to output alpha.
    pub mask: Option<RgbaImage>,
    pub inverted_mask: bool,
    /// Keyform multiply / screen colours (Cubism 4.2+); white / black = none.
    pub multiply: [f32; 3],
    pub screen: [f32; 3],
    pub alpha_mode: AlphaMode,
}

fn edge(a: Vec2, b: Vec2, p: Vec2) -> f32 {
    (p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x)
}

#[inline]
fn sample(tex: &RgbaImage, u: f32, v: f32) -> [u8; 4] {
    let (w, h) = (tex.width() as f32, tex.height() as f32);
    // bilinear sample, clamped
    let x = (u * w - 0.5).clamp(0.0, w - 1.001);
    let y = (v * h - 0.5).clamp(0.0, h - 1.001);
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(tex.width() - 1), (y0 + 1).min(tex.height() - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let p = |x: u32, y: u32| tex.get_pixel(x, y).0;
    let (p00, p10, p01, p11) = (p(x0, y0), p(x1, y0), p(x0, y1), p(x1, y1));
    let mut out = [0u8; 4];
    for c in 0..4 {
        let top = p00[c] as f32 + (p10[c] as f32 - p00[c] as f32) * fx;
        let bot = p01[c] as f32 + (p11[c] as f32 - p01[c] as f32) * fx;
        out[c] = (top + (bot - top) * fy) as u8;
    }
    out
}

pub fn blend_px(dst: &mut Rgba<u8>, src: [u8; 4], blend: Blend, alpha_mode: AlphaMode) {
    let sa = src[3] as f32 / 255.0;
    if sa <= 0.0 {
        return;
    }
    let da = dst.0[3] as f32 / 255.0;
    let sc = |c: usize| src[c] as f32 / 255.0;
    let d0 = dst.0;
    let dc = |c: usize| d0[c] as f32 / 255.0;
    // Alpha operators that ignore the colour blend.
    match alpha_mode {
        AlphaMode::Out => {
            dst.0[3] = (da * (1.0 - sa) * 255.0) as u8;
            return;
        }
        AlphaMode::Atop => {
            if da <= 0.0 {
                return;
            }
            for c in 0..3 {
                dst.0[c] = ((sc(c) * sa + dc(c) * (1.0 - sa)) * 255.0) as u8;
            }
            return;
        }
        AlphaMode::Conjoint | AlphaMode::Disjoint => {
            let fb = if da <= 0.0 {
                0.0
            } else if alpha_mode == AlphaMode::Conjoint {
                (1.0 - sa / da).max(0.0)
            } else {
                ((1.0 - sa) / da).min(1.0)
            };
            let out_a = (sa + da * fb).min(1.0);
            if out_a <= 0.0 {
                return;
            }
            for c in 0..3 {
                dst.0[c] = (((sc(c) * sa + dc(c) * da * fb) / out_a).clamp(0.0, 1.0) * 255.0) as u8;
            }
            dst.0[3] = (out_a * 255.0) as u8;
            return;
        }
        AlphaMode::Over => {}
    }
    match blend {
        Blend::Normal => {
            let out_a = sa + da * (1.0 - sa);
            if out_a <= 0.0 {
                return;
            }
            for c in 0..3 {
                dst.0[c] = (((sc(c) * sa + dc(c) * da * (1.0 - sa)) / out_a) * 255.0) as u8;
            }
            dst.0[3] = (out_a * 255.0) as u8;
        }
        Blend::Add => {
            let out_a = (sa + da * (1.0 - sa)).min(1.0);
            for c in 0..3 {
                dst.0[c] = ((dc(c) + sc(c) * sa).min(1.0) * 255.0) as u8;
            }
            dst.0[3] = (out_a * 255.0) as u8;
        }
        Blend::Multiply => {
            let out_a = sa + da * (1.0 - sa);
            for c in 0..3 {
                let r = dc(c) * sc(c);
                dst.0[c] = (((r * sa + dc(c) * da * (1.0 - sa)) / out_a.max(1e-6)) * 255.0) as u8;
            }
            dst.0[3] = (out_a * 255.0) as u8;
        }
        other => {
            // W3C compositing: Cr = (sa*((1-da)*Cs + da*B(Cb,Cs)) + (1-sa)*da*Cb) / ar
            let out_a = sa + da * (1.0 - sa);
            if out_a <= 0.0 {
                return;
            }
            for c in 0..3 {
                let b = other.mix(dc(c), sc(c));
                let v = (sa * ((1.0 - da) * sc(c) + da * b) + (1.0 - sa) * da * dc(c)) / out_a;
                dst.0[c] = (v.clamp(0.0, 1.0) * 255.0) as u8;
            }
            dst.0[3] = (out_a * 255.0) as u8;
        }
    }
}

/// Rasterize `mesh` into `dst` sampling `tex`.
pub fn draw_mesh(dst: &mut RgbaImage, tex: &RgbaImage, mesh: &MeshGeom) {
    let (w, h) = (dst.width() as i32, dst.height() as i32);
    for tri in mesh.indices.chunks_exact(3) {
        let (a, b, c) = (
            mesh.verts_px[tri[0] as usize],
            mesh.verts_px[tri[1] as usize],
            mesh.verts_px[tri[2] as usize],
        );
        let (ua, ub, uc) = (
            mesh.uvs[tri[0] as usize],
            mesh.uvs[tri[1] as usize],
            mesh.uvs[tri[2] as usize],
        );
        let area = edge(a, b, c);
        if area.abs() < 1e-7 {
            continue;
        }
        // backface culling unless double sided
        if area < 0.0 && !mesh.double_sided {
            continue;
        }
        let minx = (a.x.min(b.x).min(c.x).floor() as i32).max(0);
        let maxx = (a.x.max(b.x).max(c.x).ceil() as i32).min(w - 1);
        let miny = (a.y.min(b.y).min(c.y).floor() as i32).max(0);
        let maxy = (a.y.max(b.y).max(c.y).ceil() as i32).min(h - 1);
        for y in miny..=maxy {
            for x in minx..=maxx {
                let p = Vec2 { x: x as f32 + 0.5, y: y as f32 + 0.5 };
                let w0 = edge(b, c, p) / area;
                let w1 = edge(c, a, p) / area;
                let w2 = edge(a, b, p) / area;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let u = ua.x * w0 + ub.x * w1 + uc.x * w2;
                let v = ua.y * w0 + ub.y * w1 + uc.y * w2;
                let mut px = sample(tex, u, v);
                if mesh.multiply != [1.0; 3] || mesh.screen != [0.0; 3] {
                    for ch in 0..3 {
                        let mut c = px[ch] as f32 / 255.0 * mesh.multiply[ch];
                        c = c + mesh.screen[ch] - c * mesh.screen[ch];
                        px[ch] = (c.clamp(0.0, 1.0) * 255.0) as u8;
                    }
                }
                let mut alpha = px[3] as f32 / 255.0 * mesh.opacity;
                if let Some(mask) = &mesh.mask {
                    let m = mask.get_pixel(x as u32, y as u32).0[3] as f32 / 255.0;
                    alpha *= if mesh.inverted_mask { 1.0 - m } else { m };
                }
                px[3] = (alpha * 255.0) as u8;
                blend_px(dst.get_pixel_mut(x as u32, y as u32), px, mesh.blend, mesh.alpha_mode);
            }
        }
    }
}

/// Render an alpha-only image of a mesh (used to build clipping masks).
pub fn mesh_alpha(dst_size: (u32, u32), mesh: &MeshGeom, tex: &RgbaImage) -> RgbaImage {
    let mut out = RgbaImage::new(dst_size.0, dst_size.1);
    draw_mesh(&mut out, tex, mesh);
    out
}
