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

pub fn write_inp_ext<W: Write>(
    moc: &Moc3,
    model: &mut Model,
    textures: &[RgbaImage],
    ext: &[(String, Vec<u8>)],
    mut out: W,
) -> std::io::Result<()> {
    let crops = mesh_textures(moc, textures);
    let puppet = build_puppet(moc, model, &crops);
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
    crops: &'a [Option<MeshCrop>],
    hidden: std::collections::HashSet<String>,
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
            "name": self.moc.parts[p].id.clone(),
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
        let blend = match am.blend_mode {
            1 => "AddGlow",
            2 => "Multiply",
            _ => "Normal",
        };
        json!({
            "type": "Part",
            "uuid": self.mesh_uuid(mi),
            "name": if am.id.is_empty() { format!("ArtMesh{}", mi) } else { am.id.clone() },
            "enabled": true,
            "zsort": -(self.rp[mi] as f32),
            "transform": transform(),
            "lockToRoot": false,
            "children": [],
            "mesh": { "verts": verts, "uvs": uvs, "indices": indices, "origin": [0.0, 0.0] },
            "textures": if crop.is_some() { vec![mi as u64, u32::MAX as u64, u32::MAX as u64] } else { vec![] },
            "opacity": self.node_op[mi],
            "blend_mode": blend,
            "tint": [1.0, 1.0, 1.0],
            "screenTint": [0.0, 0.0, 0.0],
            "mask_threshold": 0.5,
            "masks": masks,
        })
    }
}

fn build_puppet(moc: &Moc3, model: &mut Model, crops: &[Option<MeshCrop>]) -> Value {
    let (cw, ch) = (moc.canvas.width, moc.canvas.height);
    let n_mesh = moc.art_meshes.len();
    let defaults: Vec<f32> = moc.parameters.iter().map(|p| p.default).collect();

    model.params = defaults.clone();
    model.invalidate();
    let base = eval_all(model, n_mesh);
    let order = model.render_order();
    let mut rp = vec![usize::MAX; n_mesh];
    for (i, &m) in order.iter().enumerate() {
        rp[m] = i;
    }

    // ---- parameters: one per moc3 parameter, bindings baked from the rig ----
    let mut params_out = Vec::new();
    let mut opacity_driven = vec![false; n_mesh];
    for (pi, prm) in moc.parameters.iter().enumerate() {
        let (lo, hi) = (prm.min.min(prm.max), prm.min.max(prm.max));
        let span = hi - lo;
        let mut kv: Vec<f32> = vec![lo, hi, prm.default.clamp(lo, hi)];
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

        let mut bindings: Vec<Value> = Vec::new();
        let mut axis: Vec<f32> = vec![0.0, 1.0];
        if span > 1e-6 && kv.len() >= 2 {
            axis = kv.iter().map(|v| ((v - lo) / span).clamp(0.0, 1.0)).collect();
            let evs: Vec<(Eval, Vec<usize>)> = kv
                .iter()
                .map(|&v| {
                    model.params = defaults.clone();
                    model.params[pi] = v;
                    model.invalidate();
                    let e = eval_all(model, n_mesh);
                    let mut r = vec![usize::MAX; n_mesh];
                    for (i, m) in model.render_order().into_iter().enumerate() {
                        r[m] = i;
                    }
                    (e, r)
                })
                .collect();
            for mi in 0..n_mesh {
                if crops[mi].is_none() {
                    continue;
                }
                let moved = evs.iter().any(|e| {
                    e.0[mi].0.iter().zip(&base[mi].0).any(|(a, b)| (a.x - b.x).abs() > 0.05 || (a.y - b.y).abs() > 0.05)
                });
                let reorders = evs.iter().any(|e| e.1[mi] != rp[mi]);
                if reorders {
                    let values: Vec<Value> = evs.iter().map(|e| { let z = -(e.1[mi] as f32 - rp[mi] as f32); json!([z, z]) }).collect();
                    bindings.push(json!({
                        "node": 1000 + mi as u64,
                        "param_name": "zSort",
                        "values": values,
                        "isSet": vec![vec![true, true]; kv.len()],
                        "interpolate_mode": "Linear",
                    }));
                }
                let fades = evs.iter().any(|e| (e.0[mi].1 - base[mi].1).abs() > 1e-3);
                if moved {
                    let values: Vec<Value> = evs
                        .iter()
                        .map(|e| {
                            let offs: Vec<Value> = e.0[mi].0.iter().zip(&base[mi].0).map(|(a, b)| json!([q(a.x - b.x), q(a.y - b.y)])).collect();
                            json!([offs.clone(), offs])
                        })
                        .collect();
                    bindings.push(json!({
                        "node": 1000 + mi as u64,
                        "param_name": "deform",
                        "values": values,
                        "isSet": vec![vec![true, true]; kv.len()],
                        "interpolate_mode": "Linear",
                    }));
                }
                if fades {
                    opacity_driven[mi] = true;
                    let values: Vec<Value> = evs.iter().map(|e| { let o = e.0[mi].1.clamp(0.0, 1.0); json!([o, o]) }).collect();
                    bindings.push(json!({
                        "node": 1000 + mi as u64,
                        "param_name": "opacity",
                        "values": values,
                        "isSet": vec![vec![true, true]; kv.len()],
                        "interpolate_mode": "Linear",
                    }));
                }
            }
        }
        params_out.push(json!({
            "uuid": 5000 + pi as u64,
            "name": prm.id.clone(),
            "is_vec2": false,
            "min": [lo, 0.0],
            "max": [if span > 1e-6 { hi } else { lo + 1.0 }, 1.0],
            "defaults": [prm.default.clamp(lo, hi), 0.0],
            "axis_points": [axis, [0.0, 1.0]],
            "merge_mode": "Additive",
            "bindings": bindings,
        }));
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
    let ctx = Ctx { moc, pc, pm, rp, base: base.into_iter().map(|b| b.0).collect(), node_op, crops, hidden: crate::emit::hidden_parts(moc, None), cw, ch };
    let root = json!({
        "type": "Node",
        "uuid": 1u64,
        "name": "Root",
        "enabled": true,
        "zsort": 0.0,
        "transform": transform(),
        "lockToRoot": false,
        "children": ctx.children(&root_parts, &root_meshes),
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
