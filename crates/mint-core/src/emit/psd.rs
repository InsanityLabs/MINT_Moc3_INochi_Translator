//! Layered PSD writer.
//!
//! Produces PSD v1, RGB/8-bit, with real layers, groups, alpha
//! channels and PackBits RLE compression — equivalent to what
//! pytoshop produces, implemented from the published file spec.

use super::{LayerGroup, LayerNode};
use crate::raster::Blend;
use image::RgbaImage;
use std::io::Write;

fn blend_key(b: Blend) -> &'static [u8; 4] {
    match b {
        Blend::Normal => b"norm",
        Blend::Add => b"lddg", // linear dodge (additive)
        Blend::Multiply | Blend::MultiplyNew => b"mul ",
        Blend::Darken => b"dark",
        Blend::ColorBurn => b"idiv",
        Blend::LinearBurn => b"lbrn",
        Blend::Lighten => b"lite",
        Blend::Screen => b"scrn",
        Blend::ColorDodge => b"div ",
        Blend::Overlay => b"over",
        Blend::SoftLight => b"sLit",
        Blend::HardLight => b"hLit",
        Blend::LinearLight => b"vLit",
    }
}

/// PackBits-RLE encode one channel row-by-row: returns (row_lengths, data).
fn rle_channel(data: &[u8], width: usize, height: usize) -> (Vec<u16>, Vec<u8>) {
    let mut lengths = Vec::with_capacity(height);
    let mut out = Vec::new();
    for y in 0..height {
        let row = &data[y * width..(y + 1) * width];
        let start = out.len();
        packbits(row, &mut out);
        lengths.push((out.len() - start) as u16);
    }
    (lengths, out)
}

fn packbits(row: &[u8], out: &mut Vec<u8>) {
    let n = row.len();
    let mut i = 0;
    while i < n {
        // run of repeats?
        let mut run = 1;
        while i + run < n && run < 128 && row[i + run] == row[i] {
            run += 1;
        }
        if run >= 3 {
            out.push((257u16 - run as u16) as u8);
            out.push(row[i]);
            i += run;
            continue;
        }
        // literal run
        let lit_start = i;
        i += run;
        while i < n {
            let mut r = 1;
            while i + r < n && r < 128 && row[i + r] == row[i] {
                r += 1;
            }
            if r >= 3 || i - lit_start >= 128 {
                break;
            }
            i += r.min(1);
        }
        let lit = &row[lit_start..i.min(lit_start + 128)];
        out.push((lit.len() - 1) as u8);
        out.extend_from_slice(lit);
        i = lit_start + lit.len();
    }
}

enum Rec<'a> {
    /// group folder record (type=1 for open, type=2 for closed)
    Group(&'a str, bool, (i32, i32, i32, i32), bool), // name, is_open, bounding box, visible
    Image(&'a crate::emit::LayerImage),
}

fn flatten<'a>(group: &'a LayerGroup, out: &mut Vec<Rec<'a>>) {
    // children are stored bottom-to-top.
    // PSD group encoding (file order): [children...] then folder record,
    // then bounding divider — see comment in write_psd.
    for child in &group.children {
        match child {
            LayerNode::Image(li) => out.push(Rec::Image(li)),
            LayerNode::Group(g) => flatten(g, out),
        }
    }
}

