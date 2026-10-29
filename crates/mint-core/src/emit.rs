//! Emission of editable layer documents (PSD/ORA) from evaluated meshes.

use crate::model::Model;
use crate::raster::{draw_mesh, Blend};
use crate::render::evaluate;
use image::RgbaImage;

/// A document layer: cropped RGBA image plus canvas placement.
pub struct LayerImage {
    pub name: String,
    pub mesh_idx: usize,
    pub image: RgbaImage,
    pub left: i32,
    pub top: i32,
    pub opacity: f32,
    pub blend: Blend,
    /// Indices of art meshes clipping this layer, if any.
    pub mask_sources: Vec<usize>,
    pub inverted_mask: bool,
}

pub struct LayerGroup {
    pub name: String,
    /// Bottom-to-top order.
    pub children: Vec<LayerNode>,
}

pub enum LayerNode {
    Group(LayerGroup),
    Image(LayerImage),
}

impl LayerNode {
    fn min_render_pos(&self, render_pos: &[usize]) -> usize {
        match self {
            LayerNode::Image(li) => render_pos.get(li.mesh_idx).copied().unwrap_or(usize::MAX),
            LayerNode::Group(g) => g
                .children
                .iter()
                .map(|c| c.min_render_pos(render_pos))
                .min()
                .unwrap_or(usize::MAX),
        }
    }
}

fn alpha_bbox(img: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = (img.width(), img.height());
    let (mut minx, mut miny, mut maxx, mut maxy) = (w, h, 0u32, 0u32);
    let mut any = false;
    for y in 0..h {
        for x in 0..w {
            if img.get_pixel(x, y).0[3] > 0 {
                any = true;
                minx = minx.min(x);
                miny = miny.min(y);
                maxx = maxx.max(x);
                maxy = maxy.max(y);
            }
        }
    }
    any.then_some((minx, miny, maxx + 1, maxy + 1))
}

