//! Basic model editor: a live viewport (pan / zoom / pick), parameter sliders
//! and 2D pads, part and layer toggles, renaming, and mesh overlays. It only
//! edits what already exists in the model (names, combined pairs, default
//! visibility); the edits are stored in `<model>.mint.json` for the exporters.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Shape, Stroke, Vec2};
use image::RgbaImage;
use mint_core::raster::Blend;
use mint_core::{Model, Moc3, Overrides};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Params,
    Layers,
    View,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Overlay {
    Off,
    Selected,
    All,
}

/// What an exact render shows: a window of the canvas.
#[derive(Clone, Copy)]
struct Camera {
    /// Image pixels per canvas pixel.
    scale: f32,
    /// Canvas position of the image's top-left corner.
    ox: f32,
    oy: f32,
    w: u32,
    h: u32,
    /// Pose version (parameters / visibility) it was rendered for.
    version: u64,
}

impl Camera {
    fn same(&self, o: &Camera) -> bool {
        self.version == o.version
            && self.w == o.w
            && self.h == o.h
            && (self.scale - o.scale).abs() <= 1e-4 * self.scale
            && (self.ox - o.ox).abs() * self.scale < 0.5
            && (self.oy - o.oy).abs() * self.scale < 0.5
    }
}

struct ExactImg {
    tex: egui::TextureHandle,
    cam: Camera,
}

type ExactSlot = Arc<Mutex<Option<(Camera, RgbaImage)>>>;
/// Atlas pages at successively halved sizes, built once in the first render.
type Mips = Arc<Mutex<Option<Arc<Vec<Vec<RgbaImage>>>>>>;

struct MeshEval {
    verts: Vec<Pos2>,
    opacity: f32,
    mul: [f32; 3],
}

pub struct Editor {
    path: PathBuf,
    moc: &'static Moc3,
    model: Model<'static>,
    atlases: Arc<Vec<RgbaImage>>,
    tex: Vec<egui::TextureHandle>,
    uvs: Vec<Vec<Pos2>>,
    tris: Vec<Vec<u32>>,
    edges: Vec<Vec<[u32; 2]>>,
    double_sided: Vec<bool>,
    pub params: Vec<f32>,
    /// Combined parameters: runs of 2 or more adjacent parameters.
    groups: Vec<Vec<usize>>,
    ov: Overrides,
    default_hidden_parts: Vec<bool>,
    hidden_parts: Vec<bool>,
    hidden_meshes: Vec<bool>,
    cdi_parts: HashMap<String, String>,
    cdi_params: HashMap<String, String>,
    part_children: Vec<Vec<usize>>,
    part_meshes: Vec<Vec<usize>>,
    root_parts: Vec<usize>,
    root_meshes: Vec<usize>,
    eval: Vec<MeshEval>,
    order: Vec<usize>,
    dirty: bool,
    offset: Vec2,
    zoom: f32,
    fit: bool,
    wire: Overlay,
    points: Overlay,
    selected: Option<usize>,
    edit_names: bool,
    filter: String,
    tab: Tab,
    pub status: String,
    exact: bool,
    version: u64,
    exact_img: Option<ExactImg>,
    exact_slot: ExactSlot,
    exact_busy: Arc<AtomicBool>,
    mips: Mips,
    /// Atlas texels per canvas pixel (median over the layers), used to pick a mip level.
    density: f32,
}

fn read_names(path: &std::path::Path) -> (HashMap<String, String>, HashMap<String, String>) {
    let mut parts = HashMap::new();
    let mut params = HashMap::new();
    if let Some(v) = std::fs::read(path.with_extension("cdi3.json")).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) {
        for (key, map) in [("Parts", &mut parts), ("Parameters", &mut params)] {
            for e in v[key].as_array().into_iter().flatten() {
                if let (Some(id), Some(name)) = (e["Id"].as_str(), e["Name"].as_str()) {
                    if !name.is_empty() {
                        map.insert(id.to_string(), name.to_string());
                    }
                }
            }
        }
    }
    (parts, params)
}

fn edge(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    (p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x)
}

fn set_name(map: &mut HashMap<String, String>, id: &str, value: &str, base: &str) {
    let v = value.trim();
    if v.is_empty() || v == base {
        map.remove(id);
    } else {
        map.insert(id.to_string(), v.to_string());
    }
}

