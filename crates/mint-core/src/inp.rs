//! Inochi2D .inp/.inx emitter: transpile MOC3 model -> Inochi2D 0.8.x puppet.
//!
//! Layout follows the v0.8.6 (de)serializers:
//! - part groups are plain `Node`s (organisational only) so every `Part`
//!   is z-sorted globally, exactly like the moc3 draw order;
//! - each art mesh is a `Part` with its OWN native-resolution texture crop;
//! - clipping masks become `masks` [{source, mode}];
//! - every moc3 parameter becomes an Inochi parameter whose bindings are
//!   baked by evaluating the full deformer chain at each key value
//!   (`deform` for vertex motion, `opacity` for visibility changes).

use crate::model::{Model, Vec2};
use crate::moc3::Moc3;
use image::RgbaImage;
use serde_json::{json, Value};
use std::io::Write;

/// Per-mesh texture crop. left/top/w/h are in NORMALIZED atlas UV space.
struct MeshCrop {
    left: f32,
    top: f32,
    w: f32,
    h: f32,
    png: Vec<u8>,
}

/// Crop the source atlas at each mesh's UV bbox (native resolution).
fn mesh_textures(moc: &Moc3, textures: &[RgbaImage]) -> Vec<Option<MeshCrop>> {
    let mut out = Vec::with_capacity(moc.art_meshes.len());
    for am in &moc.art_meshes {
        let vc = am.vertex_count.max(0) as usize;
        if vc == 0 || textures.is_empty() {
            out.push(None);
            continue;
        }
        let tex = &textures[(am.texture_no as usize).min(textures.len() - 1)];
        let (tw, th) = (tex.width() as f32, tex.height() as f32);
        let (mut minu, mut minv, mut maxu, mut maxv) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let ub = am.uv_begin.max(0) as usize / 2;
        for v in 0..vc {
            let uv = moc.uvs[(ub + v).min(moc.uvs.len() - 1)];
            minu = minu.min(uv[0]);
            maxu = maxu.max(uv[0]);
            minv = minv.min(uv[1]);
            maxv = maxv.max(uv[1]);
        }
        let x0 = (minu * tw).floor().max(0.0).min(tw - 1.0);
        let y0 = (minv * th).floor().max(0.0).min(th - 1.0);
        let x1 = (maxu * tw).ceil().min(tw).max(x0 + 1.0);
        let y1 = (maxv * th).ceil().min(th).max(y0 + 1.0);
        let mut crop = image::imageops::crop_imm(tex, x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32).to_image();
        // Inochi blends with ONE/ONE_MINUS_SRC_ALPHA and never premultiplies:
        // hand it premultiplied pixels with clean (black) transparent areas.
        for px in crop.pixels_mut() {
            let a = px.0[3] as u32;
            for c in 0..3 {
                px.0[c] = ((px.0[c] as u32 * a + 127) / 255) as u8;
            }
        }
        let mut png = Vec::new();
        if crop.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
            out.push(None);
            continue;
        }
        out.push(Some(MeshCrop { left: x0 / tw, top: y0 / th, w: (x1 - x0) / tw, h: (y1 - y0) / th, png }));
    }
    out
}

/// Write an Inochi2D package (.inp/.inx share the TRNSRTS container).
pub fn write_inp<W: Write>(moc: &Moc3, model: &mut Model, textures: &[RgbaImage], out: W) -> std::io::Result<()> {
    write_inp_ext(moc, model, textures, &[], out)
}

/// Raw JSON sidecars next to the moc3 (physics, expressions, display info,
/// VTube Studio settings) to carry along as INP vendor data.
pub fn sidecars(moc_path: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let dir = moc_path.parent().unwrap_or(std::path::Path::new("."));
    let stem = moc_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let mut out = Vec::new();
    for (name, suffix) in [("physics3", "physics3.json"), ("cdi3", "cdi3.json"), ("model3", "model3.json"), ("vtube", "vtube.json")] {
        if let Ok(b) = std::fs::read(dir.join(format!("{stem}.{suffix}"))) {
            out.push((format!("MINT.{name}"), b));
        }
    }
    if let Ok(b) = std::fs::read(crate::Overrides::path_for(moc_path)) {
        out.push(("MINT.overrides".to_string(), b));
    }
    let mut exps = serde_json::Map::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if let Some(base) = n.strip_suffix(".exp3.json") {
                if let Some(v) = std::fs::read(e.path()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
                    exps.insert(base.to_string(), v);
                }
            }
        }
    }
    if !exps.is_empty() {
        out.push(("MINT.expressions".into(), serde_json::to_vec(&Value::Object(exps)).unwrap_or_default()));
    }
    out
}

