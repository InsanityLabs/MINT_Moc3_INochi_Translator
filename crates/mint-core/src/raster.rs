//! Software triangle rasterizer: draws art meshes into RGBA pixel
//! buffers, sampling the texture atlas through each mesh's UVs.

use crate::model::Vec2;
use image::{Rgba, RgbaImage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    Normal,
    Add,
    Multiply,
}

impl Blend {
    pub fn from_mode(mode: u8) -> Blend {
        match mode {
            1 => Blend::Add,
            2 => Blend::Multiply,
            _ => Blend::Normal,
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

fn blend_px(dst: &mut Rgba<u8>, src: [u8; 4], blend: Blend) {
    let sa = src[3] as f32 / 255.0;
    if sa <= 0.0 {
        return;
    }
    let da = dst.0[3] as f32 / 255.0;
    match blend {
        Blend::Normal => {
            // source-over with premultiplied handling
            let out_a = sa + da * (1.0 - sa);
            if out_a <= 0.0 {
                return;
            }
            for c in 0..3 {
                let s = src[c] as f32 / 255.0;
                let d = dst.0[c] as f32 / 255.0;
                dst.0[c] = (((s * sa + d * da * (1.0 - sa)) / out_a) * 255.0) as u8;
            }
            dst.0[3] = (out_a * 255.0) as u8;
        }
        Blend::Add => {
            let out_a = (sa + da * (1.0 - sa)).min(1.0);
            for c in 0..3 {
                let s = src[c] as f32 / 255.0;
                let d = dst.0[c] as f32 / 255.0;
                dst.0[c] = ((d + s * sa).min(1.0) * 255.0) as u8;
            }
            dst.0[3] = (out_a * 255.0) as u8;
        }
        Blend::Multiply => {
            let out_a = sa + da * (1.0 - sa);
            for c in 0..3 {
                let s = src[c] as f32 / 255.0;
                let d = dst.0[c] as f32 / 255.0;
                let r = d * s;
                dst.0[c] = (((r * sa + d * da * (1.0 - sa)) / out_a.max(1e-6)) * 255.0) as u8;
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
                let mut alpha = px[3] as f32 / 255.0 * mesh.opacity;
                if let Some(mask) = &mesh.mask {
                    let m = mask.get_pixel(x as u32, y as u32).0[3] as f32 / 255.0;
                    alpha *= if mesh.inverted_mask { 1.0 - m } else { m };
                }
                px[3] = (alpha * 255.0) as u8;
                blend_px(dst.get_pixel_mut(x as u32, y as u32), px, mesh.blend);
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
