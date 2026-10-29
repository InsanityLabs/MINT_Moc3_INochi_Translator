//! MOC3 binary format parser.
//!
//! Clean-room implementation written against observed file layout (verified
//! on Cubism sample models v1/v3/v6) and the OpenL2D moc3ingbird spec for
//! field semantics. No Live2D SDK code is used.
//!
//! Layout:
//!   0x00  64-byte header: "MOC3" magic, u8 version, u8 big-endian flag
//!   0x40  section offset table: u32 countInfo ptr, u32 canvasInfo ptr,
//!         then one u32 pointer per *field array* of each section, in a
//!         fixed order. Runtime-space fields point to 8-bytes-per-element
//!         scratch blocks (ignored).
//!   countInfo: counts (u32 each) starting at the pointer itself.
//!   canvasInfo: 64-byte block (ppu, origin, size, flags).
//!
//! All multi-byte data is little-endian unless the header flag says
//! otherwise.

use std::fmt;

const MAGIC: &[u8; 4] = b"MOC3";
const ID_LEN: usize = 64;

#[derive(Debug)]
pub enum Error {
    Truncated { at: usize, needed: usize },
    BadMagic,
    BadVersion(u8),
    BadOffset { offset: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated { at, needed } => {
                write!(f, "truncated moc3 at {at:#x}, needed {needed} bytes")
            }
            Error::BadMagic => write!(f, "bad magic (expected MOC3)"),
            Error::BadVersion(v) => write!(f, "unsupported moc3 version {v}"),
            Error::BadOffset { offset } => write!(f, "offset {offset:#x} out of bounds"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Default, Clone, Copy)]
pub struct Counts {
    pub parts: usize,
    pub deformers: usize,
    pub warp_deformers: usize,
    pub rotation_deformers: usize,
    pub art_meshes: usize,
    pub parameters: usize,
    pub part_keyforms: usize,
    pub warp_deformer_keyforms: usize,
    pub rotation_deformer_keyforms: usize,
    pub art_mesh_keyforms: usize,
    pub keyform_positions: usize,
    pub parameter_binding_indices: usize,
    pub keyform_bindings: usize,
    pub parameter_bindings: usize,
    pub keys: usize,
    pub uvs: usize,
    pub position_indices: usize,
    pub drawable_masks: usize,
    pub draw_order_groups: usize,
    pub draw_order_group_objects: usize,
    pub glue: usize,
    pub glue_info: usize,
    pub glue_keyforms: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct Canvas {
    pub pixels_per_unit: f32,
    pub origin_x: f32,
    pub origin_y: f32,
    pub width: f32,
    pub height: f32,
    pub reverse_y: bool,
}

#[derive(Debug, Clone)]
pub struct Part {
    pub id: String,
    pub keyform_binding_source: i32,
    pub keyform_sources_begin: i32,
    pub keyform_sources_count: i32,
    pub visible: bool,
    pub enabled: bool,
    pub parent_part: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeformerType {
    Warp,
    Rotation,
    Unknown(u32),
}

#[derive(Debug, Clone)]
pub struct Deformer {
    pub id: String,
    pub keyform_binding_source: i32,
    pub visible: bool,
    pub enabled: bool,
    pub parent_part: i32,
    pub parent_deformer: i32,
    pub ty: DeformerType,
    /// Index into `warp_deformers` or `rotation_deformers` depending on `ty`.
    pub specific_index: i32,
}

#[derive(Debug, Clone)]
pub struct WarpDeformer {
    pub keyform_binding_source: i32,
    pub keyform_sources_begin: i32,
    pub keyform_sources_count: i32,
    pub vertex_count: i32,
    pub rows: u32,
    pub columns: u32,
}

#[derive(Debug, Clone)]
pub struct RotationDeformer {
    pub keyform_binding_source: i32,
    pub keyform_sources_begin: i32,
    pub keyform_sources_count: i32,
    pub base_angle: f32,
}

#[derive(Debug, Clone)]
pub struct ArtMesh {
    pub id: String,
    pub keyform_binding_source: i32,
    pub keyform_sources_begin: i32,
    pub keyform_sources_count: i32,
    pub visible: bool,
    pub enabled: bool,
    pub parent_part: i32,
    pub parent_deformer: i32,
    pub texture_no: u32,
    pub blend_mode: u8,
    pub double_sided: bool,
    pub inverted_mask: bool,
    pub vertex_count: i32,
    pub uv_begin: i32,
    pub position_index_begin: i32,
    pub position_index_count: i32,
    pub mask_begin: i32,
    pub mask_count: i32,
}

#[derive(Debug, Clone)]
pub struct Parameter {
    pub id: String,
    pub max: f32,
    pub min: f32,
    pub default: f32,
    pub repeat: bool,
    pub decimal_places: u32,
    pub binding_sources_begin: i32,
    pub binding_sources_count: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct PartKeyform {
    pub draw_order: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct WarpDeformerKeyform {
    pub opacity: f32,
    pub position_sources_begin: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct RotationDeformerKeyform {
    pub opacity: f32,
    pub angle: f32,
    pub origin_x: f32,
    pub origin_y: f32,
    pub scale: f32,
    pub reflect_x: bool,
    pub reflect_y: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ArtMeshKeyform {
    pub opacity: f32,
    pub draw_order: f32,
    pub position_sources_begin: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct KeyformBinding {
    pub parameter_binding_index_begin: i32,
    pub parameter_binding_index_count: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct ParameterBinding {
    pub keys_begin: i32,
    pub keys_count: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawOrderObjectType {
    ArtMesh,
    Part,
    Unknown(u32),
}

#[derive(Debug, Clone, Copy)]
pub struct DrawOrderGroup {
    pub objects_begin: i32,
    pub objects_count: i32,
    pub objects_total_count: i32,
    pub max_draw_order: u32,
    pub min_draw_order: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct DrawOrderGroupObject {
    pub ty: DrawOrderObjectType,
    pub index: i32,
    pub self_index: i32,
}

/// A parsed MOC3 file.
#[derive(Debug)]
pub struct Moc3 {
    pub version: u8,
    /// Parts (v6 / Cubism 5.3) drawn through an offscreen buffer with the
    /// "Out" alpha blend: only visible where nothing is behind them.
    pub offscreen_out_parts: Vec<bool>,
    pub canvas: Canvas,
    pub parts: Vec<Part>,
    pub deformers: Vec<Deformer>,
    pub warp_deformers: Vec<WarpDeformer>,
    pub rotation_deformers: Vec<RotationDeformer>,
    pub art_meshes: Vec<ArtMesh>,
    pub parameters: Vec<Parameter>,
    pub part_keyforms: Vec<PartKeyform>,
    pub warp_deformer_keyforms: Vec<WarpDeformerKeyform>,
    pub rotation_deformer_keyforms: Vec<RotationDeformerKeyform>,
    pub art_mesh_keyforms: Vec<ArtMeshKeyform>,
    /// Flat XY pool indexed by `position_sources_begin` on keyforms.
    pub keyform_positions: Vec<[f32; 2]>,
    pub parameter_binding_indices: Vec<i32>,
    pub keyform_bindings: Vec<KeyformBinding>,
    pub parameter_bindings: Vec<ParameterBinding>,
    pub keys: Vec<f32>,
    /// Flat UV pool indexed by `ArtMesh::uv_begin`.
    pub uvs: Vec<[f32; 2]>,
    pub position_indices: Vec<u16>,
    /// Art-mesh indices used as clipping masks.
    pub drawable_masks: Vec<i32>,
    pub draw_order_groups: Vec<DrawOrderGroup>,
    pub draw_order_group_objects: Vec<DrawOrderGroupObject>,
    /// v3.03+ only: whether each warp deformer uses bilinear (quad)
    /// interpolation. Older models always use triangular.
    pub warp_deformer_quad_source: Vec<bool>,
}

/// Sequential reader over the u32 offset table.
struct Table<'a> {
    r: &'a Reader<'a>,
    cur: usize,
}

impl<'a> Table<'a> {
    /// Next field pointer.
    fn ptr(&mut self) -> Result<usize> {
        let off = self.r.u32_at(self.cur)? as usize;
        self.cur += 4;
        Ok(off)
    }

    /// Skip `n` field pointers (runtime spaces etc.).
    fn skip(&mut self, n: usize) {
        self.cur += n * 4;
    }
}

struct Reader<'a> {
    data: &'a [u8],
    big_endian: bool,
}

impl<'a> Reader<'a> {
    fn check(&self, at: usize, len: usize) -> Result<()> {
        if at.checked_add(len).is_none_or(|end| end > self.data.len()) {
            return Err(Error::BadOffset { offset: at });
        }
        Ok(())
    }

    fn u32_at(&self, at: usize) -> Result<u32> {
        self.check(at, 4)?;
        let b: [u8; 4] = self.data[at..at + 4].try_into().unwrap();
        Ok(if self.big_endian { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) })
    }

    fn i32_at(&self, at: usize) -> Result<i32> {
        Ok(self.u32_at(at)? as i32)
    }

    fn f32_at(&self, at: usize) -> Result<f32> {
        Ok(f32::from_bits(self.u32_at(at)?))
    }

    fn u16_at(&self, at: usize) -> Result<u16> {
        self.check(at, 2)?;
        let b: [u8; 2] = self.data[at..at + 2].try_into().unwrap();
        Ok(if self.big_endian { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) })
    }

    fn i16_at(&self, at: usize) -> Result<i16> {
        Ok(self.u16_at(at)? as i16)
    }

    /// A contiguous array of 64-byte IDs at `base`.
    fn ids(&self, base: usize, n: usize) -> Result<Vec<String>> {
        self.check(base, n * ID_LEN)?;
        (0..n)
            .map(|i| {
                let raw = &self.data[base + i * ID_LEN..base + (i + 1) * ID_LEN];
                let end = raw.iter().position(|&b| b == 0).unwrap_or(ID_LEN);
                Ok(String::from_utf8_lossy(&raw[..end]).into_owned())
            })
            .collect()
    }

    fn f32s(&self, base: usize, n: usize) -> Result<Vec<f32>> {
        self.check(base, n * 4)?;
        (0..n).map(|i| self.f32_at(base + i * 4)).collect()
    }

    fn i32s(&self, base: usize, n: usize) -> Result<Vec<i32>> {
        self.check(base, n * 4)?;
        (0..n).map(|i| self.i32_at(base + i * 4)).collect()
    }

    fn u32s(&self, base: usize, n: usize) -> Result<Vec<u32>> {
        self.check(base, n * 4)?;
        (0..n).map(|i| self.u32_at(base + i * 4)).collect()
    }

    fn bools(&self, base: usize, n: usize) -> Result<Vec<bool>> {
        Ok(self.u32s(base, n)?.iter().map(|&v| v != 0).collect())
    }

    fn bytes(&self, base: usize, n: usize) -> Result<&'a [u8]> {
        self.check(base, n)?;
        Ok(&self.data[base..base + n])
    }

    fn xys(&self, base: usize, n: usize) -> Result<Vec<[f32; 2]>> {
        self.check(base, n * 8)?;
        (0..n)
            .map(|i| Ok([self.f32_at(base + i * 8)?, self.f32_at(base + i * 8 + 4)?]))
            .collect()
    }

    fn i16s(&self, base: usize, n: usize) -> Result<Vec<i16>> {
        self.check(base, n * 2)?;
        (0..n).map(|i| self.i16_at(base + i * 2)).collect()
    }
}

impl Moc3 {
    pub fn parse(data: &[u8]) -> Result<Moc3> {
        if data.len() < 72 {
            return Err(Error::Truncated { at: 0, needed: 72 });
        }
        if &data[0..4] != MAGIC {
            return Err(Error::BadMagic);
        }
        let version = data[4];
        if version == 0 {
            return Err(Error::BadVersion(version));
        }
        let big_endian = data[5] != 0;
        let r = Reader { data, big_endian };

        let mut t = Table { r: &r, cur: 0x40 };
        let count_info = t.ptr()?;
        let canvas_info = t.ptr()?;

        // ---- Count info table: counts start at the pointer itself ----
        let names = 23;
        let raw: Vec<usize> = (0..names).map(|i| r.u32_at(count_info + i * 4).map(|v| v as usize)).collect::<Result<_>>()?;
        let cnt = Counts {
            parts: raw[0],
            deformers: raw[1],
            warp_deformers: raw[2],
            rotation_deformers: raw[3],
            art_meshes: raw[4],
            parameters: raw[5],
            part_keyforms: raw[6],
            warp_deformer_keyforms: raw[7],
            rotation_deformer_keyforms: raw[8],
            art_mesh_keyforms: raw[9],
            keyform_positions: raw[10],
            parameter_binding_indices: raw[11],
            keyform_bindings: raw[12],
            parameter_bindings: raw[13],
            keys: raw[14],
            uvs: raw[15],
            position_indices: raw[16],
            drawable_masks: raw[17],
            draw_order_groups: raw[18],
            draw_order_group_objects: raw[19],
            glue: raw[20],
            glue_info: raw[21],
            glue_keyforms: raw[22],
        };

        // ---- Canvas info ----
        let canvas = Canvas {
            pixels_per_unit: r.f32_at(canvas_info)?,
            origin_x: r.f32_at(canvas_info + 4)?,
            origin_y: r.f32_at(canvas_info + 8)?,
            width: r.f32_at(canvas_info + 12)?,
            height: r.f32_at(canvas_info + 16)?,
            reverse_y: (data[canvas_info + 20] & 1) != 0,
        };

        // ---- Parts: [runtime, ids, kbs, ksb, ksc, vis, en, pp] ----
        let parts = {
            let n = cnt.parts;
            t.skip(1);
            let ids = r.ids(t.ptr()?, n)?;
            let kbs = r.i32s(t.ptr()?, n)?;
            let ksb = r.i32s(t.ptr()?, n)?;
            let ksc = r.i32s(t.ptr()?, n)?;
            let vis = r.bools(t.ptr()?, n)?;
            let en = r.bools(t.ptr()?, n)?;
            let pp = r.i32s(t.ptr()?, n)?;
            (0..n)
                .map(|i| Part {
                    id: ids[i].clone(),
                    keyform_binding_source: kbs[i],
                    keyform_sources_begin: ksb[i],
                    keyform_sources_count: ksc[i],
                    visible: vis[i],
                    enabled: en[i],
                    parent_part: pp[i],
                })
                .collect::<Vec<_>>()
        };

        // ---- Deformers: [runtime, ids, kbs, vis, en, pp, pd, types, ssi] ----
        let deformers = {
            let n = cnt.deformers;
            t.skip(1);
            let ids = r.ids(t.ptr()?, n)?;
            let kbs = r.i32s(t.ptr()?, n)?;
            let vis = r.bools(t.ptr()?, n)?;
            let en = r.bools(t.ptr()?, n)?;
            let pp = r.i32s(t.ptr()?, n)?;
            let pd = r.i32s(t.ptr()?, n)?;
            let tys = r.u32s(t.ptr()?, n)?;
            let ssi = r.i32s(t.ptr()?, n)?;
            (0..n)
                .map(|i| Deformer {
                    id: ids[i].clone(),
                    keyform_binding_source: kbs[i],
                    visible: vis[i],
                    enabled: en[i],
                    parent_part: pp[i],
                    parent_deformer: pd[i],
                    ty: match tys[i] {
                        0 => DeformerType::Warp,
                        1 => DeformerType::Rotation,
                        other => DeformerType::Unknown(other),
                    },
                    specific_index: ssi[i],
                })
                .collect::<Vec<_>>()
        };

        // ---- Warp deformers: [kbs, ksb, ksc, vc, rows, cols] ----
        let warp_deformers = {
            let n = cnt.warp_deformers;
            let kbs = r.i32s(t.ptr()?, n)?;
            let ksb = r.i32s(t.ptr()?, n)?;
            let ksc = r.i32s(t.ptr()?, n)?;
            let vc = r.i32s(t.ptr()?, n)?;
            let rows = r.u32s(t.ptr()?, n)?;
            let cols = r.u32s(t.ptr()?, n)?;
            (0..n)
                .map(|i| WarpDeformer {
                    keyform_binding_source: kbs[i],
                    keyform_sources_begin: ksb[i],
                    keyform_sources_count: ksc[i],
                    vertex_count: vc[i],
                    rows: rows[i],
                    columns: cols[i],
                })
                .collect::<Vec<_>>()
        };

        // ---- Rotation deformers: [kbs, ksb, ksc, baseAngle] ----
        let rotation_deformers = {
            let n = cnt.rotation_deformers;
            let kbs = r.i32s(t.ptr()?, n)?;
            let ksb = r.i32s(t.ptr()?, n)?;
            let ksc = r.i32s(t.ptr()?, n)?;
            let ba = r.f32s(t.ptr()?, n)?;
            (0..n)
                .map(|i| RotationDeformer {
                    keyform_binding_source: kbs[i],
                    keyform_sources_begin: ksb[i],
                    keyform_sources_count: ksc[i],
                    base_angle: ba[i],
                })
                .collect::<Vec<_>>()
        };

        // ---- Art meshes: [rt0..3, ids, kbs, ksb, ksc, vis, en, pp, pd, tex, flags, vc, uvb, pib, pic, mb, mc] ----
        let art_meshes = {
            let n = cnt.art_meshes;
            t.skip(4);
            let ids = r.ids(t.ptr()?, n)?;
            let kbs = r.i32s(t.ptr()?, n)?;
            let ksb = r.i32s(t.ptr()?, n)?;
            let ksc = r.i32s(t.ptr()?, n)?;
            let vis = r.bools(t.ptr()?, n)?;
            let en = r.bools(t.ptr()?, n)?;
            let pp = r.i32s(t.ptr()?, n)?;
            let pd = r.i32s(t.ptr()?, n)?;
            let tex = r.u32s(t.ptr()?, n)?;
            let flags = r.bytes(t.ptr()?, n)?;
            let vc = r.i32s(t.ptr()?, n)?;
            let uvb = r.i32s(t.ptr()?, n)?;
            let pib = r.i32s(t.ptr()?, n)?;
            let pic = r.i32s(t.ptr()?, n)?;
            let mb = r.i32s(t.ptr()?, n)?;
            let mc = r.i32s(t.ptr()?, n)?;
            (0..n)
                .map(|i| ArtMesh {
                    id: ids[i].clone(),
                    keyform_binding_source: kbs[i],
                    keyform_sources_begin: ksb[i],
                    keyform_sources_count: ksc[i],
                    visible: vis[i],
                    enabled: en[i],
                    parent_part: pp[i],
                    parent_deformer: pd[i],
                    texture_no: tex[i],
                    blend_mode: flags[i] & 0x3,
                    double_sided: (flags[i] >> 2) & 1 != 0,
                    inverted_mask: (flags[i] >> 3) & 1 != 0,
                    vertex_count: vc[i],
                    uv_begin: uvb[i],
                    position_index_begin: pib[i],
                    position_index_count: pic[i],
                    mask_begin: mb[i],
                    mask_count: mc[i],
                })
                .collect::<Vec<_>>()
        };

        // ---- Parameters: [runtime, ids, max, min, default, repeat, decimals, bsb, bsc] ----
        let parameters = {
            let n = cnt.parameters;
            t.skip(1);
            let ids = r.ids(t.ptr()?, n)?;
            let mx = r.f32s(t.ptr()?, n)?;
            let mn = r.f32s(t.ptr()?, n)?;
            let df = r.f32s(t.ptr()?, n)?;
            let rep = r.bools(t.ptr()?, n)?;
            let dp = r.u32s(t.ptr()?, n)?;
            let bb = r.i32s(t.ptr()?, n)?;
            let bc = r.i32s(t.ptr()?, n)?;
            (0..n)
                .map(|i| Parameter {
                    id: ids[i].clone(),
                    max: mx[i],
                    min: mn[i],
                    default: df[i],
                    repeat: rep[i],
                    decimal_places: dp[i],
                    binding_sources_begin: bb[i],
                    binding_sources_count: bc[i],
                })
                .collect::<Vec<_>>()
        };

        // ---- Part keyforms: [drawOrders] ----
        let part_keyforms = {
            let n = cnt.part_keyforms;
            let dos = r.f32s(t.ptr()?, n)?;
            dos.iter().map(|&d| PartKeyform { draw_order: d }).collect::<Vec<_>>()
        };

        // ---- Warp deformer keyforms: [opacities, kfPosBegin] ----
        let warp_deformer_keyforms = {
            let n = cnt.warp_deformer_keyforms;
            let ops = r.f32s(t.ptr()?, n)?;
            let kpb = r.i32s(t.ptr()?, n)?;
            (0..n)
                .map(|i| WarpDeformerKeyform { opacity: ops[i], position_sources_begin: kpb[i] })
                .collect::<Vec<_>>()
        };

        // ---- Rotation deformer keyforms: [opacities, angles, ox, oy, scales, rx, ry] ----
        let rotation_deformer_keyforms = {
            let n = cnt.rotation_deformer_keyforms;
            let ops = r.f32s(t.ptr()?, n)?;
            let ang = r.f32s(t.ptr()?, n)?;
            let ox = r.f32s(t.ptr()?, n)?;
            let oy = r.f32s(t.ptr()?, n)?;
            let sc = r.f32s(t.ptr()?, n)?;
            let rx = r.bools(t.ptr()?, n)?;
            let ry = r.bools(t.ptr()?, n)?;
            (0..n)
                .map(|i| RotationDeformerKeyform {
                    opacity: ops[i],
                    angle: ang[i],
                    origin_x: ox[i],
                    origin_y: oy[i],
                    scale: sc[i],
                    reflect_x: rx[i],
                    reflect_y: ry[i],
                })
                .collect::<Vec<_>>()
        };

        // ---- Art mesh keyforms: [opacities, drawOrders, kfPosBegin] ----
        let art_mesh_keyforms = {
            let n = cnt.art_mesh_keyforms;
            let ops = r.f32s(t.ptr()?, n)?;
            let dos = r.f32s(t.ptr()?, n)?;
            let kpb = r.i32s(t.ptr()?, n)?;
            (0..n)
                .map(|i| ArtMeshKeyform {
                    opacity: ops[i],
                    draw_order: dos[i],
                    position_sources_begin: kpb[i],
                })
                .collect::<Vec<_>>()
        };

        // ---- Keyform positions: XY pool (count is in floats) ----
        let keyform_positions = {
            let base = t.ptr()?;
            let n = cnt.keyform_positions;
            // Try N/2 XYs first; fall back to N XYs if the count was already
            // in elements (some exporters differ). Validate against usage
            // later in Model construction.
            let half = n / 2;
            if half > 0 && base + half * 8 <= data.len() {
                r.xys(base, half)?
            } else {
                r.xys(base, n)?
            }
        };

        // ---- Parameter binding indices ----
        let parameter_binding_indices = r.i32s(t.ptr()?, cnt.parameter_binding_indices)?;

        // ---- Keyform bindings: [pbiBegin, pbiCount] ----
        let keyform_bindings = {
            let n = cnt.keyform_bindings;
            let bb = r.i32s(t.ptr()?, n)?;
            let bc = r.i32s(t.ptr()?, n)?;
            (0..n)
                .map(|i| KeyformBinding {
                    parameter_binding_index_begin: bb[i],
                    parameter_binding_index_count: bc[i],
                })
                .collect::<Vec<_>>()
        };

        // ---- Parameter bindings: [keysBegin, keysCount] ----
        let parameter_bindings = {
            let n = cnt.parameter_bindings;
            let kb = r.i32s(t.ptr()?, n)?;
            let kc = r.i32s(t.ptr()?, n)?;
            (0..n)
                .map(|i| ParameterBinding { keys_begin: kb[i], keys_count: kc[i] })
                .collect::<Vec<_>>()
        };

        // ---- Keys ----
        let keys = r.f32s(t.ptr()?, cnt.keys)?;

        // ---- UVs (count is in floats) ----
        let uvs = {
            let base = t.ptr()?;
            let n = cnt.uvs;
            let half = n / 2;
            if half > 0 && base + half * 8 <= data.len() {
                r.xys(base, half)?
            } else {
                r.xys(base, n)?
            }
        };

        // ---- Position indices ----
        let position_indices = r
            .i16s(t.ptr()?, cnt.position_indices)?
            .into_iter()
            .map(|v| v as u16)
            .collect();

        // ---- Drawable masks ----
        let drawable_masks = r.i32s(t.ptr()?, cnt.drawable_masks)?;
        let mut offscreen_out_parts = vec![false; cnt.parts];
        if version >= 6 {
            let slot = |s: usize| r.u32_at(0x40 + s * 4).map(|v| v as usize);
            if let (Ok(pi), Ok(bl)) = (slot(149), slot(157)) {
                if let Ok(idx) = r.i32s(pi, cnt.parts) {
                    let n = idx.iter().copied().max().unwrap_or(-1) + 1;
                    if let Ok(blend) = r.i32s(bl, n.max(0) as usize) {
                        for (p, &o) in idx.iter().enumerate() {
                            if o >= 0 && (blend[o as usize] >> 8) & 0xFF == 2 {
                                offscreen_out_parts[p] = true;
                            }
                        }
                    }
                }
            }
        }

        // ---- Draw order groups: [objBegin, objCount, objTotal, maxDO, minDO] ----
        let draw_order_groups = {
            let n = cnt.draw_order_groups;
            let ob = r.i32s(t.ptr()?, n)?;
            let oc = r.i32s(t.ptr()?, n)?;
            let otc = r.i32s(t.ptr()?, n)?;
            let mx = r.u32s(t.ptr()?, n)?;
            let mn = r.u32s(t.ptr()?, n)?;
            (0..n)
                .map(|i| DrawOrderGroup {
                    objects_begin: ob[i],
                    objects_count: oc[i],
                    objects_total_count: otc[i],
                    max_draw_order: mx[i],
                    min_draw_order: mn[i],
                })
                .collect::<Vec<_>>()
        };

        // ---- Draw order group objects: [types, indices, selfIndices] ----
        let draw_order_group_objects = {
            let n = cnt.draw_order_group_objects;
            let tys = r.u32s(t.ptr()?, n)?;
            let idx = r.i32s(t.ptr()?, n)?;
            let sidx = r.i32s(t.ptr()?, n)?;
            (0..n)
                .map(|i| DrawOrderGroupObject {
                    ty: match tys[i] {
                        0 => DrawOrderObjectType::ArtMesh,
                        1 => DrawOrderObjectType::Part,
                        other => DrawOrderObjectType::Unknown(other),
                    },
                    index: idx[i],
                    self_index: sidx[i],
                })
                .collect::<Vec<_>>()
        };

        // ---- Glue sections: skip (9 + 2 + 1 pointers) ----
        t.skip(9);
        t.skip(2);
        t.skip(1);

        // ---- v3.03+: warp deformer quad-source flags ----
        let warp_deformer_quad_source = if version >= 2 {
            r.bools(t.ptr()?, cnt.warp_deformers)?
        } else {
            vec![false; cnt.warp_deformers]
        };

        Ok(Moc3 {
            version,
            canvas,
            parts,
            deformers,
            warp_deformers,
            rotation_deformers,
            art_meshes,
            parameters,
            part_keyforms,
            warp_deformer_keyforms,
            rotation_deformer_keyforms,
            art_mesh_keyforms,
            keyform_positions,
            parameter_binding_indices,
            keyform_bindings,
            parameter_bindings,
            keys,
            uvs,
            position_indices,
            drawable_masks,
            draw_order_groups,
            draw_order_group_objects,
            warp_deformer_quad_source,
            offscreen_out_parts,
        })
    }
}