/// Optional rig metadata recovered from the sidecar files.
#[derive(Default)]
struct Extras {
    physics: Option<Value>,
    part_names: std::collections::HashMap<String, String>,
    param_names: std::collections::HashMap<String, String>,
    overrides: crate::Overrides,
}

fn parse_extras(ext: &[(String, Vec<u8>)]) -> Extras {
    let mut x = Extras::default();
    let find = |key: &str| ext.iter().find(|(n, _)| n == key).and_then(|(_, b)| serde_json::from_slice::<Value>(b).ok());
    x.physics = find("MINT.physics3");
    if let Some(cdi) = find("MINT.cdi3") {
        for (key, map) in [("Parts", &mut x.part_names), ("Parameters", &mut x.param_names)] {
            for e in cdi[key].as_array().into_iter().flatten() {
                if let (Some(id), Some(name)) = (e["Id"].as_str(), e["Name"].as_str()) {
                    if !name.is_empty() {
                        map.insert(id.to_string(), name.to_string());
                    }
                }
            }
        }
    }
    if let Some(ov) = ext.iter().find(|(n, _)| n == "MINT.overrides").and_then(|(_, b)| crate::Overrides::from_bytes(b)) {
        for (k, v) in &ov.part_names {
            x.part_names.insert(k.clone(), v.clone());
        }
        for (k, v) in &ov.param_names {
            x.param_names.insert(k.clone(), v.clone());
        }
        x.overrides = ov;
    }
    x
}

pub fn write_inp_ext<W: Write>(
    moc: &Moc3,
    model: &mut Model,
    textures: &[RgbaImage],
    ext: &[(String, Vec<u8>)],
    mut out: W,
) -> std::io::Result<()> {
    let crops = mesh_textures(moc, textures);
    let puppet = build_puppet(moc, model, &crops, &parse_extras(ext));
    let json_bytes = serde_json::to_vec(&puppet).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    out.write_all(b"TRNSRTS\0")?;
    out.write_all(&(json_bytes.len() as u32).to_be_bytes())?;
    out.write_all(&json_bytes)?;

    // TEX_SECT: one texture per art mesh, slot index == mesh index.
    out.write_all(b"TEX_SECT")?;
    out.write_all(&(crops.len() as u32).to_be_bytes())?;
    for c in &crops {
        match c {
            Some(c) => {
                out.write_all(&(c.png.len() as u32).to_be_bytes())?;
                out.write_all(&[0u8])?; // 0 = PNG
                out.write_all(&c.png)?;
            }
            None => {
                out.write_all(&0u32.to_be_bytes())?;
                out.write_all(&[0u8])?;
            }
        }
    }
    if !ext.is_empty() {
        out.write_all(b"EXT_SECT")?;
        out.write_all(&(ext.len() as u32).to_be_bytes())?;
        for (name, data) in ext {
            out.write_all(&(name.len() as u32).to_be_bytes())?;
            out.write_all(name.as_bytes())?;
            out.write_all(&(data.len() as u32).to_be_bytes())?;
            out.write_all(data)?;
        }
    }
    Ok(())
}

/// canvas px -> inochi2d space (centered, Y-down like the canvas).
fn to_i2d(v: Vec2, w: f32, h: f32) -> [f32; 2] {
    [v.x - w / 2.0, v.y - h / 2.0]
}

/// Quantize to 0.1 px to keep the binding payload small.
fn q(x: f32) -> Value {
    let r = ((x as f64) * 10.0).round() / 10.0;
    if r == 0.0 { json!(0) } else { json!(r) }
}

/// Binding source of a deformer's own keyforms (type-specific table).
pub fn deformer_kbs(moc: &Moc3, d: &crate::moc3::Deformer) -> i32 {
    use crate::moc3::DeformerType;
    let i = d.specific_index.max(0) as usize;
    match d.ty {
        DeformerType::Warp => moc.warp_deformers.get(i).map_or(-1, |w| w.keyform_binding_source),
        DeformerType::Rotation => moc.rotation_deformers.get(i).map_or(-1, |r| r.keyform_binding_source),
        _ => -1,
    }
}

