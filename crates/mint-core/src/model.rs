//! Default-pose evaluation: parameter -> keyform weights -> deformer
//! transforms -> canvas-space vertex positions.
//!
//! Semantics follow the published moc3 format and the open-source
//! reference implementations (OpenL2D moc3ingbird, moc3-rs).

use crate::moc3::{DeformerType, Moc3};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Self = Vec2 { x: 0.0, y: 0.0 };
    fn new(x: f32, y: f32) -> Self {
        Vec2 { x, y }
    }
}

impl std::ops::Add<Vec2> for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}
impl std::ops::Sub<Vec2> for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}
impl std::ops::Mul<f32> for Vec2 {
    type Output = Vec2;
    fn mul(self, s: f32) -> Vec2 {
        Vec2::new(self.x * s, self.y * s)
    }
}
impl std::ops::Mul<Vec2> for Vec2 {
    type Output = Vec2;
    fn mul(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x * o.x, self.y * o.y)
    }
}
impl std::ops::Div<f32> for Vec2 {
    type Output = Vec2;
    fn div(self, s: f32) -> Vec2 {
        Vec2::new(self.x / s, self.y / s)
    }
}

fn lerp(a: Vec2, b: Vec2, t: f32) -> Vec2 {
    a + (b - a) * t
}

fn bilinear(t: Vec2, p00: Vec2, p10: Vec2, p01: Vec2, p11: Vec2) -> Vec2 {
    lerp(lerp(p00, p10, t.x), lerp(p01, p11, t.x), t.y)
}

/// Barycentric interpolation across a quad split into two triangles
/// along the diagonal (p00, p11).
fn triangular(t: Vec2, p00: Vec2, p10: Vec2, p01: Vec2, p11: Vec2) -> Vec2 {
    if t.x < t.y {
        p00 * (1.0 - t.y) + p01 * (t.y - t.x) + p11 * t.x
    } else {
        p00 * (1.0 - t.x) + p10 * (t.x - t.y) + p11 * t.y
    }
}

/// Locate `value` in a sorted key table -> (lower index, fraction).
fn key_segment(keys: &[f32], value: f32) -> (usize, f32) {
    if keys.len() < 2 {
        return (0, 0.0);
    }
    let last = keys.len() - 1;
    if value <= keys[0] {
        return (0, 0.0);
    }
    if value >= keys[last] {
        return (last - 1, 1.0);
    }
    let upper = keys.partition_point(|&k| k <= value).clamp(1, last);
    (upper - 1, (value - keys[upper - 1]) / (keys[upper] - keys[upper - 1]))
}

/// One bound parameter of an object: the parameter index plus its key
/// domain for this binding.
#[derive(Debug, Clone)]
struct Binding {
    param: usize,
    keys: std::ops::Range<usize>,
}

pub struct Model<'a> {
    pub moc: &'a Moc3,
    /// Current parameter values (index-aligned with `moc.parameters`).
    pub params: Vec<f32>,
    /// parameterBinding index -> parameter index
    pb_to_param: Vec<i32>,
    // memoized evaluation state
    kf_weights: Vec<Vec<f32>>,
    warp_local: Vec<Option<Vec<Vec2>>>,   // grid verts in parent space
    rot_local: Vec<Option<TransformData>>,
    rot_world: Vec<Option<RotWorld>>,
    warp_world: Vec<Option<Vec<Vec2>>>,
    hidden_parts: Vec<bool>,
}

/// Full child-space -> canvas transform of a rotation deformer.
#[derive(Debug, Clone, Copy)]
struct RotWorld {
    xx: f32,
    xy: f32,
    yx: f32,
    yy: f32,
    origin: Vec2,
    acc_scale: f32,
}

#[derive(Debug, Clone, Copy, Default)]
struct TransformData {
    origin: Vec2,
    scale: f32,
    angle: f32,
}

impl<'a> Model<'a> {
    pub fn new(moc: &'a Moc3) -> Self {
        let params: Vec<f32> = moc.parameters.iter().map(|p| p.default).collect();
        // pb index -> param index, via parameter.binding_sources which
        // directly indexes parameterBindings.
        let mut pb_to_param = vec![-1i32; moc.parameter_bindings.len()];
        for (p, prm) in moc.parameters.iter().enumerate() {
            let b = prm.binding_sources_begin.max(0) as usize;
            let e = b + prm.binding_sources_count.max(0) as usize;
            for j in b..e.min(pb_to_param.len()) {
                pb_to_param[j] = p as i32;
            }
        }
        let nkfb = moc.keyform_bindings.len();
        Model {
            moc,
            params,
            pb_to_param,
            kf_weights: vec![Vec::new(); nkfb],
            warp_local: vec![None; moc.warp_deformers.len()],
            rot_local: vec![None; moc.rotation_deformers.len()],
            rot_world: vec![None; moc.deformers.len()],
            warp_world: vec![None; moc.deformers.len()],
            hidden_parts: vec![false; moc.parts.len()],
        }
    }