/// Render every visible art mesh into its own canvas-space layer,
/// then nest layers under their part hierarchy.
///
/// If export_all is true, includes even meshes with no visible pixels at default pose
/// (animation/pose-toggle layers). Otherwise skips them.
///
/// Returns (root group, canvas width, canvas height); children are
/// in bottom-to-top order.
pub fn build_layers(model: &mut Model, textures: &[RgbaImage], export_all: bool) -> (LayerGroup, u32, u32) {
    let moc = model.moc;
    let size = (
        moc.canvas.width.max(1.0) as u32,
        moc.canvas.height.max(1.0) as u32,
    );
    let evals = evaluate(model, textures);
    let order = model.render_order();
    let mut render_pos = vec![usize::MAX; moc.art_meshes.len()];
    for (pos, &mi) in order.iter().enumerate() {
        render_pos[mi] = pos;
    }

    // render each evaluated mesh into a canvas buffer, crop to alpha bbox
    let mut layer_of: Vec<Option<LayerImage>> = (0..moc.art_meshes.len()).map(|_| None).collect();
    let mut skipped_empty = 0;
    for em in &evals {
        let am = &moc.art_meshes[em.index];
        let mut buf = RgbaImage::new(size.0, size.1);
        let tex = &textures[em.texture_no.min(textures.len().saturating_sub(1))];
        // For export_all, force opacity to 1.0 to reveal texture content even if hidden at default pose
        let draw_opacity = if export_all { 1.0 } else { em.geom.opacity };
        let geom = crate::raster::MeshGeom {
            verts_px: em.geom.verts_px.clone(),
            uvs: em.geom.uvs.clone(),
            indices: em.geom.indices.clone(),
            opacity: draw_opacity,
            blend: em.geom.blend,
            double_sided: em.geom.double_sided,
            mask: em.geom.mask.clone(),
            inverted_mask: em.geom.inverted_mask,
        };
        draw_mesh(&mut buf, tex, &geom);
        let bbox = alpha_bbox(&buf);
        if !export_all && bbox.is_none() {
            skipped_empty += 1;
            eprintln!("skipping mesh {} ({}): no visible pixels at default pose", em.index, am.id);
            continue;
        }
        let (x0, y0, x1, y1) = bbox.unwrap_or((0, 0, size.0, size.1));
        let cropped = image::imageops::crop_imm(&buf, x0, y0, x1 - x0, y1 - y0).to_image();
        let begin = am.mask_begin.max(0) as usize;
        let end = (begin + am.mask_count.max(0) as usize).min(moc.drawable_masks.len());
        layer_of[em.index] = Some(LayerImage {
            name: if am.id.is_empty() { format!("ArtMesh{}", em.index) } else { am.id.clone() },
            mesh_idx: em.index,
            image: cropped,
            left: x0 as i32,
            top: y0 as i32,
            opacity: if export_all { 1.0 } else { em.geom.opacity.clamp(0.0, 1.0) },
            blend: em.geom.blend,
            mask_sources: moc.drawable_masks[begin..end]
                .iter()
                .map(|&m| m.max(0) as usize)
                .collect(),
            inverted_mask: am.inverted_mask,
        });
    }
    eprintln!("exported {} layers, skipped {} empty meshes", layer_of.iter().filter(|x| x.is_some()).count(), skipped_empty);

    // part tree
    let mut part_children: Vec<Vec<usize>> = vec![Vec::new(); moc.parts.len()];
    let mut root_parts: Vec<usize> = Vec::new();
    for (i, p) in moc.parts.iter().enumerate() {
        if p.parent_part >= 0 && (p.parent_part as usize) < moc.parts.len() {
            part_children[p.parent_part as usize].push(i);
        } else {
            root_parts.push(i);
        }
    }
    let mut mesh_children: Vec<Vec<usize>> = vec![Vec::new(); moc.parts.len()];
    let mut root_meshes: Vec<usize> = Vec::new();
    for (i, a) in moc.art_meshes.iter().enumerate() {
        if layer_of[i].is_none() {
            continue;
        }
        if a.parent_part >= 0 && (a.parent_part as usize) < moc.parts.len() {
            mesh_children[a.parent_part as usize].push(i);
        } else {
            root_meshes.push(i);
        }
    }

    fn subtree(
        p: usize,
        part_children: &[Vec<usize>],
        mesh_children: &[Vec<usize>],
        render_pos: &[usize],
        layer_of: &mut [Option<LayerImage>],
        moc: &crate::moc3::Moc3,
    ) -> Option<LayerNode> {
        let mut subpart_nodes: Vec<LayerNode> = Vec::new();
        for &sp in &part_children[p] {
            if let Some(n) = subtree(sp, part_children, mesh_children, render_pos, layer_of, moc) {
                subpart_nodes.push(n);
            }
        }
        let mesh_indices = mesh_children[p].clone();
        let mut mesh_nodes: Vec<LayerNode> = Vec::new();
        for &mi in &mesh_indices {
            mesh_nodes.push(LayerNode::Image(layer_of[mi].take().unwrap()));
        }
        if subpart_nodes.is_empty() && mesh_nodes.is_empty() {
            return None;
        }
        // Merge and sort by render position
        let mut items: Vec<LayerNode> = Vec::new();
        items.extend(subpart_nodes);
        items.extend(mesh_nodes);
        items.sort_by_key(|n| n.min_render_pos(render_pos));
        Some(LayerNode::Group(LayerGroup {
            name: moc.parts[p].id.clone(),
            children: items,
        }))
    }

    let mut items: Vec<LayerNode> = Vec::new();
    for &rp in &root_parts {
        if let Some(n) = subtree(rp, &part_children, &mesh_children, &render_pos, &mut layer_of, moc)
        {
            items.push(n);
        }
    }
    for &mi in &root_meshes {
        items.push(LayerNode::Image(layer_of[mi].take().unwrap()));
    }
    items.sort_by_key(|n| n.min_render_pos(&render_pos));

    (
        LayerGroup {
            name: "root".into(),
            children: items,
        },
        size.0,
        size.1,
    )
}

pub mod ora;
pub mod psd;
pub use crate::inp::write_inp;

