//! OpenRaster (.ora) writer — a zip container of layer PNGs with an
//! XML layer stack. Opens in Krita, GIMP, and Photoshop (via plugin).

use super::{LayerGroup, LayerNode};
use image::{codecs::png::PngEncoder, ImageEncoder, RgbaImage};
use std::io::{Seek, Write};

fn composite_op(b: crate::raster::Blend) -> &'static str {
    use crate::raster::Blend::*;
    match b {
        Normal => "svg:src-over",
        Add => "svg:plus",
        Multiply | MultiplyNew => "svg:multiply",
        Darken => "svg:darken",
        ColorBurn => "svg:color-burn",
        LinearBurn => "krita:linear_burn",
        Lighten => "svg:lighten",
        Screen => "svg:screen",
        ColorDodge => "svg:color-dodge",
        Overlay => "svg:overlay",
        SoftLight => "svg:soft-light",
        HardLight => "svg:hard-light",
        LinearLight => "krita:linear_light",
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

struct LayerFiles {
    xml: String,
    files: Vec<(String, RgbaImage)>,
}

/// Serialize a group into stack.xml + file list. `order` must already be
/// top-to-bottom (ORA lists layers topmost first).
fn emit_group(group: &LayerGroup, depth: usize, counter: &mut usize, out: &mut LayerFiles) {
    // children stored bottom-to-top; emit top-first
    for child in group.children.iter().rev() {
        match child {
            LayerNode::Image(li) => {
                let fname = format!("data/layer{}.png", *counter);
                *counter += 1;
                out.xml.push_str(&format!(
                    "{}<layer name=\"{}\" src=\"{}\" x=\"{}\" y=\"{}\" opacity=\"{}\" visibility=\"{}\" composite-op=\"{}\" />\n",
                    "  ".repeat(depth),
                    xml_escape(&li.name),
                    fname,
                    li.left,
                    li.top,
                    li.opacity,
                    if li.visible { "visible" } else { "hidden" },
                    composite_op(li.blend),
                ));
                out.files.push((fname, li.image.clone()));
            }
            LayerNode::Group(g) => {
                out.xml.push_str(&format!(
                    "{}<stack name=\"{}\" visibility=\"{}\">\n",
                    "  ".repeat(depth),
                    xml_escape(&g.name),
                    if g.visible { "visible" } else { "hidden" },
                ));
                emit_group(g, depth + 1, counter, out);
                out.xml.push_str(&format!("{}</stack>\n", "  ".repeat(depth)));
            }
        }
    }
}

pub fn write_ora<W: Write + Seek>(
    root: &LayerGroup,
    merged: &RgbaImage,
    w: u32,
    h: u32,
    writer: W,
) -> std::io::Result<()> {
    let mut lf = LayerFiles {
        xml: String::new(),
        files: Vec::new(),
    };
    let mut counter = 0;
    emit_group(root, 1, &mut counter, &mut lf);
    // ORA has no layer masks: ship them as one hidden stack so the cut-out
    // shapes are not lost (name = "<layer>.mask", white where shown).
    let mut masks = String::new();
    fn collect_masks(g: &LayerGroup, counter: &mut usize, masks: &mut String, files: &mut Vec<(String, RgbaImage)>) {
        for child in g.children.iter().rev() {
            match child {
                LayerNode::Image(li) => {
                    if let Some(m) = &li.mask {
                        let img = RgbaImage::from_fn(m.width(), m.height(), |x, y| image::Rgba([255, 255, 255, m.get_pixel(x, y).0[0]]));
                        let fname = format!("data/mask{}.png", *counter);
                        *counter += 1;
                        masks.push_str(&format!(
                            "  <layer name=\"{}.mask\" src=\"{}\" x=\"{}\" y=\"{}\" opacity=\"1\" visibility=\"hidden\" />\n",
                            xml_escape(&li.name), fname, li.left, li.top
                        ));
                        files.push((fname, img));
                    }
                }
                LayerNode::Group(g) => collect_masks(g, counter, masks, files),
            }
        }
    }
    collect_masks(root, &mut counter, &mut masks, &mut lf.files);
    if !masks.is_empty() {
        lf.xml.push_str(&format!("  <stack name=\"Masks (hidden)\" visibility=\"hidden\">\n{}  </stack>\n", masks));
    }
    let stack_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<image w=\"{}\" h=\"{}\">\n <stack>\n{} </stack>\n</image>\n",
        w, h, lf.xml
    );

    let mut zip = zip::ZipWriter::new(writer);
    let stored = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("mimetype", stored)?;
    zip.write_all(b"image/openraster")?;

    zip.start_file("stack.xml", deflated)?;
    zip.write_all(stack_xml.as_bytes())?;

    // merged image
    let mut png_buf = Vec::new();
    PngEncoder::new(&mut png_buf)
        .write_image(merged.as_raw(), merged.width(), merged.height(), image::ExtendedColorType::Rgba8)
        .map_err(std::io::Error::other)?;
    zip.start_file("mergedimage.png", deflated)?;
    zip.write_all(&png_buf)?;

    for (name, img) in &lf.files {
        let mut buf = Vec::new();
        PngEncoder::new(&mut buf)
            .write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgba8)
            .map_err(std::io::Error::other)?;
        zip.start_file(name.clone(), deflated)?;
        zip.write_all(&buf)?;
    }

    zip.finish()?;
    Ok(())
}