    /// Hide every mesh inside the given parts (by id) in this model's
    /// evaluation, e.g. pose-hidden alternates or offscreen "Out" parts.
    pub fn hide_parts(&mut self, ids: &std::collections::HashSet<String>) {
        for (i, p) in self.moc.parts.iter().enumerate() {
            self.hidden_parts[i] = ids.contains(&p.id);
        }
    }

    /// Hide the parts the model's pose/offscreen data say are off by default.
    pub fn hide_default_hidden(&mut self, moc_path: &std::path::Path) {
        let mut ids = crate::emit::hidden_parts(self.moc, Some(moc_path));
        // "Out" parts are cut-outs handled by the renderer, not hidden.
        for (p, out) in self.moc.parts.iter().zip(&self.moc.offscreen_out_parts) {
            if *out {
                ids.remove(&p.id);
            }
        }
        self.hide_parts(&ids);
    }

    /// Nearest ancestor part of mesh `mi` that is an offscreen "Out" cut-out.
    pub fn out_ancestor(&self, mi: usize) -> Option<usize> {
        let (mut part, mut guard) = (self.moc.art_meshes[mi].parent_part, 0);
        while part >= 0 && (part as usize) < self.moc.parts.len() && guard < 64 {
            if self.moc.offscreen_out_parts.get(part as usize).copied().unwrap_or(false) {
                return Some(part as usize);
            }
            part = self.moc.parts[part as usize].parent_part;
            guard += 1;
        }
        None
    }

    pub fn part_hidden(&self, part: usize) -> bool {
        self.hidden_parts.get(part).copied().unwrap_or(false)
    }

    /// Reset memoized state after changing `params`.
    pub fn invalidate(&mut self) {
        for w in &mut self.kf_weights {
            w.clear();
        }
        self.warp_local.fill(None);
        self.rot_local.fill(None);
        self.rot_world.fill(None);
        self.warp_world.fill(None);
    }

    /// Bound (param, keys) pairs for an object given its
    /// `keyform_binding_source` index. Returns empty for unbound objects.
    fn bindings(&self, kbs: i32) -> Vec<Binding> {
        if kbs < 0 {
            return Vec::new();
        }
        let kfb = &self.moc.keyform_bindings[kbs as usize];
        let b = kfb.parameter_binding_index_begin.max(0) as usize;
        let e = b + kfb.parameter_binding_index_count.max(0) as usize;
        self.moc.parameter_binding_indices[b..e.min(self.moc.parameter_binding_indices.len())]
            .iter()
            .map(|&pbi| {
                let pb = &self.moc.parameter_bindings[pbi as usize];
                Binding {
                    param: self.pb_to_param[pbi as usize].max(0) as usize,
                    keys: pb.keys_begin.max(0) as usize
                        ..(pb.keys_begin.max(0) + pb.keys_count.max(0)) as usize,
                }
            })
            .collect()
    }

    /// Per-keyform interpolation weights for an object, given current
    /// parameter values. First bound parameter varies fastest.
    /// Unbound objects use their first keyform at full weight.
    fn weights(&mut self, kbs: i32) -> &[f32] {
        if kbs < 0 {
            return &[1.0];
        }
        let kbs = kbs as usize;
        if !self.kf_weights[kbs].is_empty() {
            return &self.kf_weights[kbs];
        }
        let bindings = self.bindings(kbs as i32);
        if bindings.is_empty() {
            self.kf_weights[kbs] = vec![1.0];
            return &self.kf_weights[kbs];
        }
        // per-param (lower, frac)
        let segs: Vec<(usize, f32)> = bindings
            .iter()
            .map(|b| key_segment(&self.moc.keys[b.keys.clone()], self.params[b.param]))
            .collect();
        let total: usize = bindings
            .iter()
            .map(|b| b.keys.end - b.keys.start)
            .product();
        let mut w = vec![0.0f32; total];
        for num in 0..(1usize << bindings.len()) {
            let mut mult = 1.0;
            let mut index = 0;
            let mut stride = 1;
            for (i, b) in bindings.iter().enumerate() {
                let n = b.keys.end - b.keys.start;
                let (lower, frac) = segs[i];
                index += lower * stride;
                if num & (1 << i) != 0 {
                    index += stride;
                    mult *= frac;
                } else {
                    mult *= 1.0 - frac;
                }
                stride *= n;
            }
            if index < w.len() {
                w[index] += mult;
            }
        }
        self.kf_weights[kbs] = w;
        &self.kf_weights[kbs]
    }