/// (parameter index, key range) pairs an object is bound to.
pub fn obj_bindings(moc: &Moc3, kbs: i32) -> Vec<(usize, std::ops::Range<usize>)> {
    if kbs < 0 {
        return Vec::new();
    }
    let kfb = &moc.keyform_bindings[kbs as usize];
    let b = kfb.parameter_binding_index_begin.max(0) as usize;
    let e = (b + kfb.parameter_binding_index_count.max(0) as usize).min(moc.parameter_binding_indices.len());
    let mut pb_to_param = vec![0usize; moc.parameter_bindings.len()];
    for (p, prm) in moc.parameters.iter().enumerate() {
        let b0 = prm.binding_sources_begin.max(0) as usize;
        let e0 = (b0 + prm.binding_sources_count.max(0) as usize).min(pb_to_param.len());
        for j in b0..e0 {
            pb_to_param[j] = p;
        }
    }
    moc.parameter_binding_indices[b..e]
        .iter()
        .map(|&pbi| {
            let pb = &moc.parameter_bindings[pbi.max(0) as usize];
            (
                pb_to_param[pbi.max(0) as usize],
                pb.keys_begin.max(0) as usize..(pb.keys_begin.max(0) + pb.keys_count.max(0)) as usize,
            )
        })
        .collect()
}

type Eval = Vec<(Vec<Vec2>, f32)>;

/// Evaluate every mesh (canvas px verts, opacity) at the model's current params.
fn eval_all(model: &mut Model, n: usize) -> Eval {
    (0..n)
        .map(|mi| {
            let (v, o, _d) = model.art_mesh(mi);
            let px: Vec<Vec2> = v.into_iter().map(|x| model.canvas_to_px(x)).collect();
            (px, o)
        })
        .collect()
}

struct Ctx<'a> {
    moc: &'a Moc3,
    pc: Vec<Vec<usize>>,
    pm: Vec<Vec<usize>>,
    rp: Vec<usize>,
    base: Vec<Vec<Vec2>>,
    node_op: Vec<f32>,
    tints: Vec<([f32; 3], [f32; 3])>,
    crops: &'a [Option<MeshCrop>],
    hidden: std::collections::HashSet<String>,
    ov: &'a crate::Overrides,
    part_names: &'a std::collections::HashMap<String, String>,
    cw: f32,
    ch: f32,
}

const NODE_T: &str = r#"{"trans":[0.0,0.0,0.0],"rot":[0.0,0.0,0.0],"scale":[1.0,1.0]}"#;

fn transform() -> Value {
    serde_json::from_str(NODE_T).unwrap()
}

impl<'a> Ctx<'a> {
    fn mesh_uuid(&self, i: usize) -> u64 {
        1000 + i as u64
    }

    /// Earliest draw position among a part's descendant meshes.
    fn pos(&self, p: usize) -> usize {
        let a = self.pm[p].iter().map(|&m| self.rp[m]).min().unwrap_or(usize::MAX);
        let b = self.pc[p].iter().map(|&s| self.pos(s)).min().unwrap_or(usize::MAX);
        a.min(b)
    }

    /// Children in moc3 draw order (parts and meshes interleaved).
    fn children(&self, parts: &[usize], meshes: &[usize]) -> Vec<Value> {
        let mut items: Vec<(usize, bool, usize)> = Vec::new();
        for &p in parts {
            let pos = self.pos(p);
            if pos != usize::MAX {
                items.push((pos, true, p));
            }
        }
        for &m in meshes {
            items.push((self.rp[m], false, m));
        }
        items.sort_by_key(|&(pos, _, _)| pos);
        items.into_iter().map(|(_, is_part, i)| if is_part { self.part(i) } else { self.mesh(i) }).collect()
    }

    fn part(&self, p: usize) -> Value {
        json!({
            "type": "Node",
            "uuid": 100 + p as u64,
            "name": self.part_names.get(&self.moc.parts[p].id).cloned().unwrap_or_else(|| self.moc.parts[p].id.clone()),
            "enabled": !self.hidden.contains(self.moc.parts[p].id.as_str()),
            "zsort": 0.0,
            "transform": transform(),
            "lockToRoot": false,
            "children": self.children(&self.pc[p], &self.pm[p]),
        })
    }

