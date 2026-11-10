//! Model rendering: evaluate meshes at current parameters and draw
//! them in draw order, honoring clipping masks and blend modes.

use crate::model::{Model, Vec2};
use crate::raster::{draw_mesh, draw_mesh_at, Blend, MeshGeom};
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
    let size = model.render_size();
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
    let size = model.render_size();
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


/// Render the camera window set by `model.px_scale` / `model.px_offset` into an
/// image of size `out`, split into horizontal bands drawn on `threads` threads.
/// Exact (masks, all blend modes, offscreen parts) and independent of the
/// canvas size, so it stays fast when zoomed in or the canvas is huge.
pub fn render_view(model: &mut Model, textures: &[RgbaImage], out: (u32, u32), threads: usize) -> RgbaImage {
    let moc = model.moc;
    let all: Vec<EvalMesh> = (0..moc.art_meshes.len()).map(|mi| eval_mesh(model, mi, out)).collect();
    let order = model.render_order();
    let bbox: Vec<(f32, f32, f32, f32)> = all
        .iter()
        .map(|e| {
            e.geom.verts_px.iter().fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |b, v| (b.0.min(v.x), b.1.min(v.y), b.2.max(v.x), b.3.max(v.y)))
        })
        .collect();
    let bands = threads.clamp(1, 32).min(out.1.max(1) as usize);
    let band_h = (out.1 as usize).div_ceil(bands).max(1);
    let mut img = RgbaImage::new(out.0, out.1);
    let pieces: Vec<(usize, RgbaImage)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..bands)
            .filter(|b| b * band_h < out.1 as usize)
            .map(|b| {
                let y0 = b * band_h;
                let h = band_h.min(out.1 as usize - y0) as u32;
                let (all, order, bbox) = (&all, &order, &bbox);
                s.spawn(move || (y0, render_band(moc, textures, all, order, bbox, out.0, y0 as f32, h)))
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("render band")).collect()
    });
    for (y0, band) in pieces {
        image::imageops::replace(&mut img, &band, 0, y0 as i64);
    }
    img
}

#[allow(clippy::too_many_arguments)]
fn render_band(
    moc: &crate::moc3::Moc3,
    textures: &[RgbaImage],
    all: &[EvalMesh],
    order: &[usize],
    bbox: &[(f32, f32, f32, f32)],
    w: u32,
    y0: f32,
    h: u32,
) -> RgbaImage {
    let y1 = y0 + h as f32;
    let in_band = |mi: usize| {
        let b = bbox[mi];
        b.3 >= y0 && b.1 <= y1 && b.2 >= 0.0 && b.0 <= w as f32
    };
    let tex_of = |e: &EvalMesh| &textures[e.texture_no.min(textures.len() - 1)];
    let mut canvas = RgbaImage::new(w, h);
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
    for &mi in order {
        let am = &moc.art_meshes[mi];
        if !am.visible || !am.enabled {
            continue;
        }
        let chain = chain_of(mi);
        let common = stack.iter().zip(&chain).take_while(|(s, c)| s.0 == **c).count();
        while stack.len() > common {
            composite(&mut canvas, &mut stack);
        }
        for &p in &chain[common..] {
            stack.push((p, RgbaImage::new(w, h)));
        }
        let e = &all[mi];
        if e.geom.opacity <= 0.0 || !in_band(mi) {
            continue;
        }
        let mask = if am.mask_count > 0 {
            let begin = am.mask_begin.max(0) as usize;
            let end = (begin + am.mask_count.max(0) as usize).min(moc.drawable_masks.len());
            let mut m = RgbaImage::new(w, h);
            for &src in &moc.drawable_masks[begin..end] {
                let src = src as usize;
                if src >= all.len() || !in_band(src) {
                    continue;
                }
                let mut buf = RgbaImage::new(w, h);
                let sg = &all[src].geom;
                let src_geom = MeshGeom {
                    verts_px: sg.verts_px.clone(),
                    uvs: sg.uvs.clone(),
                    indices: sg.indices.clone(),
                    opacity: 1.0,
                    blend: Blend::Normal,
                    double_sided: sg.double_sided,
                    mask: None,
                    inverted_mask: false,
                    multiply: [1.0; 3],
                    screen: [0.0; 3],
                    alpha_mode: crate::raster::AlphaMode::Over,
                };
                draw_mesh_at(&mut buf, tex_of(&all[src]), &src_geom, y0, None);
                for (mp, bp) in m.pixels_mut().zip(buf.pixels()) {
                    let a = (mp.0[3] as u32 + bp.0[3] as u32).min(255) as u8;
                    *mp = image::Rgba([255, 255, 255, a]);
                }
            }
            Some(m)
        } else {
            None
        };
        let target = match stack.last_mut() {
            Some((_, t)) => t,
            None => &mut canvas,
        };
        draw_mesh_at(target, tex_of(e), &e.geom, y0, mask.as_ref());
    }
    while !stack.is_empty() {
        composite(&mut canvas, &mut stack);
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    /// render_view over the whole canvas must equal render():
    /// `cargo test -p mint-core --release -- --ignored render_view`
    #[test]
    #[ignore]
    fn render_view_matches_render() {
        let dir = "../../testdata/samples/Ren";
        let moc = crate::Moc3::parse(&std::fs::read(format!("{dir}/Ren.moc3")).unwrap()).unwrap();
        let tex = vec![image::open(format!("{dir}/Ren.2048/texture_00.png")).unwrap().into_rgba8()];
        let mut m = Model::new(&moc);
        let full = render(&mut m, &tex);
        let mut m2 = Model::new(&moc);
        let t = std::time::Instant::now();
        let view = render_view(&mut m2, &tex, (full.width(), full.height()), 8);
        println!("render_view full canvas {}x{}: {:?}", full.width(), full.height(), t.elapsed());
        let mut worst = 0u32;
        let mut differing = 0usize;
        for (a, b) in full.pixels().zip(view.pixels()) {
            let d = (0..4).map(|i| (a.0[i] as i32 - b.0[i] as i32).unsigned_abs()).max().unwrap();
            worst = worst.max(d);
            if d > 2 { differing += 1; }
        }
        println!("worst channel difference {worst}, pixels differing by >2: {differing}");
        assert!(differing < 50, "render_view differs from render");
        // a zoomed window: 1000x700 px at 0.5x starting at canvas (1800, 300)
        let mut m3 = Model::new(&moc);
        m3.px_scale = 0.5;
        m3.px_offset = (900.0, 150.0);
        let t = std::time::Instant::now();
        let win = render_view(&mut m3, &tex, (1000, 700), 8);
        println!("window render 1000x700: {:?}", t.elapsed());
        assert_eq!(win.dimensions(), (1000, 700));
    }
}