    // ---------------- deformer evaluation ----------------

    /// Grid vertex positions of warp deformer `wi` in its parent space.
    fn warp_local_grid(&mut self, wi: usize) -> Vec<Vec2> {
        if let Some(g) = &self.warp_local[wi] {
            return g.clone();
        }
        let wd = &self.moc.warp_deformers[wi];
        let count = wd.vertex_count.max(0) as usize;
        let w = self.weights(wd.keyform_binding_source).to_vec();
        let mut grid = vec![Vec2::ZERO; count];
        for (k, &weight) in w.iter().enumerate() {
            if weight == 0.0 {
                continue;
            }
            let kf = &self.moc.warp_deformer_keyforms[wd.keyform_sources_begin.max(0) as usize + k];
            let base = kf.position_sources_begin.max(0) as usize / 2;
            for v in 0..count {
                let p = self.moc.keyform_positions.get(base + v).copied().unwrap_or([0.0; 2]);
                grid[v] = grid[v] + Vec2::new(p[0], p[1]) * weight;
            }
        }
        self.warp_local[wi] = Some(grid.clone());
        grid
    }

    fn warp_world_grid(&mut self, di: usize) -> Vec<Vec2> {
        if let Some(g) = &self.warp_world[di] {
            return g.clone();
        }
        let d = &self.moc.deformers[di];
        let (wi, parent) = (d.specific_index as usize, d.parent_deformer);
        let local = self.warp_local_grid(wi);
        let world: Vec<Vec2> = if parent < 0 {
            local
        } else {
            local.into_iter().map(|pt| self.point_to_canvas(parent, pt)).collect()
        };
        self.warp_world[di] = Some(world.clone());
        world
    }

    /// Map a point expressed in deformer `di`'s child-space into canvas
    /// space, recursively.
    fn point_to_canvas(&mut self, di: i32, p: Vec2) -> Vec2 {
        if di < 0 {
            return p;
        }
        let d = &self.moc.deformers[di as usize];
        let parent_space = match d.ty {
            DeformerType::Warp => {
                // Cubism maps the lattice points through the parent chain and
                // interpolates in canvas space (not the other way round).
                let wi = d.specific_index as usize;
                let world = self.warp_world_grid(di as usize);
                let quad = self.moc.warp_deformer_quad_source[wi];
                let wd = &self.moc.warp_deformers[wi];
                return warp_apply(&world, wd.rows as usize, wd.columns as usize, quad, p);
            }
            DeformerType::Rotation => {
                let w = self.rot_world_of(di as usize);
                return Vec2::new(
                    w.xx * p.x + w.xy * p.y + w.origin.x,
                    w.yx * p.x + w.yy * p.y + w.origin.y,
                );
            }
            DeformerType::Unknown(_) => p,
        };
        self.point_to_canvas(d.parent_deformer, parent_space)
    }

    fn acc_scale_of(&mut self, di: i32) -> f32 {
        if di < 0 {
            return 1.0;
        }
        let d = &self.moc.deformers[di as usize];
        let parent = d.parent_deformer;
        match d.ty {
            DeformerType::Rotation => self.rot_world_of(di as usize).acc_scale,
            _ => self.acc_scale_of(parent),
        }
    }