    fn mesh(&self, mi: usize) -> Value {
        let moc = self.moc;
        let am = &moc.art_meshes[mi];
        let mut verts: Vec<f32> = Vec::with_capacity(self.base[mi].len() * 2);
        for v in &self.base[mi] {
            let p = to_i2d(*v, self.cw, self.ch);
            verts.push(p[0]);
            verts.push(p[1]);
        }
        let crop = &self.crops[mi];
        let vc = am.vertex_count.max(0) as usize;
        let ub = am.uv_begin.max(0) as usize / 2;
        let mut uvs: Vec<f32> = Vec::with_capacity(vc * 2);
        if let Some(c) = crop {
            for v in 0..vc {
                let uv = moc.uvs.get(ub + v).copied().unwrap_or([0.0, 0.0]);
                uvs.push((uv[0] - c.left) / c.w);
                uvs.push((uv[1] - c.top) / c.h);
            }
        }
        let pb = am.position_index_begin.max(0) as usize;
        let indices: Vec<u32> = (0..am.position_index_count.max(0) as usize)
            .map(|i| moc.position_indices.get(pb + i).copied().unwrap_or(0) as u32)
            .collect();
        let mb = am.mask_begin.max(0) as usize;
        let me = (mb + am.mask_count.max(0) as usize).min(moc.drawable_masks.len());
        let mode = if am.inverted_mask { "DodgeMask" } else { "Mask" };
        let masks: Vec<Value> = moc.drawable_masks[mb..me]
            .iter()
            .map(|&m| json!({ "source": self.mesh_uuid(m.max(0) as usize), "mode": mode }))
            .collect();
        use crate::raster::Blend;
        let b = if am.blend_v6 >= 0 { Blend::from_v6((am.blend_v6 & 0xFF) as u8) } else { Blend::from_mode(am.blend_mode) };
        let blend = match b {
            Blend::Normal | Blend::LinearLight => "Normal",
            Blend::Add => "LinearDodge",
            Blend::Multiply | Blend::MultiplyNew | Blend::Darken | Blend::LinearBurn => "Multiply",
            Blend::ColorBurn => "ColorBurn",
            Blend::Lighten => "Lighten",
            Blend::Screen => "Screen",
            Blend::ColorDodge => "ColorDodge",
            Blend::Overlay => "Overlay",
            Blend::SoftLight => "SoftLight",
            Blend::HardLight => "HardLight",
        };
        json!({
            "type": "Part",
            "uuid": self.mesh_uuid(mi),
            "name": if am.id.is_empty() { format!("ArtMesh{}", mi) } else { self.ov.mesh_name(&am.id).to_string() },
            "enabled": !(self.ov.apply_visibility && self.ov.hidden_meshes.contains(am.id.as_str())),
            "zsort": -(self.rp[mi] as f32),
            "transform": transform(),
            "lockToRoot": false,
            "children": [],
            "mesh": { "verts": verts, "uvs": uvs, "indices": indices, "origin": [0.0, 0.0] },
            "textures": if crop.is_some() { vec![mi as u64, u32::MAX as u64, u32::MAX as u64] } else { vec![] },
            "opacity": self.node_op[mi],
            "blend_mode": blend,
            "tint": self.tints[mi].0,
            "screenTint": self.tints[mi].1,
            "mask_threshold": 0.5,
            "masks": masks,
        })
    }
}

