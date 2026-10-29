//! Model rendering: evaluate meshes at current parameters and draw
//! them in draw order, honoring clipping masks and blend modes.

use crate::model::{Model, Vec2};
use crate::raster::{draw_mesh, Blend, MeshGeom};
use image::RgbaImage;

/// Per-mesh geometry in canvas *pixel* space, plus UVs and indices.
pub struct EvalMesh {
    pub index: usize,
    pub geom: MeshGeom,
    pub draw_order: f32,
    pub texture_no: usize,
}

fn eval_mesh(model: &mut Model, mi: usize, canvas: (u32, u32)) -> EvalMesh {
    let _ = canvas;
    let am = &model.moc.art_meshes[mi];
    let vc = am.vertex_count.max(0) as usize;
    let (verts, opacity, draw_order) = model.art_mesh(mi);
    let uvs: Vec<Vec2> = (0..vc)
        .map(|v| {
            // uv_begin is in float units
            let uv = model.moc.uvs[(am.uv_begin.max(0) as usize / 2 + v).min(model.moc.uvs.len() - 1)];
            Vec2 { x: uv[0], y: uv[1] }
        })
        .collect();
    let indices: Vec<u32> = (0..am.position_index_count.max(0) as usize)
        .map(|i| model.moc.position_indices[(am.position_index_begin.max(0) as usize + i).min(model.moc.position_indices.len() - 1)] as u32)
        .collect();
    EvalMesh {
        index: mi,
        geom: MeshGeom {
            verts_px: verts.into_iter().map(|v| model.canvas_to_px(v)).collect(),
            uvs,
            indices,
            opacity,
            blend: Blend::from_mode(am.blend_mode),
            double_sided: am.double_sided,
            mask: None,
            inverted_mask: am.inverted_mask,
        },
        draw_order,
        texture_no: am.texture_no as usize,
    }
}

/// Build the clipping mask buffer for mesh `mi` (union of its mask
/// sources' coverage).
fn build_mask(model: &mut Model, mi: usize, textures: &[RgbaImage], size: (u32, u32)) -> Option<RgbaImage> {
    let am = &model.moc.art_meshes[mi];
    if am.mask_count <= 0 {
        return None;
    }
    let begin = am.mask_begin.max(0) as usize;
    let end = (begin + am.mask_count.max(0) as usize).min(model.moc.drawable_masks.len());
    let mut mask = RgbaImage::new(size.0, size.1);
    for &src in &model.moc.drawable_masks[begin..end] {
        let src = src as usize;
        if src >= model.moc.art_meshes.len() {
            continue;
        }
        let em = eval_mesh(model, src, size);
        let tex = &textures[em.texture_no.min(textures.len() - 1)];
        let mut buf = RgbaImage::new(size.0, size.1);
        draw_mesh(&mut buf, tex, &em.geom);
        // union: keep max coverage
        for (m, b) in mask.pixels_mut().zip(buf.pixels()) {
            let a = (m.0[3] as u32 + b.0[3] as u32).min(255) as u8;
            *m = image::Rgba([255, 255, 255, a]);
        }
    }
    Some(mask)
}

/// Evaluate every visible art mesh at the current pose, including
/// clipping-mask buffers.
pub fn evaluate(model: &mut Model, textures: &[RgbaImage]) -> Vec<EvalMesh> {
    let size = (
        model.moc.canvas.width.max(1.0) as u32,
        model.moc.canvas.height.max(1.0) as u32,
    );
    let mut out = Vec::new();
    for mi in 0..model.moc.art_meshes.len() {
        let am = &model.moc.art_meshes[mi];
        if !am.visible || !am.enabled {
            continue;
        }
        let mut em = eval_mesh(model, mi, size);
        if std::env::var("REAPER_NO_MASK").is_err() {
            em.geom.mask = build_mask(model, mi, textures, size);
        }
        out.push(em);
    }
    out
}

/// Evaluate and composite the full model at current parameters.
pub fn render(model: &mut Model, textures: &[RgbaImage]) -> RgbaImage {
    let size = (
        model.moc.canvas.width.max(1.0) as u32,
        model.moc.canvas.height.max(1.0) as u32,
    );
    let mut canvas = RgbaImage::new(size.0, size.1);
    let meshes = evaluate(model, textures);
    for &mi in &model.render_order() {
        let Some(em) = meshes.iter().find(|e| e.index == mi) else {
            continue;
        };
        let tex = &textures[em.texture_no.min(textures.len() - 1)];
        draw_mesh(&mut canvas, tex, &em.geom);
    }
    canvas
}