    /// World transform of rotation deformer `di`: the child coordinates
    /// are mapped straight to canvas space (warp parents only supply the
    /// origin and the inherited direction, never rescale the children).
    fn rot_world_of(&mut self, di: usize) -> RotWorld {
        if let Some(w) = self.rot_world[di] {
            return w;
        }
        let d = &self.moc.deformers[di];
        let (ri, parent) = (d.specific_index as usize, d.parent_deformer);
        let t = self.rot_local_data(ri);
        let rd = &self.moc.rotation_deformers[ri];
        let (mut rx, mut ry) = (false, false);
        let ws = self.weights(rd.keyform_binding_source).to_vec();
        if let Some(k) = ws.iter().position(|&w| w > 0.0) {
            let kf = &self.moc.rotation_deformer_keyforms[rd.keyform_sources_begin.max(0) as usize + k];
            rx = kf.reflect_x;
            ry = kf.reflect_y;
        }
        let base = rd.base_angle;
        let (origin, angle_deg, acc) = if parent < 0 {
            (t.origin, base + t.angle, t.scale)
        } else {
            let world_origin = self.point_to_canvas(parent, t.origin);
            let parent_is_rot = matches!(self.moc.deformers[parent as usize].ty, DeformerType::Rotation);
            let disp: f32 = if parent_is_rot { -10.0 } else { -0.1 };
            let mut delta = Vec2::ZERO;
            let mut ps = 1.0f32;
            for _ in 0..10 {
                let probe = self.point_to_canvas(parent, Vec2::new(t.origin.x, t.origin.y + disp * ps));
                delta = probe - world_origin;
                if delta != Vec2::ZERO {
                    break;
                }
                let probe = self.point_to_canvas(parent, Vec2::new(t.origin.x, t.origin.y - disp * ps));
                delta = world_origin - probe;
                if delta != Vec2::ZERO {
                    break;
                }
                ps *= 0.1;
            }
            let inherited = if delta == Vec2::ZERO {
                0.0
            } else {
                let mut a = disp.atan2(0.0) - delta.y.atan2(delta.x);
                while a > std::f32::consts::PI { a -= 2.0 * std::f32::consts::PI; }
                while a <= -std::f32::consts::PI { a += 2.0 * std::f32::consts::PI; }
                a
            };
            let acc = self.acc_scale_of(parent) * t.scale;
            (world_origin, base + t.angle - inherited.to_degrees(), acc)
        };
        let (s, c) = angle_deg.to_radians().sin_cos();
        let (fx, fy) = (if rx { -1.0 } else { 1.0 }, if ry { -1.0 } else { 1.0 });
        let w = RotWorld { xx: c * acc * fx, xy: -s * acc * fy, yx: s * acc * fx, yy: c * acc * fy, origin, acc_scale: acc };
        self.rot_world[di] = Some(w);
        w
    }

    /// Interpolated transform data for rotation deformer `ri`
    /// (in deformer-local terms).
    fn rot_local_data(&mut self, ri: usize) -> TransformData {
        if let Some(t) = self.rot_local[ri] {
            return t;
        }
        let rd = &self.moc.rotation_deformers[ri];
        let w = self.weights(rd.keyform_binding_source).to_vec();
        let mut out = TransformData::default();
        for (k, &weight) in w.iter().enumerate() {
            if weight == 0.0 {
                continue;
            }
            let kf = &self.moc.rotation_deformer_keyforms[rd.keyform_sources_begin.max(0) as usize + k];
            out.origin = out.origin + Vec2::new(kf.origin_x, kf.origin_y) * weight;
            out.angle += kf.angle * weight;
            out.scale += kf.scale * weight;
        }
        if out.scale == 0.0 {
            out.scale = 1.0;
        }
        self.rot_local[ri] = Some(out);
        out
    }

    /// Opacity of deformer `di` (interpolated over its keyforms,
    /// multiplied up the parent chain).
    fn deformer_opacity_at(&mut self, di: usize) -> f32 {
        // recompute chain each call is fine for our sizes
        let d = &self.moc.deformers[di];
        let own = match d.ty {
            DeformerType::Warp => {
                let wd = &self.moc.warp_deformers[d.specific_index as usize];
                let w = self.weights(wd.keyform_binding_source).to_vec();
                let mut o = 0.0;
                for (k, &wt) in w.iter().enumerate() {
                    let kf = &self.moc.warp_deformer_keyforms[wd.keyform_sources_begin.max(0) as usize + k];
                    o += kf.opacity * wt;
                }
                o
            }
            DeformerType::Rotation => {
                let rd = &self.moc.rotation_deformers[d.specific_index as usize];
                let w = self.weights(rd.keyform_binding_source).to_vec();
                let mut o = 0.0;
                for (k, &wt) in w.iter().enumerate() {
                    let kf = &self.moc.rotation_deformer_keyforms[rd.keyform_sources_begin.max(0) as usize + k];
                    o += kf.opacity * wt;
                }
                o
            }
            DeformerType::Unknown(_) => 1.0,
        };
        if d.parent_deformer >= 0 {
            own * self.deformer_opacity_at(d.parent_deformer as usize)
        } else {
            own
        }
    }

    // ---------------- public API ----------------

