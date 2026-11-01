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
    let (multiply, screen) = model.mesh_colors(mi);
    EvalMesh {
        index: mi,
        geom: MeshGeom {
            multiply,
            screen,
            verts_px: verts.into_iter().map(|v| model.canvas_to_px(v)).collect(),
            uvs,
            indices,
            opacity,
            blend: if am.blend_v6 >= 0 { Blend::from_v6((am.blend_v6 & 0xFF) as u8) } else { Blend::from_mode(am.blend_mode) },
            alpha_mode: if am.blend_v6 >= 0 { crate::raster::AlphaMode::from_v6(((am.blend_v6 >> 8) & 0xFF) as u8) } else { crate::raster::AlphaMode::Over },
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
        let mut em = eval_mesh(model, src, size);
        em.geom.opacity = 1.0;
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
    // Offscreen parts (Cubism 5.3) are rendered into their own buffers and
    // composited back with the part's blend (incl. the "Out" cut-out).
    let moc = model.moc;
    let chain_of = |mi: usize| -> Vec<usize> {
        let mut chain = Vec::new();
        let (mut part, mut guard) = (moc.art_meshes[mi].parent_part, 0);
        while part >= 0 && (part as usize) < moc.parts.len() && guard < 64 {
            if moc.part_offscreen.get(part as usize).copied().unwrap_or(false) {
                chain.push(part as usize);
            }
            part = moc.parts[part as usize].parent_part;
            guard += 1;
        }
        chain.reverse();
        chain
    };
    let mut stack: Vec<(usize, RgbaImage)> = Vec::new();
    let composite = |canvas: &mut RgbaImage, stack: &mut Vec<(usize, RgbaImage)>| {
        let (part, buf) = stack.pop().unwrap();
        let v = moc.part_offscreen_blend.get(part).copied().unwrap_or(0);
        let blend = Blend::from_v6((v & 0xFF) as u8);
        let alpha = crate::raster::AlphaMode::from_v6(((v >> 8) & 0xFF) as u8);
        let target = match stack.last_mut() {
            Some((_, t)) => t,
            None => canvas,
        };
        for (d, s) in target.pixels_mut().zip(buf.pixels()) {
            if s.0[3] > 0 {
                crate::raster::blend_px(d, s.0, blend, alpha);
            }
        }
    };
    for &mi in &model.render_order() {
        let Some(em) = meshes.iter().find(|e| e.index == mi) else {
            continue;
        };
        let tex = &textures[em.texture_no.min(textures.len() - 1)];
        let chain = chain_of(mi);
        let common = stack.iter().zip(&chain).take_while(|(s, c)| s.0 == **c).count();
        while stack.len() > common {
            composite(&mut canvas, &mut stack);
        }
        for &p in &chain[common..] {
            stack.push((p, RgbaImage::new(size.0, size.1)));
        }
        match stack.last_mut() {
            Some((_, buf)) => draw_mesh(buf, tex, &em.geom),
            None => draw_mesh(&mut canvas, tex, &em.geom),
        }
    }
    while !stack.is_empty() {
        composite(&mut canvas, &mut stack);
    }
    canvas
}