pub fn write_psd<W: Write>(
    root: &LayerGroup,
    merged: &RgbaImage,
    w: u32,
    h: u32,
    mut out: W,
) -> std::io::Result<()> {
    // ---- build flat record list ----
    // File order: layers stored topmost-first. For a group:
    //   [bounding divider (type=3, name "</Layer group>")]
    //   [children, top-first]
    //   [folder record (name, type=1 open)]
    // Wait — actual convention (Adobe spec + psd-tools):
    //   the layer array lists records TOP-first; a group is encoded as
    //   [folder record][children top-first][bounding divider].
    // We flatten bottom-to-top internally, so emit accordingly.
    let mut recs: Vec<Rec> = Vec::new();
    fn walk<'a>(group: &'a LayerGroup, out: &mut Vec<Rec<'a>>) -> (i32, i32, i32, i32) {
        // Returns bounding box (left, top, right, bottom) of this group
        let mut bbox = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        // PSD stores layer records bottom-most first — our children are
        // already in bottom-to-top order, so emit them as-is.
        for child in group.children.iter() {
            match child {
                LayerNode::Image(li) => {
                    out.push(Rec::Image(li));
                    bbox.0 = bbox.0.min(li.left);
                    bbox.1 = bbox.1.min(li.top);
                    bbox.2 = bbox.2.max(li.left + li.image.width() as i32);
                    bbox.3 = bbox.3.max(li.top + li.image.height() as i32);
                }
                LayerNode::Group(g) => {
                    // PSD group encoding (file order, topmost-first):
                    //   [bounding divider (type=3)]
                    //   [children, top-first]
                    //   [folder record (type=1 open)]
                    out.push(Rec::Group("</Layer group>", false, (0, 0, 0, 0), true));
                    let child_bbox = walk(g, out);
                    bbox.0 = bbox.0.min(child_bbox.0);
                    bbox.1 = bbox.1.min(child_bbox.1);
                    bbox.2 = bbox.2.max(child_bbox.2);
                    bbox.3 = bbox.3.max(child_bbox.3);
                    out.push(Rec::Group(&g.name, true, child_bbox, g.visible));
                }
            }
        }
        // If no children, use (0,0,0,0)
        if bbox.0 == i32::MAX {
            (0, 0, 0, 0)
        } else {
            bbox
        }
    }
    let _ = flatten;
    let _root_bbox = walk(root, &mut recs);

    // ---- header ----
    out.write_all(b"8BPS")?;
    out.write_all(&1u16.to_be_bytes())?; // version
    out.write_all(&[0u8; 6])?;
    out.write_all(&4u16.to_be_bytes())?; // channels in composite (RGBA)
    out.write_all(&h.to_be_bytes())?; // rows (height) first per spec
    out.write_all(&w.to_be_bytes())?; // columns (width)
    out.write_all(&8u16.to_be_bytes())?; // depth
    out.write_all(&3u16.to_be_bytes())?; // RGB

    // ---- color mode data + image resources ----
    out.write_all(&0u32.to_be_bytes())?;
    let mut res = Vec::new();
    let n_recs = recs.len();
    let mut resource = |id: u16, data: &[u8]| {
        res.extend_from_slice(b"8BIM");
        res.extend_from_slice(&id.to_be_bytes());
        res.extend_from_slice(&[0u8, 0u8]); // empty pascal name, padded
        res.extend_from_slice(&(data.len() as u32).to_be_bytes());
        res.extend_from_slice(data);
        if data.len() % 2 == 1 {
            res.push(0);
        }
    };
    // 1005 ResolutionInfo: 72 dpi
    resource(1005, &[0x00, 0x48, 0, 0, 0, 1, 0, 1, 0x00, 0x48, 0, 0, 0, 1, 0, 1]);
    // 1024 layer state (active layer), 1026 layer group ids, 1072 group-enabled flags
    resource(1024, &0u16.to_be_bytes());
    resource(1026, &vec![0u8; n_recs * 2]);
    resource(1072, &vec![1u8; n_recs]);
    out.write_all(&(res.len() as u32).to_be_bytes())?;
    out.write_all(&res)?;

    // ---- layer records + channel data ----
    let mut rec_bytes = Vec::new();
    let mut chan_bytes = Vec::new();
    let n_layers = recs.len() as i16;
    for (rec_idx, rec) in recs.iter().enumerate() {
        let layer_id = rec_idx as u32 + 2;
        match rec {
            Rec::Group(..) => {
                write_group_record(rec, &mut rec_bytes, &mut chan_bytes, layer_id)?;
            }
            Rec::Image(li) => {
                write_image_record(li, &mut rec_bytes, &mut chan_bytes, layer_id)?;
            }
        }
    }

    let mut layer_info = Vec::new();
    layer_info.extend_from_slice(&(-n_layers).to_be_bytes()); // neg = alpha is transparency
    layer_info.extend_from_slice(&rec_bytes);
    layer_info.extend_from_slice(&chan_bytes);
    // pad to even
    while layer_info.len() % 2 != 0 {
        layer_info.push(0);
    }

    // global layer mask info (none)
    let mut section = Vec::new();
    section.extend_from_slice(&(layer_info.len() as u32).to_be_bytes());
    section.extend_from_slice(&layer_info);
    section.extend_from_slice(&0u32.to_be_bytes()); // global layer mask info
    // pad section to even
    while section.len() % 2 != 0 {
        section.push(0);
    }
    out.write_all(&(section.len() as u32).to_be_bytes())?;
    out.write_all(&section)?;

    // ---- composite image (merged) ----
    // Spec: all row byte-counts for ALL channels first, then all pixel data.
    out.write_all(&1u16.to_be_bytes())?; // RLE
    let (mw, mh) = (merged.width() as usize, merged.height() as usize);
    let raw = merged.as_raw();
    let mut all_lens = Vec::with_capacity(4 * mh);
    let mut all_data = Vec::new();
    for c in 0..4 {
        let mut chan = vec![0u8; mw * mh];
        for (i, px) in raw.chunks_exact(4).enumerate() {
            chan[i] = px[c];
        }
        let (lens, data) = rle_channel(&chan, mw, mh);
        all_lens.extend(lens);
        all_data.extend(data);
    }
    for l in &all_lens {
        out.write_all(&l.to_be_bytes())?;
    }
    out.write_all(&all_data)?;

    Ok(())
}