    /// Interpolated (multiply, screen) colours of art mesh `mi`; white / black when
    /// the model has no keyform colours.
    pub fn mesh_colors(&mut self, mi: usize) -> ([f32; 3], [f32; 3]) {
        let am = &self.moc.art_meshes[mi];
        let begin = self.moc.mesh_color_begin.get(mi).copied().unwrap_or(-1);
        let (mut mul, mut scr) = ([1.0f32; 3], [0.0f32; 3]);
        if begin < 0 || self.moc.multiply_colors.is_empty() {
            return (mul, scr);
        }
        let w = self.weights(am.keyform_binding_source).to_vec();
        (mul, scr) = ([0.0; 3], [0.0; 3]);
        for (k, &wt) in w.iter().enumerate() {
            let i = begin as usize + k;
            let m = self.moc.multiply_colors.get(i).copied().unwrap_or([1.0; 3]);
            let s = self.moc.screen_colors.get(i).copied().unwrap_or([0.0; 3]);
            for ch in 0..3 {
                mul[ch] += m[ch] * wt;
                scr[ch] += s[ch] * wt;
            }
        }
        (mul, scr)
    }

    /// Interpolated vertex positions of art mesh `mi` in canvas units,
    /// its final opacity, and its interpolated draw order.
    pub fn art_mesh(&mut self, mi: usize) -> (Vec<Vec2>, f32, f32) {
        let am = &self.moc.art_meshes[mi];
        let count = am.vertex_count.max(0) as usize;
        let w = self.weights(am.keyform_binding_source).to_vec();
        let mut verts = vec![Vec2::ZERO; count];
        let mut opacity = 0.0;
        let mut draw_order = 0.0;
        for (k, &weight) in w.iter().enumerate() {
            if weight == 0.0 {
                continue;
            }
            let kf = &self.moc.art_mesh_keyforms[am.keyform_sources_begin.max(0) as usize + k];
            opacity += kf.opacity * weight;
            draw_order += kf.draw_order * weight;
            let base = kf.position_sources_begin.max(0) as usize / 2;
            for v in 0..count {
                let p = self.moc.keyform_positions.get(base + v).copied().unwrap_or([0.0; 2]);
                verts[v] = verts[v] + Vec2::new(p[0], p[1]) * weight;
            }
        }
        let parent_chain_opacity = if am.parent_deformer >= 0 {
            self.deformer_opacity_at(am.parent_deformer as usize)
        } else {
            1.0
        };
        let mut parent_chain_opacity = parent_chain_opacity;
        let (mut part, mut guard) = (am.parent_part, 0);
        while part >= 0 && (part as usize) < self.moc.parts.len() && guard < 64 {
            parent_chain_opacity *= self.moc.part_offscreen_opacity.get(part as usize).copied().unwrap_or(1.0);
            if self.hidden_parts.get(part as usize).copied().unwrap_or(false) {
                parent_chain_opacity = 0.0;
            }
            part = self.moc.parts[part as usize].parent_part;
            guard += 1;
        }
        let verts: Vec<Vec2> = verts
            .into_iter()
            .map(|v| self.point_to_canvas(am.parent_deformer, v))
            .collect();
        (verts, opacity * parent_chain_opacity, draw_order)
    }

    /// Convert a canvas-unit point to pixel coordinates.
    pub fn canvas_to_px(&self, v: Vec2) -> Vec2 {
        let c = &self.moc.canvas;
        let x = v.x * c.pixels_per_unit + c.origin_x;
        let y = if c.reverse_y {
            c.origin_y - v.y * c.pixels_per_unit
        } else {
            v.y * c.pixels_per_unit + c.origin_y
        };
        Vec2::new(x, y)
    }

    /// Interpolated draw order of part `pi`.
    fn part_draw_order(&mut self, pi: usize) -> f32 {
        let part = &self.moc.parts[pi];
        let w = self.weights(part.keyform_binding_source).to_vec();
        let mut d = 0.0;
        for (k, &wt) in w.iter().enumerate() {
            if wt == 0.0 {
                continue;
            }
            let kf = &self.moc.part_keyforms[part.keyform_sources_begin.max(0) as usize + k];
            d += kf.draw_order * wt;
        }
        d
    }