fn build_puppet(moc: &Moc3, model: &mut Model, crops: &[Option<MeshCrop>], extras: &Extras) -> Value {
    let (cw, ch) = (moc.canvas.width, moc.canvas.height);
    let n_mesh = moc.art_meshes.len();
    let defaults: Vec<f32> = moc.parameters.iter().map(|p| p.default).collect();

    model.params = defaults.clone();
    model.invalidate();
    let base = eval_all(model, n_mesh);
    let base_cols: Vec<([f32; 3], [f32; 3])> = (0..n_mesh).map(|mi| model.mesh_colors(mi)).collect();
    let mut tint_driven = vec![false; n_mesh];
    let mut screen_driven = vec![false; n_mesh];
    let order = model.render_order();
    let mut rp = vec![usize::MAX; n_mesh];
    for (i, &m) in order.iter().enumerate() {
        rp[m] = i;
    }

    // ---- parameters: single sliders, plus X/Y pairs merged into 2D params ----
    let pairs = match extras.overrides.pair_indices(moc) {
        Some(p) => p,
        None => find_pairs(moc),
    };
    let mut axis_target: std::collections::HashMap<usize, (u64, u8)> = std::collections::HashMap::new();
    for &(px, py) in &pairs {
        axis_target.insert(px, (5000 + px as u64, 0));
        axis_target.insert(py, (5000 + px as u64, 1));
    }
    let (phys_nodes, phys_inputs) = physics_nodes(moc, extras.physics.as_ref(), &axis_target);

    let mut params_out = Vec::new();
    let mut opacity_driven = vec![false; n_mesh];
    let in_pair: std::collections::HashSet<usize> = pairs.iter().flat_map(|&(a, b)| [a, b]).collect();
    let mut units: Vec<Vec<usize>> = Vec::new();
    for pi in 0..moc.parameters.len() {
        if let Some(&(px, py)) = pairs.iter().find(|&&(a, _)| a == pi) {
            units.push(vec![px, py]);
        } else if !in_pair.contains(&pi) {
            units.push(vec![pi]);
        }
    }
    for unit in units {
        let axes: Vec<Axis> = unit.iter().map(|&pi| make_axis(moc, pi)).collect();
        let active = axes.iter().all(|a| a.span > 1e-6 && a.kv.len() >= 2);
        let is2d = axes.len() == 2;
        let nx = axes[0].kv.len();
        let ny = if is2d { axes[1].kv.len() } else { 1 };
        let ny_out = if is2d { ny } else { 2 };
        let mut bindings: Vec<Value> = Vec::new();

        if active {
            let mut evs: Vec<Vec<(Eval, Vec<usize>, Vec<([f32; 3], [f32; 3])>)>> = Vec::new();
            for i in 0..nx {
                let mut row = Vec::new();
                for j in 0..ny {
                    model.params = defaults.clone();
                    model.params[axes[0].pi] = axes[0].kv[i];
                    if is2d {
                        model.params[axes[1].pi] = axes[1].kv[j];
                    }
                    model.invalidate();
                    let e = eval_all(model, n_mesh);
                    let mut r = vec![usize::MAX; n_mesh];
                    for (k, m) in model.render_order().into_iter().enumerate() {
                        r[m] = k;
                    }
                    let cols = (0..n_mesh).map(|mi| model.mesh_colors(mi)).collect();
                    row.push((e, r, cols));
                }
                evs.push(row);
            }
            let cell = |i: usize, j: usize| &evs[i][j.min(ny - 1)];
            let grid = |f: &dyn Fn(usize, usize) -> Value| -> Value { Value::Array((0..nx).map(|i| Value::Array((0..ny_out).map(|j| f(i, j)).collect())).collect()) };
            let is_set = || vec![vec![true; ny_out]; nx];
            let all_cells = || (0..nx).flat_map(|i| (0..ny).map(move |j| (i, j)));

            for mi in 0..n_mesh {
                if crops[mi].is_none() {
                    continue;
                }
                let moved = all_cells().any(|(i, j)| {
                    cell(i, j).0[mi].0.iter().zip(&base[mi].0).any(|(a, b)| (a.x - b.x).abs() > 0.05 || (a.y - b.y).abs() > 0.05)
                });
                let reorders = all_cells().any(|(i, j)| cell(i, j).1[mi] != rp[mi]);
                let fades = all_cells().any(|(i, j)| (cell(i, j).0[mi].1 - base[mi].1).abs() > 1e-3);
                let node = 1000 + mi as u64;
                let mut push = |name: String, values: Value| {
                    bindings.push(json!({ "node": node, "param_name": name, "values": values, "isSet": is_set(), "interpolate_mode": "Linear" }));
                };
                if reorders {
                    push("zSort".into(), grid(&|i, j| json!(-(cell(i, j).1[mi] as f32 - rp[mi] as f32))));
                }
                if moved {
                    push("deform".into(), grid(&|i, j| {
                        let offs: Vec<Value> = cell(i, j).0[mi].0.iter().zip(&base[mi].0).map(|(a, b)| json!([q(a.x - b.x), q(a.y - b.y)])).collect();
                        Value::Array(offs)
                    }));
                }
                for (screen, label) in [(false, "tint"), (true, "screenTint")] {
                    let pick = |t: &([f32; 3], [f32; 3])| if screen { t.1 } else { t.0 };
                    let b = pick(&base_cols[mi]);
                    if !all_cells().any(|(i, j)| pick(&cell(i, j).2[mi]).iter().zip(&b).any(|(x, y)| (x - y).abs() > 0.002)) {
                        continue;
                    }
                    if screen { screen_driven[mi] = true } else { tint_driven[mi] = true }
                    for (ch, cn) in ["r", "g", "b"].iter().enumerate() {
                        push(format!("{label}.{cn}"), grid(&|i, j| json!(pick(&cell(i, j).2[mi])[ch])));
                    }
                }
                if fades {
                    opacity_driven[mi] = true;
                    push("opacity".into(), grid(&|i, j| json!(cell(i, j).0[mi].1.clamp(0.0, 1.0))));
                }
            }
            for (k, ax) in axes.iter().enumerate() {
                for (node, name, scale) in phys_inputs.get(&ax.pi).into_iter().flatten() {
                    let values = grid(&|i, j| json!(((if k == 0 { ax.kv[i] } else { ax.kv[j] }) - ax.def) * scale));
                    bindings.push(json!({ "node": node, "param_name": name, "values": values, "isSet": is_set(), "interpolate_mode": "Linear" }));
                }
            }
        }
        if axes.len() == 1 && axes[0].is_divider && bindings.is_empty() {
            continue;
        }

        let pname = |pi: usize| extras.param_names.get(&moc.parameters[pi].id).cloned().unwrap_or_else(|| moc.parameters[pi].id.clone());
        let ax_json = |a: &Axis| -> Vec<f32> { if a.span > 1e-6 && a.kv.len() >= 2 { a.kv.iter().map(|v| ((v - a.lo) / a.span).clamp(0.0, 1.0)).collect() } else { vec![0.0, 1.0] } };
        let hi_of = |a: &Axis| if a.span > 1e-6 { a.hi } else { a.lo + 1.0 };
        if is2d {
            params_out.push(json!({
                "uuid": 5000 + axes[0].pi as u64,
                "name": pair_name(&pname(axes[0].pi), &pname(axes[1].pi)),
                "is_vec2": true,
                "min": [axes[0].lo, axes[1].lo],
                "max": [hi_of(&axes[0]), hi_of(&axes[1])],
                "defaults": [axes[0].def, axes[1].def],
                "axis_points": [ax_json(&axes[0]), ax_json(&axes[1])],
                "merge_mode": "Additive",
                "bindings": bindings,
            }));
        } else {
            let a = &axes[0];
            params_out.push(json!({
                "uuid": 5000 + a.pi as u64,
                "name": pname(a.pi),
                "is_vec2": false,
                "min": [a.lo, 0.0],
                "max": [hi_of(a), 1.0],
                "defaults": [a.def, 0.0],
                "axis_points": [ax_json(a), [0.0, 1.0]],
                "merge_mode": "Additive",
                "bindings": bindings,
            }));
        }
    }
    model.params = defaults;
    model.invalidate();

    // ---- node tree (same part hierarchy as the PSD/ORA exports) ----
    let mut pc: Vec<Vec<usize>> = vec![Vec::new(); moc.parts.len()];
    let mut root_parts = Vec::new();
    for (i, p) in moc.parts.iter().enumerate() {
        if p.parent_part >= 0 && (p.parent_part as usize) < moc.parts.len() {
            pc[p.parent_part as usize].push(i);
        } else {
            root_parts.push(i);
        }
    }
    let mut pm: Vec<Vec<usize>> = vec![Vec::new(); moc.parts.len()];
    let mut root_meshes = Vec::new();
    for (i, a) in moc.art_meshes.iter().enumerate() {
        if a.parent_part >= 0 && (a.parent_part as usize) < moc.parts.len() {
            pm[a.parent_part as usize].push(i);
        } else {
            root_meshes.push(i);
        }
    }
    let node_op: Vec<f32> = (0..n_mesh).map(|i| if opacity_driven[i] { 1.0 } else { base[i].1.clamp(0.0, 1.0) }).collect();
    let tints: Vec<([f32; 3], [f32; 3])> = (0..n_mesh).map(|i| (if tint_driven[i] { [1.0; 3] } else { base_cols[i].0 }, if screen_driven[i] { [0.0; 3] } else { base_cols[i].1 })).collect();
    let ctx = Ctx { moc, tints, pc, pm, rp, base: base.into_iter().map(|b| b.0).collect(), node_op, crops, hidden: { let mut h = crate::emit::hidden_parts(moc, None); if extras.overrides.apply_visibility { h.extend(extras.overrides.hidden_parts.iter().cloned()); } h }, ov: &extras.overrides, part_names: &extras.part_names, cw, ch };
    let mut root_children = ctx.children(&root_parts, &root_meshes);
    root_children.extend(phys_nodes);
    let root = json!({
        "type": "Node",
        "uuid": 1u64,
        "name": "Root",
        "enabled": true,
        "zsort": 0.0,
        "transform": transform(),
        "lockToRoot": false,
        "children": root_children,
    });

    json!({
        "meta": {
            "name": "model",
            "version": "0.8.6",
            "authors": ["MINT"],
            "rigger": "MINT",
            "rigger_version": "0.1.0",
            "licenseInFile": false,
        },
        "physics": { "pixels_per_meter": 1000.0, "gravity": 9.8 },
        "nodes": root,
        "param": params_out,
        "animations": [],
        "automation": [],
    })
}