/// Raw PNG bytes of every texture referenced by `<name>.model3.json`
/// next to the given `.moc3` (atlas page order).
pub fn texture_pngs(moc_path: &std::path::Path) -> Vec<Vec<u8>> {
    let model3 = moc_path.with_extension("model3.json");
    let dir = model3.parent().unwrap_or(std::path::Path::new("."));
    let Ok(bytes) = std::fs::read(&model3) else { return Vec::new() };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return Vec::new() };
    json["FileReferences"]["Textures"]
        .as_array()
        .map(|a| a.iter().filter_map(|t| std::fs::read(dir.join(t.as_str()?.replace("\\", "/"))).ok()).collect())
        .unwrap_or_default()
}

/// Result of a Cubism export: the `.cmo3` plus a `.can3` animation project
/// when the source `model3.json` manifest is available.
pub struct Cmo3Export {
    pub cmo3: Vec<u8>,
    pub can3: Option<Vec<u8>>,
}

/// Configures a decompiler with the shared options: draw ranks, deformer
/// parents, visibility, model name, icons, and cdi3 display names.
fn configured_decompiler(
    moc: &crate::moc3::Moc3,
    moc_path: &std::path::Path,
    model: &mut crate::model::Model,
    textures: &[image::RgbaImage],
    force_visible: bool,
) -> moc2cmo::Decompiler {
    let mut dec = moc2cmo::Decompiler::new();
    dec.set_force_visible(force_visible);
    let mut ranks = vec![0i32; moc.art_meshes.len()];
    for (pos, m) in model.render_order().into_iter().enumerate() {
        ranks[m] = pos as i32;
    }
    dec.set_draw_ranks(ranks);
    dec.set_deformer_parts(
        moc.deformers
            .iter()
            .map(|d| (d.parent_part >= 0).then_some(d.parent_part as usize))
            .collect(),
    );
    if let Some(stem) = moc_path.file_stem().and_then(|s| s.to_str()) {
        dec.set_model_name(stem);
    }
    dec.set_icons(make_icons(model, textures));
    let mut names = moc2cmo::DisplayNames::default();
    if let Ok(info) =
        moc2cmo::displayinfo3::DisplayInfo3::from_json_file(moc_path.with_extension("cdi3.json"))
    {
        names = moc2cmo::DisplayNames::from_display_info(&info);
    }
    names.hidden_parts = hidden_parts(moc, Some(moc_path));
    dec.set_display_names(names);
    dec
}

/// Parts the runtime hides at startup: the non-first members of each
/// `pose3.json` group, or (without a pose file) the `B` variant of an
/// `A`/`B` arm pair such as `PartArmB` or `Part01ArmRB001`.
pub fn hidden_parts(moc: &crate::moc3::Moc3, moc_path: Option<&std::path::Path>) -> std::collections::HashSet<String> {
    let mut hidden: std::collections::HashSet<String> = moc
        .parts
        .iter()
        .zip(&moc.offscreen_out_parts)
        .filter(|(_, o)| **o)
        .map(|(p, _)| p.id.clone())
        .collect();
    let pose = moc_path.and_then(|p| std::fs::read(p.with_extension("pose3.json")).ok())
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    if let Some(groups) = pose.as_ref().and_then(|p| p["Groups"].as_array()) {
        for g in groups.iter().filter_map(|g| g.as_array()) {
            for (i, item) in g.iter().enumerate() {
                if i == 0 { continue; }
                if let Some(id) = item["Id"].as_str() { hidden.insert(id.to_string()); }
                for l in item["Link"].as_array().into_iter().flatten().filter_map(|l| l.as_str()) {
                    hidden.insert(l.to_string());
                }
            }
        }
        return hidden;
    }
    let ids: std::collections::HashSet<&str> = moc.parts.iter().map(|p| p.id.as_str()).collect();
    for p in &moc.parts {
        let id = p.id.as_str();
        let Some(pos) = id.find("Arm") else { continue };
        let rest = &id[pos + 3..];
        let side = rest.chars().take_while(|c| *c == 'L' || *c == 'R').count();
        if rest[side..].starts_with('B') {
            let at = pos + 3 + side;
            let a = format!("{}A{}", &id[..at], &id[at + 1..]);
            if ids.contains(a.as_str()) { hidden.insert(id.to_string()); }
        }
    }
    hidden
}