fn pad4(v: &mut Vec<u8>) {
    while v.len() % 4 != 0 {
        v.push(0);
    }
}

fn pascal_name(name: &str, extra: &mut Vec<u8>) {
    let bytes = name.as_bytes();
    let len = bytes.len().min(255);
    extra.push(len as u8);
    extra.extend_from_slice(&bytes[..len]);
    // pad to multiple of 4 total (including len byte)
    pad4(extra);
}

fn luni(name: &str, extra: &mut Vec<u8>) {
    let utf16: Vec<u16> = name.encode_utf16().collect();
    let mut body = Vec::with_capacity(4 + utf16.len() * 2 + 2);
    body.extend_from_slice(&(utf16.len() as u32).to_be_bytes());
    for u in &utf16 {
        body.extend_from_slice(&u.to_be_bytes());
    }
    // the declared block length includes its padding (multiple of 4)
    pad4(&mut body);
    extra.extend_from_slice(b"8BIM");
    extra.extend_from_slice(b"luni");
    extra.extend_from_slice(&(body.len() as u32).to_be_bytes());
    extra.extend_from_slice(&body);
}

/// Default (full-range) layer blending ranges, as Photoshop writes them.
fn blending_ranges(extra: &mut Vec<u8>) {
    extra.extend_from_slice(&40u32.to_be_bytes());
    for _ in 0..10 {
        extra.extend_from_slice(&[0x00, 0x00, 0xFF, 0xFF]);
    }
}

fn tagged(key: &[u8; 4], data: &[u8], extra: &mut Vec<u8>) {
    extra.extend_from_slice(b"8BIM");
    extra.extend_from_slice(key);
    let mut body = data.to_vec();
    pad4(&mut body);
    extra.extend_from_slice(&(body.len() as u32).to_be_bytes());
    extra.extend_from_slice(&body);
}

fn lsct(kind: u32, extra: &mut Vec<u8>) {
    let mut body = Vec::with_capacity(16);
    body.extend_from_slice(&kind.to_be_bytes());
    body.extend_from_slice(b"8BIM");
    body.extend_from_slice(b"norm");
    body.extend_from_slice(&0u32.to_be_bytes());
    tagged(b"lsct", &body, extra);
}

fn write_group_record(rec: &Rec<'_>, out: &mut Vec<u8>, chan: &mut Vec<u8>, layer_id: u32) -> std::io::Result<()> {
    let (name, kind, bbox, visible) = match rec {
        Rec::Group(n, is_open, bbox, vis) => (*n, if *is_open { 1u32 } else { 3u32 }, *bbox, *vis),
        _ => unreachable!(),
    };
    // Group records carry no pixels, so (like Photoshop) their rect is empty.
    let _ = bbox;
    for v in [0i32, 0, 0, 0] {
        out.write_all(&v.to_be_bytes())?;
    }
    // Photoshop gives group records 4 empty channels (just the 2-byte
    // compression marker each).
    out.write_all(&4u16.to_be_bytes())?;
    for id in [-1i16, 0, 1, 2] {
        out.write_all(&id.to_be_bytes())?;
        out.write_all(&2u32.to_be_bytes())?;
        chan.extend_from_slice(&0u16.to_be_bytes());
    }
    out.write_all(b"8BIM")?;
    out.write_all(b"norm")?;
    let flags = 0x18u8 | if visible { 0 } else { 0x02 };
    out.write_all(&[255u8, 0, flags, 0])?; // opacity, clipping, flags, filler
    let mut extra = Vec::new();
    extra.extend_from_slice(&0u32.to_be_bytes()); // mask data
    blending_ranges(&mut extra);
    pascal_name(name, &mut extra);
    luni(name, &mut extra);
    tagged(b"lyid", &layer_id.to_be_bytes(), &mut extra);
    lsct(kind, &mut extra);
    tagged(b"lspf", &0u32.to_be_bytes(), &mut extra);
    tagged(b"lclr", &[0u8; 8], &mut extra);
    out.write_all(&(extra.len() as u32).to_be_bytes())?;
    out.write_all(&extra)?;
    Ok(())
}