/// Approximate Live2D physics with Inochi2D `SimplePhysics`.
///
/// Each physics setting becomes an anchor `Node` whose transform is bound to
/// the setting's input parameters (X/Y -> translation, Angle -> rotation);
/// every output becomes a `SimplePhysics` child that swings from that anchor
/// and writes into the output parameter.
fn physics_nodes(moc: &Moc3, physics: Option<&Value>, targets: &std::collections::HashMap<usize, (u64, u8)>) -> (Vec<Value>, std::collections::HashMap<usize, Vec<(u64, &'static str, f32)>>) {
    use std::collections::HashMap;
    let mut nodes = Vec::new();
    let mut inputs: HashMap<usize, Vec<(u64, &'static str, f32)>> = HashMap::new();
    let Some(p) = physics else { return (nodes, inputs) };
    let pidx = |id: &str| moc.parameters.iter().position(|q| q.id == id);
    let dict: Vec<(String, String)> = p["Meta"]["PhysicsDictionary"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| (d["Id"].as_str().unwrap_or("").to_string(), d["Name"].as_str().unwrap_or("").to_string()))
        .collect();
    let num = |v: &Value, d: f32| v.as_f64().map_or(d, |x| x as f32);
    for (si, s) in p["PhysicsSettings"].as_array().into_iter().flatten().enumerate() {
        let id = s["Id"].as_str().unwrap_or("");
        let name = dict.iter().find(|(i, _)| i == id).map(|(_, n)| n.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| id.to_string());
        let anchor = 20000 + si as u64 * 100;
        let mut any_input = false;
        for i in s["Input"].as_array().into_iter().flatten() {
            let Some(pi) = pidx(i["Source"]["Id"].as_str().unwrap_or("")) else { continue };
            let prm = &moc.parameters[pi];
            let reach = (prm.max - prm.default).abs().max((prm.min - prm.default).abs()).max(1e-4);
            let w = num(&i["Weight"], 0.0) / 100.0;
            let sign = if i["Reflect"].as_bool().unwrap_or(false) { -1.0 } else { 1.0 };
            let (bind, scale) = match i["Type"].as_str().unwrap_or("") {
                "X" => ("transform.t.x", sign * 100.0 * w / reach),
                "Y" => ("transform.t.y", -sign * 100.0 * w / reach),
                _ => ("transform.r.z", sign * 10.0 * w / reach * std::f32::consts::PI / 180.0),
            };
            inputs.entry(pi).or_default().push((anchor, bind, scale));
            any_input = true;
        }
        let verts: Vec<[f32; 2]> = s["Vertices"].as_array().into_iter().flatten().map(|v| [num(&v["Position"]["X"], 0.0), num(&v["Position"]["Y"], 0.0)]).collect();
        let mut kids = Vec::new();
        for (oi, o) in s["Output"].as_array().into_iter().flatten().enumerate() {
            let Some(pi) = pidx(o["Destination"]["Id"].as_str().unwrap_or("")) else { continue };
            let prm = &moc.parameters[pi];
            let vi = o["VertexIndex"].as_u64().unwrap_or(1) as usize;
            let (a, b) = (verts.get(vi.saturating_sub(1)).copied().unwrap_or([0.0, 0.0]), verts.get(vi).copied().unwrap_or([0.0, 8.0]));
            let seg = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt().max(1.0);
            let v = s["Vertices"].as_array().and_then(|a| a.get(vi)).cloned().unwrap_or(Value::Null);
            let mobility = num(&v["Mobility"], 0.9);
            let delay = num(&v["Delay"], 0.8);
            let scale = num(&o["Scale"], 1.0);
            let sign = if o["Reflect"].as_bool().unwrap_or(false) { -1.0 } else { 1.0 };
            let (uuid, axis) = targets.get(&pi).copied().unwrap_or((5000 + pi as u64, 0));
            let (chan, s) = match o["Type"].as_str().unwrap_or("") {
                "X" => (1, sign * scale * seg),
                "Y" => (2, sign * scale * seg),
                _ => (0, sign * std::f32::consts::PI * scale),
            };
            let (mode, oscale) = match (chan, axis) {
                (0, 0) => ("AngleLength", [s, 0.0]),
                (0, _) => ("LengthAngle", [0.0, s]),
                (1, 0) => ("XY", [s, 0.0]),
                (1, _) => ("YX", [0.0, s]),
                (_, 0) => ("YX", [s, 0.0]),
                (_, _) => ("XY", [0.0, s]),
            };
            kids.push(json!({
                "type": "SimplePhysics",
                "uuid": anchor + 1 + oi as u64,
                "name": format!("{name} -> {}", prm.id),
                "enabled": true,
                "zsort": 0.0,
                "transform": transform(),
                "lockToRoot": false,
                "children": [],
                "param": uuid,
                "model_type": "Pendulum",
                "map_mode": mode,
                "gravity": 1.0,
                "length": seg * 10.0,
                "frequency": (0.8 / delay.max(0.2)).clamp(0.4, 3.0),
                "angle_damping": (0.3 + (1.0 - mobility) * 2.0).clamp(0.2, 1.0),
                "length_damping": 0.5,
                "output_scale": oscale,
                "local_only": false,
            }));
        }
        if any_input && !kids.is_empty() {
            nodes.push(json!({
                "type": "Node",
                "uuid": anchor,
                "name": format!("Physics: {name}"),
                "enabled": true,
                "zsort": 0.0,
                "transform": transform(),
                "lockToRoot": false,
                "children": kids,
            }));
        }
    }
    (nodes, inputs)
}

struct Axis {
    pi: usize,
    lo: f32,
    hi: f32,
    span: f32,
    def: f32,
    kv: Vec<f32>,
    is_divider: bool,
}

/// Key values (axis points) of one moc3 parameter.
fn make_axis(moc: &Moc3, pi: usize) -> Axis {
    let prm = &moc.parameters[pi];
    let (lo, hi) = (prm.min.min(prm.max), prm.min.max(prm.max));
    let def = prm.default.clamp(lo, hi);
    let mut kv: Vec<f32> = vec![lo, hi, def];
    let kbs_all = moc.art_meshes.iter().map(|a| a.keyform_binding_source).chain(moc.deformers.iter().map(|d| deformer_kbs(moc, d)));
    for kbs in kbs_all {
        for (bp, keys) in obj_bindings(moc, kbs) {
            if bp == pi {
                kv.extend(keys.filter_map(|k| moc.keys.get(k).copied()));
            }
        }
    }
    kv.retain(|v| v.is_finite());
    for v in kv.iter_mut() {
        *v = v.clamp(lo, hi);
    }
    kv.sort_by(|a, b| a.partial_cmp(b).unwrap());
    kv.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
    Axis { pi, lo, hi, span: hi - lo, def, kv, is_divider: prm.id.to_lowercase().starts_with("divider") }
}

/// Parameter pairs Cubism combines in its standard template (head angle,
/// eyeballs, mouth form + open). The moc3 does not record which pairs the
/// author combined, so only these standard, adjacent pairs are merged into 2D
/// Inochi parameters; nothing else is guessed.
/// Parameter pairs combined by default (Cubism's standard template, plus the
/// name-based guess when enabled).
pub fn find_pairs(moc: &Moc3) -> Vec<(usize, usize)> {
    const STANDARD: [(&str, &str); 3] = [
        ("ParamAngleX", "ParamAngleY"),
        ("ParamEyeBallX", "ParamEyeBallY"),
        ("ParamMouthForm", "ParamMouthOpenY"),
    ];
    let usable = |i: usize| make_axis(moc, i).kv.len() >= 2 && make_axis(moc, i).span > 1e-6;
    let mut out: Vec<(usize, usize)> = (0..moc.parameters.len().saturating_sub(1))
        .filter(|&a| STANDARD.iter().any(|&(x, y)| moc.parameters[a].id == x && moc.parameters[a + 1].id == y))
        .filter(|&a| usable(a) && usable(a + 1))
        .map(|a| (a, a + 1))
        .collect();
    if crate::guess_pairs() {
        let mut used: std::collections::HashSet<usize> = out.iter().flat_map(|&(a, b)| [a, b]).collect();
        for (a, p) in moc.parameters.iter().enumerate() {
            let Some(stem) = p.id.strip_suffix('X') else { continue };
            let Some(b) = moc.parameters.iter().position(|q| q.id == format!("{stem}Y")) else { continue };
            if !used.contains(&a) && !used.contains(&b) && usable(a) && usable(b) {
                used.insert(a);
                used.insert(b);
                out.push((a, b));
            }
        }
    }
    out
}

/// "Angle X" + "Angle Y" -> "Angle"; unrelated names -> "A / B".
fn pair_name(a: &str, b: &str) -> String {
    let common: String = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).map(|(x, _)| x).collect();
    let trimmed = common.trim_end_matches(|c: char| c.is_whitespace() || c == '_' || c == '-' || c == '/');
    if trimmed.chars().count() >= 2 { trimmed.to_string() } else { format!("{a} / {b}") }
}