/// Renders the model once and produces the 64/32/16px icon PNGs Cubism shows
/// in its model picker.
fn make_icons(
    model: &mut crate::model::Model,
    textures: &[image::RgbaImage],
) -> Vec<(u32, Vec<u8>)> {
    if textures.is_empty() {
        return Vec::new();
    }
    let render = crate::render::render(model, textures);
    let mut out = Vec::new();
    for size in [64u32, 32, 16] {
        let icon = image::imageops::resize(
            &render,
            size,
            size,
            image::imageops::FilterType::Lanczos3,
        );
        let mut buf = std::io::Cursor::new(Vec::new());
        if icon
            .write_to(&mut buf, image::ImageFormat::Png)
            .is_ok()
        {
            out.push((size, buf.into_inner()));
        }
    }
    out
}

/// Export a Cubism `.cmo3` for `moc`. When the `model3.json` manifest sits
/// next to `moc_path`, the export also embeds the physics/display-info/
/// expression/user-data JSON resources, restores editor display names and
/// parameter groups, and builds a `.can3` animation project; otherwise it
/// decompiles the `.moc3` directly (still reading a sibling `.cdi3.json`
/// for names).
pub fn export_cmo3(
    moc: &crate::moc3::Moc3,
    moc_path: &std::path::Path,
    model: &mut crate::model::Model,
    textures: &[image::RgbaImage],
    force_visible: bool,
) -> Result<Cmo3Export, String> {
    let dec = configured_decompiler(moc, moc_path, model, textures, force_visible);
    let model3 = moc_path.with_extension("model3.json");
    if model3.exists() {
        let projects =
            moc2cmo::decompile_model3_with(&model3, dec).map_err(|e| e.to_string())?;
        return Ok(Cmo3Export {
            cmo3: projects.cmo3.encode().map_err(|e| e.to_string())?,
            can3: projects.can3.encode().ok(),
        });
    }
    let moc3_data = std::fs::read(moc_path).map_err(|e| e.to_string())?;
    let mut tex = Vec::new();
    for p in texture_pngs(moc_path) {
        tex.push(moc2cmo::Texture::from_png(p).map_err(|e| format!("texture: {e}"))?);
    }
    let mut dec = dec;
    dec.set_textures(tex);
    dec.decompile_project(&moc3_data)
        .and_then(|p| p.encode())
        .map(|cmo3| Cmo3Export { cmo3, can3: None })
        .map_err(|e| e.to_string())
}

/// Build a Cubism `.cmo3` project object (unencoded) for inspection tools.
pub fn cmo3_project(moc: &crate::moc3::Moc3, moc3_data: &[u8], pngs: &[Vec<u8>], force_visible: bool) -> Result<moc2cmo::cmo3::Cmo3Project, String> {
    let mut dec = moc2cmo::Decompiler::new();
    let mut tex = Vec::new();
    for p in pngs {
        tex.push(moc2cmo::Texture::from_png(p.clone()).map_err(|e| format!("texture: {e}"))?);
    }
    dec.set_textures(tex);
    dec.set_force_visible(force_visible);
    let mut ranks = vec![0i32; moc.art_meshes.len()];
    for (pos, m) in crate::model::Model::new(moc).render_order().into_iter().enumerate() {
        ranks[m] = pos as i32;
    }
    dec.set_draw_ranks(ranks);
    dec.set_deformer_parts(moc.deformers.iter().map(|d| (d.parent_part >= 0).then_some(d.parent_part as usize)).collect());
    dec.decompile_project(moc3_data).map_err(|e| e.to_string())
}

/// Encode a Cubism `.cmo3` without a sibling model3.json (legacy callers).
pub fn write_cmo3(moc: &crate::moc3::Moc3, moc3_data: &[u8], pngs: &[Vec<u8>], force_visible: bool) -> Result<Vec<u8>, String> {
    cmo3_project(moc, moc3_data, pngs, force_visible)?.encode().map_err(|e| e.to_string())
}