    /// Mesh render order (back to front), honoring draw order groups.
    /// Port of the standard draw-order-group sort: objects are sorted
    /// by interpolated draw order within each group (ties keep file
    /// order); part objects reserve a slot for the group they own.
    pub fn render_order(&mut self) -> Vec<usize> {
        use crate::moc3::DrawOrderObjectType;
        let moc = self.moc;
        let n_mesh = moc.art_meshes.len();
        if moc.draw_order_groups.is_empty() {
            return (0..n_mesh).collect();
        }
        // interpolated draw orders
        let mesh_dos: Vec<f32> = (0..n_mesh)
            .map(|mi| self.art_mesh(mi).2)
            .collect();
        let part_dos: Vec<f32> = (0..moc.parts.len())
            .map(|pi| self.part_draw_order(pi))
            .collect();
        let mesh_opacity: Vec<f32> = (0..n_mesh)
            .map(|mi| {
                let am = &self.moc.art_meshes[mi];
                if !am.visible || !am.enabled {
                    0.0
                } else {
                    1.0
                }
            })
            .collect();

        let mut sequence = vec![u32::MAX; n_mesh + moc.parts.len()];
        let mut cursors = vec![0u32; moc.draw_order_groups.len()];

        for (gi, group) in moc.draw_order_groups.iter().enumerate() {
            let count = group.objects_count.max(0) as usize;
            if count == 0 {
                continue;
            }
            let start = group.objects_begin.max(0) as usize;
            let mut sorted: Vec<u32> = (0..count as u32).collect();
            sorted.sort_by_key(|&j| {
                let obj = &moc.draw_order_group_objects[start + j as usize];
                let i = obj.index.max(0) as usize;
                let order = match obj.ty {
                    DrawOrderObjectType::ArtMesh => {
                        (mesh_opacity[i] > 0.0).then(|| mesh_dos[i])
                    }
                    DrawOrderObjectType::Part => {
                        moc.parts[i].enabled.then(|| part_dos[i])
                    }
                    DrawOrderObjectType::Unknown(_) => None,
                };
                let order = order
                    .map_or(group.min_draw_order as i64, |o| o.round() as i64)
                    .clamp(group.min_draw_order as i64, group.max_draw_order as i64);
                (order, j)
            });
            let mut pos = cursors[gi] as usize;
            for &j in &sorted {
                let obj = &moc.draw_order_group_objects[start + j as usize];
                match obj.ty {
                    DrawOrderObjectType::Part => {
                        if obj.self_index >= 0 {
                            let owned = obj.self_index as usize;
                            cursors[owned] = pos as u32;
                            pos += moc.draw_order_groups[owned].objects_total_count.max(0) as usize;
                        }
                    }
                    DrawOrderObjectType::ArtMesh => {
                        if pos < sequence.len() {
                            sequence[pos] = obj.index as u32;
                        }
                        pos += 1;
                    }
                    _ => {}
                }
            }
        }

        sequence
            .iter()
            .copied()
            .filter(|&m| m != u32::MAX && mesh_opacity[m as usize] > 0.0)
            .map(|m| m as usize)
            .collect()
    }
}

/// Apply a warp deformer grid to a normalized-grid point.
fn warp_apply(grid: &[Vec2], rows: usize, columns: usize, quad: bool, p: Vec2) -> Vec2 {
    // Points outside the lattice extend the nearest edge cell (u / v leave 0..1)
    // instead of using one global parallelogram, which squashes strongly
    // warped lattices.
    let cell = |v: f32, n: usize| -> usize {
        if v <= 0.0 {
            0
        } else if v >= n as f32 {
            n - 1
        } else {
            v as usize
        }
    };
    let (gx, gy) = (p.x * columns as f32, p.y * rows as f32);
    let (col, row) = (cell(gx, columns), cell(gy, rows));
    let (u, v) = (gx - col as f32, gy - row as f32);
    let cp = columns + 1;
    let i = row * cp + col;
    let (p00, p10, p01, p11) = (grid[i], grid[i + 1], grid[i + cp], grid[i + cp + 1]);
    if quad {
        let top = p00 * (1.0 - u) + p10 * u;
        let bottom = p01 * (1.0 - u) + p11 * u;
        top * (1.0 - v) + bottom * v
    } else if u + v <= 1.0 {
        p00 * (1.0 - u - v) + p10 * u + p01 * v
    } else {
        p11 * (u + v - 1.0) + p01 * (1.0 - u) + p10 * (1.0 - v)
    }
}

fn rot_apply(t: TransformData, base_angle: f32, p: Vec2) -> Vec2 {
    let a = (base_angle + t.angle).to_radians();
    let (s, c) = a.sin_cos();
    let x = p.x * t.scale;
    let y = p.y * t.scale;
    Vec2::new(x * c - y * s + t.origin.x, x * s + y * c + t.origin.y)
}