fn write_image_record(
    li: &crate::emit::LayerImage,
    rec: &mut Vec<u8>,
    chan: &mut Vec<u8>,
    layer_id: u32,
) -> std::io::Result<()> {
    let (lw, lh) = (li.image.width() as usize, li.image.height() as usize);
    let (top, left) = (li.top, li.left);
    let (bottom, right) = (top + lh as i32, left + lw as i32);
    for v in [top, left, bottom, right] {
        rec.write_all(&v.to_be_bytes())?;
    }
    // 4 channels: -1 alpha, 0 R, 1 G, 2 B (+ -2 user layer mask)
    let n_chan = if li.mask.is_some() { 5usize } else { 4 };
    rec.write_all(&(n_chan as u16).to_be_bytes())?;
    // reserve space for channel lengths
    let len_pos = rec.len();
    rec.write_all(&vec![0u8; n_chan * (2 + 4)])?; // i16 id + u32 len each

    rec.write_all(b"8BIM")?;
    rec.write_all(blend_key(li.blend))?;
    rec.write_all(&[(li.opacity * 255.0) as u8, 0u8, 0x08 | if li.visible { 0u8 } else { 0x02 }, 0u8])?;

    // channel data first (to know lengths): de-interleave + RLE
    let raw = li.image.as_raw();
    let mut chan_data: Vec<Vec<u8>> = Vec::with_capacity(4);
    let mut row_counts: Vec<Vec<u16>> = Vec::with_capacity(4);
    for c in 0..4 {
        let mut plane = vec![0u8; lw * lh];
        // channel order in file: alpha first? PSD channel id -1 = transparency
        let src = if c == 0 { 3 } else { c - 1 }; // emit alpha,R,G,B
        for (i, px) in raw.chunks_exact(4).enumerate() {
            plane[i] = px[src];
        }
        let (lens, data) = rle_channel(&plane, lw, lh);
        row_counts.push(lens);
        chan_data.push(data);
    }
    // write per-channel compressed blocks
    let mut blocks = Vec::with_capacity(5);
    for c in 0..4 {
        let mut b = Vec::with_capacity(2 + row_counts[c].len() * 2 + chan_data[c].len());
        b.extend_from_slice(&1u16.to_be_bytes()); // RLE
        for l in &row_counts[c] {
            b.extend_from_slice(&l.to_be_bytes());
        }
        b.extend_from_slice(&chan_data[c]);
        // pad channel data to even
        while b.len() % 2 != 0 {
            b.push(0);
        }
        blocks.push(b);
    }
    if let Some(m) = &li.mask {
        let (lens, data) = rle_channel(m.as_raw(), lw, lh);
        let mut b = Vec::with_capacity(2 + lens.len() * 2 + data.len());
        b.extend_from_slice(&1u16.to_be_bytes());
        for l in &lens {
            b.extend_from_slice(&l.to_be_bytes());
        }
        b.extend_from_slice(&data);
        while b.len() % 2 != 0 {
            b.push(0);
        }
        blocks.push(b);
    }
    for (i, b) in blocks.iter().enumerate() {
        chan.extend_from_slice(b);
        // fill in channel info: id + length
        let id: i16 = if i == 0 { -1 } else if i == 4 { -2 } else { (i - 1) as i16 };
        let pos = len_pos + i * 6;
        rec[pos..pos + 2].copy_from_slice(&id.to_be_bytes());
        rec[pos + 2..pos + 6].copy_from_slice(&(b.len() as u32).to_be_bytes());
    }

    // extra data
    let mut extra = Vec::new();
    if li.mask.is_some() {
        // layer mask data: rect (top,left,bottom,right), default colour, flags, padding
        extra.extend_from_slice(&20u32.to_be_bytes());
        for v in [top, left, bottom, right] {
            extra.extend_from_slice(&v.to_be_bytes());
        }
        extra.extend_from_slice(&[0u8, 0u8, 0u8, 0u8]);
    } else {
        extra.extend_from_slice(&0u32.to_be_bytes());
    }
    blending_ranges(&mut extra);
    pascal_name(&li.name, &mut extra);
    luni(&li.name, &mut extra);
    tagged(b"lyid", &layer_id.to_be_bytes(), &mut extra);
    tagged(b"clbl", &[1, 0, 0, 0], &mut extra);
    tagged(b"infx", &[0, 0, 0, 0], &mut extra);
    tagged(b"knko", &[0, 0, 0, 0], &mut extra);
    tagged(b"lspf", &0u32.to_be_bytes(), &mut extra);
    tagged(b"lclr", &[0u8; 8], &mut extra);
    rec.write_all(&(extra.len() as u32).to_be_bytes())?;
    rec.write_all(&extra)?;
    Ok(())
}