impl Editor {
    pub fn new(ctx: &egui::Context, path: PathBuf, moc: Moc3, atlases: Vec<RgbaImage>) -> Self {
        let moc: &'static Moc3 = Box::leak(Box::new(moc));
        let model = Model::new(moc);
        let ov = Overrides::load(&path).unwrap_or_default();
        let tex = atlases
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let img = egui::ColorImage::from_rgba_unmultiplied([a.width() as usize, a.height() as usize], a.as_raw());
                ctx.load_texture(format!("atlas{i}"), img, egui::TextureOptions::LINEAR)
            })
            .collect();
        let n = moc.art_meshes.len();
        let mut uvs = Vec::with_capacity(n);
        let mut tris = Vec::with_capacity(n);
        let mut edges = Vec::with_capacity(n);
        for am in &moc.art_meshes {
            let ub = am.uv_begin.max(0) as usize / 2;
            uvs.push((0..am.vertex_count.max(0) as usize).map(|v| {
                let uv = moc.uvs.get(ub + v).copied().unwrap_or([0.0, 0.0]);
                Pos2::new(uv[0], uv[1])
            }).collect::<Vec<_>>());
            let pb = am.position_index_begin.max(0) as usize;
            let t: Vec<u32> = (0..am.position_index_count.max(0) as usize)
                .map(|i| moc.position_indices.get(pb + i).copied().unwrap_or(0) as u32)
                .collect();
            let mut seen = HashSet::new();
            let mut e = Vec::new();
            for tri in t.chunks_exact(3) {
                for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                    let k = (a.min(b), a.max(b));
                    if seen.insert(k) {
                        e.push([k.0, k.1]);
                    }
                }
            }
            tris.push(t);
            edges.push(e);
        }
        let mut part_children = vec![Vec::new(); moc.parts.len()];
        let mut part_meshes = vec![Vec::new(); moc.parts.len()];
        let (mut root_parts, mut root_meshes) = (Vec::new(), Vec::new());
        for (i, p) in moc.parts.iter().enumerate() {
            match usize::try_from(p.parent_part).ok().filter(|&q| q < moc.parts.len()) {
                Some(q) => part_children[q].push(i),
                None => root_parts.push(i),
            }
        }
        for (i, a) in moc.art_meshes.iter().enumerate() {
            match usize::try_from(a.parent_part).ok().filter(|&q| q < moc.parts.len()) {
                Some(q) => part_meshes[q].push(i),
                None => root_meshes.push(i),
            }
        }
        let default_ids = mint_core::emit::hidden_parts(moc, Some(&path));
        let default_hidden_parts: Vec<bool> = moc
            .parts
            .iter()
            .zip(&moc.offscreen_out_parts)
            .map(|(p, out)| default_ids.contains(&p.id) && !*out)
            .collect();
        let (cdi_parts, cdi_params) = read_names(&path);
        let params: Vec<f32> = moc.parameters.iter().map(|p| p.default).collect();
        let mut ed = Editor {
            path,
            moc,
            model,
            atlases: Arc::new(atlases),
            tex,
            uvs,
            tris,
            edges,
            double_sided: moc.art_meshes.iter().map(|a| a.double_sided).collect(),
            params,
            groups: Vec::new(),
            ov,
            hidden_parts: default_hidden_parts.clone(),
            default_hidden_parts,
            hidden_meshes: vec![false; n],
            cdi_parts,
            cdi_params,
            part_children,
            part_meshes,
            root_parts,
            root_meshes,
            eval: Vec::new(),
            order: Vec::new(),
            dirty: true,
            offset: Vec2::ZERO,
            zoom: 1.0,
            fit: true,
            wire: Overlay::Selected,
            points: Overlay::Selected,
            selected: None,
            edit_names: false,
            filter: String::new(),
            tab: Tab::Params,
            status: String::new(),
            exact: true,
            version: 0,
            exact_img: None,
            exact_slot: Arc::new(Mutex::new(None)),
            exact_busy: Arc::new(AtomicBool::new(false)),
            mips: Arc::new(Mutex::new(None)),
            density: 0.0,
        };
        ed.load_state();
        ed
    }

    /// (Re)apply the saved edits to the working state.
    fn load_state(&mut self) {
        let base = match self.ov.pair_indices(self.moc) {
            Some(p) => p,
            None => mint_core::inp::find_pairs(self.moc),
        };
        self.groups = base.into_iter().map(|(a, b)| vec![a, b]).collect();
        for g in &self.ov.groups {
            let idx: Vec<usize> = g.iter().filter_map(|id| self.moc.parameters.iter().position(|p| &p.id == id)).collect();
            if idx.len() == g.len() && idx.len() >= 3 && idx.windows(2).all(|w| w[1] == w[0] + 1) {
                self.groups.retain(|x| !x.iter().any(|m| idx.contains(m)));
                self.groups.push(idx);
            }
        }
        self.groups.sort();
        self.hidden_parts = self.default_hidden_parts.clone();
        self.hidden_meshes = vec![false; self.moc.art_meshes.len()];
        if self.ov.apply_visibility {
            for (i, p) in self.moc.parts.iter().enumerate() {
                if self.ov.hidden_parts.contains(&p.id) {
                    self.hidden_parts[i] = true;
                }
            }
            for (i, m) in self.moc.art_meshes.iter().enumerate() {
                if self.ov.hidden_meshes.contains(&m.id) {
                    self.hidden_meshes[i] = true;
                }
            }
        }
        self.dirty = true;
    }

    /// Edits as they should be exported right now.
    pub fn overrides(&self) -> Overrides {
        let mut ov = self.ov.clone();
        let id = |i: usize| self.moc.parameters[i].id.clone();
        // Cubism / Inochi controls are two-axis: a group exports as its first two members.
        let ids: Vec<(String, String)> = self.groups.iter().map(|g| (id(g[0]), id(g[1]))).collect();
        ov.groups = self.groups.iter().filter(|g| g.len() > 2).map(|g| g.iter().map(|&i| id(i)).collect()).collect();
        let auto: Vec<(usize, usize)> = mint_core::inp::find_pairs(self.moc);
        let mut a: Vec<(usize, usize)> = self.groups.iter().map(|g| (g[0], g[1])).collect();
        let mut b = auto;
        a.sort();
        b.sort();
        ov.pairs = if a == b { None } else { Some(ids) };
        ov.hidden_parts = self.moc.parts.iter().zip(&self.hidden_parts).filter(|(_, h)| **h).map(|(p, _)| p.id.clone()).collect();
        ov.hidden_meshes = self.moc.art_meshes.iter().zip(&self.hidden_meshes).filter(|(_, h)| **h).map(|(m, _)| m.id.clone()).collect();
        ov
    }

    pub fn save(&mut self) {
        let ov = self.overrides();
        self.status = match ov.save(&self.path) {
            Ok(()) => format!("Saved {}", Overrides::path_for(&self.path).display()),
            Err(e) => format!("Could not save edits: {e}"),
        };
        self.ov = ov;
    }

    pub fn discard(&mut self) {
        self.ov = Overrides::load(&self.path).unwrap_or_default();
        self.params = self.moc.parameters.iter().map(|p| p.default).collect();
        self.load_state();
        self.status = "Reverted to the saved edits".into();
    }

    fn part_label(&self, i: usize) -> String {
        let id = &self.moc.parts[i].id;
        self.ov.part_names.get(id).or_else(|| self.cdi_parts.get(id)).cloned().unwrap_or_else(|| id.clone())
    }

    fn param_label(&self, i: usize) -> String {
        let id = &self.moc.parameters[i].id;
        self.ov.param_names.get(id).or_else(|| self.cdi_params.get(id)).cloned().unwrap_or_else(|| id.clone())
    }

    fn evaluate(&mut self) {
        let n = self.moc.art_meshes.len();
        self.model.params.copy_from_slice(&self.params);
        self.model.invalidate();
        self.model.set_hidden_parts(&self.hidden_parts);
        self.model.set_hidden_meshes(&self.hidden_meshes);
        let mut eval = Vec::with_capacity(n);
        for mi in 0..n {
            let (v, opacity, _) = self.model.art_mesh(mi);
            let verts = v.iter().map(|p| { let q = self.model.canvas_to_px(*p); Pos2::new(q.x, q.y) }).collect();
            let (mul, _) = self.model.mesh_colors(mi);
            eval.push(MeshEval { verts, opacity, mul });
        }
        self.eval = eval;
        self.order = self.model.render_order();
        self.dirty = false;
        self.version += 1;
    }

    fn poll_exact(&mut self, ctx: &egui::Context) {
        let done = self.exact_slot.lock().unwrap().take();
        if let Some((cam, img)) = done {
            let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
            let tex = ctx.load_texture("exact", ci, egui::TextureOptions::LINEAR);
            self.exact_img = Some(ExactImg { tex, cam });
        }
    }

    /// Atlas texels per canvas pixel at the current pose (median over layers).
    fn texel_density(&self) -> f32 {
        let mut d: Vec<f32> = Vec::new();
        for (mi, ev) in self.eval.iter().enumerate() {
            let (uv, atlas) = (&self.uvs[mi], &self.atlases[(self.moc.art_meshes[mi].texture_no as usize).min(self.atlases.len() - 1)]);
            if uv.is_empty() || ev.verts.len() != uv.len() {
                continue;
            }
            let ext = |f: &dyn Fn(&Pos2) -> f32, v: &[Pos2]| {
                let (lo, hi) = v.iter().fold((f32::MAX, f32::MIN), |a, p| (a.0.min(f(p)), a.1.max(f(p))));
                hi - lo
            };
            let (cw, uw) = (ext(&|p| p.x, &ev.verts), ext(&|p| p.x, uv) * atlas.width() as f32);
            if cw > 30.0 && uw > 8.0 {
                d.push(uw / cw);
            }
        }
        d.sort_by(|a, b| a.partial_cmp(b).unwrap());
        d.get(d.len() / 2).copied().unwrap_or(1.0).clamp(0.1, 4.0)
    }

    /// Keep an exact render of exactly what is on screen: masks, every blend
    /// mode, offscreen parts, at the screen's pixel density, only for the
    /// visible window, drawn in bands on all cores. Renders back to back while
    /// the pose or view keeps changing (the last completed image stays up).
    fn maybe_render(&mut self, ctx: &egui::Context, cam: Camera) {
        if !self.exact || self.exact_busy.load(Ordering::SeqCst) {
            return;
        }
        if self.exact_img.as_ref().is_some_and(|e| e.cam.same(&cam)) {
            return;
        }
        if self.density <= 0.0 {
            self.density = self.texel_density();
        }
        self.exact_busy.store(true, Ordering::SeqCst);
        let (moc, params, hp, hm, density) = (self.moc, self.params.clone(), self.hidden_parts.clone(), self.hidden_meshes.clone(), self.density);
        let (atlases, mips, slot, busy, ctx) = (self.atlases.clone(), self.mips.clone(), self.exact_slot.clone(), self.exact_busy.clone(), ctx.clone());
        std::thread::spawn(move || {
            let levels = mips
                .lock()
                .unwrap()
                .get_or_insert_with(|| {
                    Arc::new(
                        atlases
                            .iter()
                            .map(|a| {
                                let mut lv = vec![a.clone()];
                                while lv.len() < 7 && lv.last().unwrap().width().max(lv.last().unwrap().height()) > 64 {
                                    let l = lv.last().unwrap();
                                    lv.push(image::imageops::resize(l, (l.width() / 2).max(1), (l.height() / 2).max(1), image::imageops::FilterType::Triangle));
                                }
                                lv
                            })
                            .collect(),
                    )
                })
                .clone();
            // texels per output pixel = density / scale; pick the mip closest to 1:1
            let level = (density / cam.scale).max(1.0).log2().round() as usize;
            let textures: Vec<RgbaImage> = levels.iter().map(|lv| lv[level.min(lv.len() - 1)].clone()).collect();
            let mut model = Model::new(moc);
            model.px_scale = cam.scale;
            model.px_offset = (cam.ox * cam.scale, cam.oy * cam.scale);
            model.params.copy_from_slice(&params);
            model.invalidate();
            model.set_hidden_parts(&hp);
            model.set_hidden_meshes(&hm);
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16);
            let img = mint_core::render::render_view(&mut model, &textures, (cam.w, cam.h), threads);
            *slot.lock().unwrap() = Some((cam, img));
            busy.store(false, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }

    fn drawn(&self, mi: usize) -> bool {
        self.eval[mi].opacity > 0.003 && self.model.out_ancestor(mi).is_none()
    }

    fn canvas_size(&self) -> Vec2 {
        Vec2::new(self.moc.canvas.width.max(1.0), self.moc.canvas.height.max(1.0))
    }

    /// Topmost mesh with visible pixels under the canvas point.
    fn pick(&self, c: Pos2) -> Option<usize> {
        for &mi in self.order.iter().rev() {
            if !self.drawn(mi) {
                continue;
            }
            let v = &self.eval[mi].verts;
            let tex = self.moc.art_meshes[mi].texture_no as usize;
            let Some(atlas) = self.atlases.get(tex.min(self.atlases.len().saturating_sub(1))) else { continue };
            for t in self.tris[mi].chunks_exact(3) {
                let (a, b, cc) = (v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]);
                let area = edge(a, b, cc);
                if area.abs() < 1e-6 || (area < 0.0 && !self.double_sided[mi]) {
                    continue;
                }
                let (w0, w1, w2) = (edge(b, cc, c) / area, edge(cc, a, c) / area, edge(a, b, c) / area);
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let uv = &self.uvs[mi];
                let (u, vv) = (
                    uv[t[0] as usize].x * w0 + uv[t[1] as usize].x * w1 + uv[t[2] as usize].x * w2,
                    uv[t[0] as usize].y * w0 + uv[t[1] as usize].y * w1 + uv[t[2] as usize].y * w2,
                );
                let (px, py) = (((u * atlas.width() as f32) as u32).min(atlas.width() - 1), ((vv * atlas.height() as f32) as u32).min(atlas.height() - 1));
                if atlas.get_pixel(px, py).0[3] > 24 {
                    return Some(mi);
                }
            }
        }
        None
    }

    pub fn viewport(&mut self, ui: &mut egui::Ui) {
        if self.dirty {
            self.evaluate();
        }
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_gray(34));
        let cs = self.canvas_size();
        if self.fit {
            self.zoom = (rect.width() / cs.x).min(rect.height() / cs.y) * 0.94;
            self.offset = Vec2::ZERO;
            self.fit = false;
        }
        let center = rect.center();
        let half = cs / 2.0;
        let to_screen = |p: Pos2, off: Vec2, zoom: f32| center + off + (p.to_vec2() - half) * zoom;
        let to_canvas = |s: Pos2, off: Vec2, zoom: f32| ((s - center - off) / zoom + half).to_pos2();

        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                if let Some(h) = resp.hover_pos() {
                    let before = to_canvas(h, self.offset, self.zoom);
                    self.zoom = (self.zoom * (scroll * 0.0015).exp()).clamp(0.02, 40.0);
                    let after = to_screen(before, Vec2::ZERO, self.zoom);
                    self.offset = h - after;
                }
            }
        }
        if resp.dragged() {
            self.offset += resp.drag_delta();
        }
        if resp.clicked() {
            if let Some(h) = resp.interact_pointer_pos() {
                self.selected = self.pick(to_canvas(h, self.offset, self.zoom));
                if let Some(s) = self.selected {
                    self.tab = if self.tab == Tab::Params { Tab::Layers } else { self.tab };
                    self.status = format!("Selected {} ({})", self.ov.mesh_name(&self.moc.art_meshes[s].id), self.moc.art_meshes[s].id);
                }
            }
        }

        // canvas outline
        let (z, off) = (self.zoom, self.offset);
        let canvas_rect = Rect::from_two_pos(to_screen(Pos2::ZERO, off, z), to_screen(Pos2::new(cs.x, cs.y), off, z));
        painter.rect_filled(canvas_rect, 0.0, Color32::from_gray(52));
        painter.rect_stroke(canvas_rect, 0.0, Stroke::new(1.0, Color32::from_gray(110)), egui::StrokeKind::Inside);

        // Exact render of the visible window (with a margin so panning does not
        // show gaps). Until the first one is ready the fast meshes are drawn.
        self.poll_exact(ui.ctx());
        let ppp = ui.ctx().pixels_per_point();
        let region = rect.expand2(rect.size() * 0.08);
        let origin = to_canvas(region.left_top(), off, z);
        let (mut scale, mut w, mut h) = (z * ppp, region.width() * ppp, region.height() * ppp);
        if w * h > 3.5e6 {
            let f = (3.5e6 / (w * h)).sqrt();
            scale *= f;
            w *= f;
            h *= f;
        }
        let cam = Camera { scale, ox: origin.x, oy: origin.y, w: w.max(1.0) as u32, h: h.max(1.0) as u32, version: self.version };
        self.maybe_render(ui.ctx(), cam);
        let exact_now = self.exact && self.exact_img.is_some();
        if exact_now {
            if let Some(e) = &self.exact_img {
                let c = e.cam;
                let img_rect = Rect::from_min_max(
                    to_screen(Pos2::new(c.ox, c.oy), off, z),
                    to_screen(Pos2::new(c.ox + c.w as f32 / c.scale, c.oy + c.h as f32 / c.scale), off, z),
                );
                painter.image(e.tex.id(), img_rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
        }

        // art meshes in draw order
        for &mi in &self.order {
            if exact_now || !self.drawn(mi) {
                continue;
            }
            let am = &self.moc.art_meshes[mi];
            let ev = &self.eval[mi];
            let tex = &self.tex[(am.texture_no as usize).min(self.tex.len().saturating_sub(1))];
            let mut mesh = egui::Mesh::with_texture(tex.id());
            let op = ev.opacity.clamp(0.0, 1.0);
            let add = if am.blend_v6 >= 0 { Blend::from_v6((am.blend_v6 & 0xFF) as u8) } else { Blend::from_mode(am.blend_mode) } == Blend::Add;
            let c = |m: f32| (m * op * 255.0).clamp(0.0, 255.0) as u8;
            let color = Color32::from_rgba_premultiplied(c(ev.mul[0]), c(ev.mul[1]), c(ev.mul[2]), if add { 0 } else { (op * 255.0) as u8 });
            for (i, v) in ev.verts.iter().enumerate() {
                mesh.vertices.push(egui::epaint::Vertex { pos: to_screen(*v, off, z), uv: self.uvs[mi][i], color });
            }
            for t in self.tris[mi].chunks_exact(3) {
                let area = edge(ev.verts[t[0] as usize], ev.verts[t[1] as usize], ev.verts[t[2] as usize]);
                if area.abs() < 1e-6 || (area < 0.0 && !self.double_sided[mi]) {
                    continue;
                }
                mesh.indices.extend_from_slice(t);
            }
            painter.add(Shape::mesh(mesh));
        }

        // overlays
        let draw_wire = |mi: usize, stroke: Stroke| {
            let v = &self.eval[mi].verts;
            let segs: Vec<Shape> = self.edges[mi].iter().map(|e| Shape::line_segment([to_screen(v[e[0] as usize], off, z), to_screen(v[e[1] as usize], off, z)], stroke)).collect();
            painter.extend(segs);
        };
        let draw_points = |mi: usize, size: f32, color: Color32| {
            let v = &self.eval[mi].verts;
            let shapes: Vec<Shape> = v.iter().map(|p| Shape::rect_filled(Rect::from_center_size(to_screen(*p, off, z), Vec2::splat(size)), 0.0, color)).collect();
            painter.extend(shapes);
        };
        let all: Vec<usize> = self.order.iter().copied().filter(|&m| self.drawn(m)).collect();
        match self.wire {
            Overlay::All => all.iter().for_each(|&m| draw_wire(m, Stroke::new(0.6, Color32::from_rgba_unmultiplied(120, 220, 255, 90)))),
            Overlay::Selected | Overlay::Off => {}
        }
        match self.points {
            Overlay::All => all.iter().for_each(|&m| draw_points(m, 2.5, Color32::from_rgb(255, 200, 60))),
            Overlay::Selected | Overlay::Off => {}
        }
        if let Some(s) = self.selected.filter(|&s| s < self.eval.len()) {
            if self.wire != Overlay::Off {
                draw_wire(s, Stroke::new(1.2, Color32::from_rgb(255, 235, 90)));
            }
            if self.points != Overlay::Off {
                draw_points(s, 5.0, Color32::from_rgb(255, 90, 90));
                if let Some(h) = resp.hover_pos() {
                    let near = self.eval[s].verts.iter().enumerate().map(|(i, p)| (i, to_screen(*p, off, z))).filter(|(_, sp)| sp.distance(h) < 9.0).min_by(|a, b| a.1.distance(h).partial_cmp(&b.1.distance(h)).unwrap());
                    if let Some((i, sp)) = near {
                        let p = self.eval[s].verts[i];
                        painter.text(sp + Vec2::new(8.0, -8.0), egui::Align2::LEFT_BOTTOM, format!("v{i}  ({:.1}, {:.1})", p.x, p.y), egui::FontId::monospace(11.0), Color32::WHITE);
                    }
                }
            }
        }
        let state = if !self.exact {
            "Fast preview (masks and most blend modes off)"
        } else if exact_now {
            if self.exact_busy.load(Ordering::SeqCst) { "Exact render (updating)" } else { "Exact render" }
        } else {
            "Preparing the exact render..."
        };
        painter.text(rect.left_top() + Vec2::new(8.0, 6.0), egui::Align2::LEFT_TOP, format!("{state}   |   scroll = zoom, drag = pan, click = select layer"), egui::FontId::proportional(11.0), Color32::from_gray(150));
    }

    pub fn side_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Params, "Parameters");
            ui.selectable_value(&mut self.tab, Tab::Layers, "Layers");
            ui.selectable_value(&mut self.tab, Tab::View, "View");
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.edit_names, "Edit names");
            if ui.button("Save edits").clicked() {
                self.save();
            }
            if ui.button("Revert").clicked() {
                self.discard();
            }
        });
        if !self.status.is_empty() {
            ui.label(egui::RichText::new(&self.status).weak().small());
        }
        ui.separator();
        match self.tab {
            Tab::Params => self.params_tab(ui),
            Tab::Layers => self.layers_tab(ui),
            Tab::View => self.view_tab(ui),
        }
    }

    fn view_tab(&mut self, ui: &mut egui::Ui) {
        for (label, which) in [("Wireframe", 0), ("Mesh points", 1)] {
            ui.horizontal(|ui| {
                ui.label(label);
                let cur = if which == 0 { &mut self.wire } else { &mut self.points };
                for (name, v) in [("Off", Overlay::Off), ("Selected", Overlay::Selected), ("All", Overlay::All)] {
                    ui.selectable_value(cur, v, name);
                }
            });
        }
        ui.checkbox(&mut self.exact, "Exact render (masks, blend modes, offscreen parts)")
            .on_hover_text("Always shows the real render (masks, blend modes, offscreen parts) at screen resolution. Off = fast mesh preview only.");
        ui.horizontal(|ui| {
            if ui.button("Fit view").clicked() {
                self.fit = true;
            }
            if ui.button("Reset parameters").clicked() {
                self.params = self.moc.parameters.iter().map(|p| p.default).collect();
                self.dirty = true;
            }
        });
        ui.separator();
        ui.checkbox(&mut self.ov.apply_visibility, "Use the layer / part toggles as the default visibility in exports")
            .on_hover_text("Off: toggles only change what you see here. On: hidden parts and layers start hidden in PSD, ORA, Inochi and Cubism files.");
        if let Some(s) = self.selected {
            let am = &self.moc.art_meshes[s];
            ui.separator();
            ui.label(egui::RichText::new("Selected layer").strong());
            ui.label(format!("id: {}", am.id));
            ui.label(format!("vertices: {}   triangles: {}", am.vertex_count, am.position_index_count / 3));
            ui.label(format!("texture page: {}   masks: {}", am.texture_no, am.mask_count));
            if let Some(ev) = self.eval.get(s) {
                ui.label(format!("opacity now: {:.2}", ev.opacity));
            }
            ui.label(format!("part: {}", usize::try_from(am.parent_part).ok().and_then(|p| self.moc.parts.get(p)).map_or("(root)", |p| p.id.as_str())));
        }
    }

    fn params_tab(&mut self, ui: &mut egui::Ui) {
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Filter..."));
        let filter = self.filter.to_lowercase();
        egui::ScrollArea::vertical().show(ui, |ui| {
            let n = self.moc.parameters.len();
            let mut i = 0;
            while i < n {
                let gi = self.groups.iter().position(|g| g[0] == i);
                let label = self.param_label(i);
                let id = self.moc.parameters[i].id.clone();
                let visible = filter.is_empty() || label.to_lowercase().contains(&filter) || id.to_lowercase().contains(&filter);
                if let Some(gi) = gi {
                    let members = self.groups[gi].clone();
                    if visible || members.iter().any(|&m| self.param_label(m).to_lowercase().contains(&filter)) {
                        ui.group(|ui| {
                            let title = if members.len() == 2 { "Combined: X / Y pad".to_string() } else { format!("Combined x{}: X / Y pad + extra sliders", members.len()) };
                            ui.label(egui::RichText::new(title).small().weak());
                            for &m in &members {
                                let l = self.param_label(m);
                                self.name_row(ui, m, &l);
                            }
                            self.pad(ui, members[0], members[1]);
                            for &m in &members[2..] {
                                let l = self.param_label(m);
                                self.slider_row(ui, m, &l);
                            }
                            ui.horizontal(|ui| {
                                let last = *members.last().unwrap();
                                let free = last + 1 < n && !self.groups.iter().any(|g| g.contains(&(last + 1)));
                                if free && ui.small_button("Add next").on_hover_text(format!("Add {} to this group", self.param_label(last + 1))).clicked() {
                                    self.groups[gi].push(last + 1);
                                }
                                if members.len() > 2 && ui.small_button("Remove last").clicked() {
                                    self.groups[gi].pop();
                                }
                                if ui.small_button("Split").on_hover_text("Back to separate sliders").clicked() {
                                    self.groups.remove(gi);
                                }
                            });
                        });
                    }
                    i += members.len();
                    continue;
                }
                if visible {
                    self.slider_row(ui, i, &label);
                    let can_pair = i + 1 < n && !self.groups.iter().any(|g| g.contains(&(i + 1)));
                    if can_pair && ui.small_button("Combine with next").on_hover_text(format!("Join with {} as one group (a third parameter can be added after)", self.param_label(i + 1))).clicked() {
                        self.groups.push(vec![i, i + 1]);
                        self.groups.sort();
                    }
                }
                i += 1;
            }
        });
    }

    fn name_row(&mut self, ui: &mut egui::Ui, i: usize, label: &str) {
        if self.edit_names {
            let id = self.moc.parameters[i].id.clone();
            let base = self.cdi_params.get(&id).cloned().unwrap_or_else(|| id.clone());
            let mut s = label.to_string();
            if ui.add(egui::TextEdit::singleline(&mut s).desired_width(170.0)).changed() {
                set_name(&mut self.ov.param_names, &id, &s, &base);
            }
        } else {
            ui.label(label);
        }
    }

    fn slider_row(&mut self, ui: &mut egui::Ui, i: usize, label: &str) {
        let p = &self.moc.parameters[i];
        let (lo, hi) = (p.min.min(p.max), p.min.max(p.max));
        let default = p.default;
        self.name_row(ui, i, label);
        let r = ui.add(egui::Slider::new(&mut self.params[i], lo..=hi).clamping(egui::SliderClamping::Always));
        if r.changed() {
            self.dirty = true;
        }
        if r.double_clicked() {
            self.params[i] = default;
            self.dirty = true;
        }
    }

    fn pad(&mut self, ui: &mut egui::Ui, ix: usize, iy: usize) {
        let (px, py) = (&self.moc.parameters[ix], &self.moc.parameters[iy]);
        let (xlo, xhi, ylo, yhi) = (px.min.min(px.max), px.min.max(px.max), py.min.min(py.max), py.min.max(py.max));
        let (xdef, ydef) = (px.default, py.default);
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(150.0), Sense::click_and_drag());
        let norm = |v: f32, lo: f32, hi: f32| if hi > lo { ((v - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.5 };
        if let (true, Some(pos)) = (resp.dragged() || resp.clicked(), resp.interact_pointer_pos()) {
            let fx = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let fy = (1.0 - (pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            self.params[ix] = xlo + fx * (xhi - xlo);
            self.params[iy] = ylo + fy * (yhi - ylo);
            self.dirty = true;
        }
        if resp.double_clicked() {
            self.params[ix] = xdef;
            self.params[iy] = ydef;
            self.dirty = true;
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, Color32::from_gray(28));
        painter.rect_stroke(rect, 2.0, Stroke::new(1.0, Color32::from_gray(90)), egui::StrokeKind::Inside);
        let at = |vx: f32, vy: f32| Pos2::new(rect.left() + norm(vx, xlo, xhi) * rect.width(), rect.bottom() - norm(vy, ylo, yhi) * rect.height());
        let d = at(xdef, ydef);
        painter.line_segment([Pos2::new(rect.left(), d.y), Pos2::new(rect.right(), d.y)], Stroke::new(1.0, Color32::from_gray(70)));
        painter.line_segment([Pos2::new(d.x, rect.top()), Pos2::new(d.x, rect.bottom())], Stroke::new(1.0, Color32::from_gray(70)));
        painter.circle_filled(at(self.params[ix], self.params[iy]), 6.0, Color32::from_rgb(120, 200, 255));
        ui.horizontal(|ui| {
            for k in [ix, iy] {
                let p = &self.moc.parameters[k];
                let (lo, hi) = (p.min.min(p.max), p.min.max(p.max));
                if ui.add(egui::DragValue::new(&mut self.params[k]).range(lo..=hi).speed((hi - lo) / 200.0).fixed_decimals(2)).changed() {
                    self.dirty = true;
                }
            }
        });
    }

    fn mesh_visible_in_filter(&self, mi: usize, filter: &str) -> bool {
        filter.is_empty() || self.moc.art_meshes[mi].id.to_lowercase().contains(filter) || self.ov.mesh_name(&self.moc.art_meshes[mi].id).to_lowercase().contains(filter)
    }

    fn part_has_match(&self, p: usize, filter: &str) -> bool {
        filter.is_empty()
            || self.part_label(p).to_lowercase().contains(filter)
            || self.part_meshes[p].iter().any(|&m| self.mesh_visible_in_filter(m, filter))
            || self.part_children[p].iter().any(|&c| self.part_has_match(c, filter))
    }

    fn layers_tab(&mut self, ui: &mut egui::Ui) {
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Filter layers / parts..."));
        let filter = self.filter.to_lowercase();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for p in self.root_parts.clone() {
                self.part_node(ui, p, &filter);
            }
            for m in self.root_meshes.clone() {
                self.mesh_row(ui, m, &filter);
            }
        });
    }

    fn part_node(&mut self, ui: &mut egui::Ui, p: usize, filter: &str) {
        if !self.part_has_match(p, filter) {
            return;
        }
        let label = self.part_label(p);
        let mut open = egui::CollapsingHeader::new(label).id_salt(("part", p));
        if !filter.is_empty() {
            open = open.open(Some(true));
        }
        ui.horizontal(|ui| {
            let mut shown = !self.hidden_parts[p];
            if ui.checkbox(&mut shown, "").on_hover_text("Show / hide this whole part").changed() {
                self.hidden_parts[p] = !shown;
                self.dirty = true;
            }
            open.show(ui, |ui| {
                if self.edit_names {
                    let id = self.moc.parts[p].id.clone();
                    let base = self.cdi_parts.get(&id).cloned().unwrap_or_else(|| id.clone());
                    let mut s = self.part_label(p);
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        if ui.add(egui::TextEdit::singleline(&mut s).desired_width(170.0)).changed() {
                            set_name(&mut self.ov.part_names, &id, &s, &base);
                        }
                    });
                }
                for c in self.part_children[p].clone() {
                    self.part_node(ui, c, filter);
                }
                for m in self.part_meshes[p].clone() {
                    self.mesh_row(ui, m, filter);
                }
            });
        });
    }

    fn mesh_row(&mut self, ui: &mut egui::Ui, mi: usize, filter: &str) {
        if !self.mesh_visible_in_filter(mi, filter) {
            return;
        }
        let id = self.moc.art_meshes[mi].id.clone();
        ui.horizontal(|ui| {
            let mut shown = !self.hidden_meshes[mi];
            if ui.checkbox(&mut shown, "").changed() {
                self.hidden_meshes[mi] = !shown;
                self.dirty = true;
            }
            if self.edit_names {
                let mut s = self.ov.mesh_name(&id).to_string();
                if ui.add(egui::TextEdit::singleline(&mut s).desired_width(150.0)).changed() {
                    set_name(&mut self.ov.mesh_names, &id, &s, &id);
                }
                ui.label(egui::RichText::new(&id).weak().small());
            } else {
                let sel = self.selected == Some(mi);
                if ui.selectable_label(sel, self.ov.mesh_name(&id)).clicked() {
                    self.selected = Some(mi);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, ed: &mut Editor) -> usize {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0))), ..Default::default() };
        let out = ctx.run(input, |ctx| {
            egui::SidePanel::right("s").show(ctx, |ui| ed.side_panel(ui));
            egui::CentralPanel::default().show(ctx, |ui| ed.viewport(ui));
        });
        ctx.tessellate(out.shapes, out.pixels_per_point).len()
    }

    /// Runs real frames of the editor on a sample model:
    /// `cargo test -p mint-gui --release -- --ignored editor`
    #[test]
    #[ignore]
    fn headless_frames() {
        let dir = "../../testdata/samples/Mark";
        let path = PathBuf::from(format!("{dir}/Mark.moc3"));
        let moc = Moc3::parse(&std::fs::read(&path).unwrap()).unwrap();
        let atlases = vec![image::open(format!("{dir}/Mark.2048/texture_00.png")).unwrap().into_rgba8()];
        let ctx = egui::Context::default();
        let mut ed = Editor::new(&ctx, path, moc, atlases);

        for tab in [Tab::Params, Tab::Layers, Tab::View] {
            ed.tab = tab;
            assert!(frame(&ctx, &mut ed) > 0, "nothing drawn");
        }
        // move parameters, turn on every overlay and name editing
        ed.params[0] = 20.0;
        ed.params[1] = -10.0;
        ed.dirty = true;
        ed.wire = Overlay::All;
        ed.points = Overlay::All;
        ed.edit_names = true;
        for tab in [Tab::Params, Tab::Layers, Tab::View] {
            ed.tab = tab;
            frame(&ctx, &mut ed);
        }
        // pick a layer in the middle of the canvas
        let c = Pos2::new(ed.moc.canvas.width / 2.0, ed.moc.canvas.height / 2.0);
        let picked = ed.pick(c);
        println!("picked {:?}", picked.map(|m| ed.moc.art_meshes[m].id.clone()));
        ed.selected = picked.or(Some(0));
        ed.wire = Overlay::Selected;
        ed.points = Overlay::Selected;
        frame(&ctx, &mut ed);
        // hide a part and a layer, break / make pairs
        ed.hidden_parts[0] = true;
        ed.hidden_meshes[0] = true;
        let before = ed.groups.len();
        ed.groups.clear();
        ed.dirty = true;
        frame(&ctx, &mut ed);
        ed.groups = mint_core::inp::find_pairs(ed.moc).into_iter().map(|(a, b)| vec![a, b]).collect();
        assert_eq!(ed.groups.len(), before);
        // a group of three adjacent parameters (angle X / Y / Z) round-trips
        ed.groups = vec![vec![0, 1, 2]];
        ed.dirty = true;
        for tab in [Tab::Params, Tab::Layers] {
            ed.tab = tab;
            frame(&ctx, &mut ed);
        }
        let ov3 = ed.overrides();
        assert_eq!(ov3.groups.len(), 1);
        assert_eq!(ov3.pairs.as_ref().map(|p| p.len()), Some(1), "exports keep the first two as the pair");
        ed.ov = ov3;
        ed.load_state();
        assert_eq!(ed.groups, vec![vec![0, 1, 2]]);
        ed.groups = mint_core::inp::find_pairs(ed.moc).into_iter().map(|(a, b)| vec![a, b]).collect();
        ed.ov.groups.clear();
        ed.ov.pairs = None;
        ed.hidden_parts[0] = true;
        ed.hidden_meshes[0] = true;
        // names and the overrides round-trip
        set_name(&mut ed.ov.part_names, &ed.moc.parts[0].id.clone(), "Renamed part", "x");
        ed.ov.apply_visibility = true;
        let ov = ed.overrides();
        assert!(ov.hidden_parts.contains(&ed.moc.parts[0].id) && ov.hidden_meshes.contains(&ed.moc.art_meshes[0].id));
        assert_eq!(ov.part_names.get(&ed.moc.parts[0].id).map(String::as_str), Some("Renamed part"));
        assert!(ov.pairs.is_none(), "default pairs should stay automatic");
        let again = Overrides::from_json(&ov.to_json());
        assert_eq!(again, ov);
        println!("editor frames ok, {} meshes, {} groups", ed.moc.art_meshes.len(), ed.groups.len());
    }

    /// Timing on a large model: `MINT_BIG=testdata/samples/Ren/Ren.moc3 cargo test ... -- --ignored big`
    #[test]
    #[ignore]
    fn big_model_timing() {
        let Ok(p) = std::env::var("MINT_BIG") else { return };
        let path = PathBuf::from(format!("../../{p}"));
        let moc = Moc3::parse(&std::fs::read(&path).unwrap()).unwrap();
        let dir = path.parent().unwrap();
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        let model3 = std::fs::read(dir.join(format!("{stem}.model3.json"))).unwrap();
        let j: serde_json::Value = serde_json::from_slice(&model3).unwrap();
        let atlases: Vec<RgbaImage> = j["FileReferences"]["Textures"].as_array().unwrap().iter().map(|t| {
            let img = image::open(dir.join(t.as_str().unwrap().replace('\\', "/"))).unwrap().into_rgba8();
            let m = img.width().max(img.height());
            if m > 4096 { let s = 4096.0 / m as f32; image::imageops::resize(&img, (img.width() as f32 * s) as u32, (img.height() as f32 * s) as u32, image::imageops::FilterType::Triangle) } else { img }
        }).collect();
        let ctx = egui::Context::default();
        let t = std::time::Instant::now();
        let mut ed = Editor::new(&ctx, path, moc, atlases);
        println!("new: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        frame(&ctx, &mut ed);
        println!("first frame (evaluate + draw): {:?}", t.elapsed());
        let t = std::time::Instant::now();
        for k in 0..10 {
            ed.params[0] = k as f32 * 3.0;
            ed.dirty = true;
            frame(&ctx, &mut ed);
        }
        println!("per frame while dragging a parameter: {:?}", t.elapsed() / 10);
        ed.wire = Overlay::All;
        ed.points = Overlay::All;
        let t = std::time::Instant::now();
        frame(&ctx, &mut ed);
        println!("frame with all wireframes + points: {:?}", t.elapsed());
        // exact render of a 1400x900 window (what the editor does while you tweak)
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16);
        let cs = ed.canvas_size();
        for (label, scale) in [("fit", (900.0 / cs.y).min(1400.0 / cs.x)), ("zoomed in 1:1", 1.0f32)] {
            let textures: Vec<RgbaImage> = ed.atlases.iter().map(|a| a.clone()).collect();
            let mut m = Model::new(ed.moc);
            m.px_scale = scale;
            m.px_offset = if scale >= 1.0 { (cs.x / 2.0 - 700.0, cs.y * 0.15) } else { (0.0, 0.0) };
            m.set_hidden_parts(&ed.hidden_parts);
            let mut best = std::time::Duration::MAX;
            let mut img = RgbaImage::new(1, 1);
            for _ in 0..3 {
                let t = std::time::Instant::now();
                img = mint_core::render::render_view(&mut m, &textures, (1400, 900), threads);
                best = best.min(t.elapsed());
            }
            println!("exact window render ({label}, {threads} threads): {:?}", best);
            if let Ok(dir) = std::env::var("MINT_SAVE") {
                let name = label.replace([' ', ':'], "_");
                let _ = img.save(format!("{dir}/window_{name}.png"));
            }
        }
    }
}
