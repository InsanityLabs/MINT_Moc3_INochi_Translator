//! OpenRaster (.ora) writer — a zip container of layer PNGs with an
//! XML layer stack. Opens in Krita, GIMP, and Photoshop (via plugin).

use super::{LayerGroup, LayerNode};
use image::{codecs::png::PngEncoder, ImageEncoder, RgbaImage};
use std::io::{Seek, Write};

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

struct LayerFiles<'a> {
    xml: String,
    files: Vec<(String, &'a RgbaImage)>,
}

/// Serialize a group into stack.xml + file list. `order` must already be
/// top-to-bottom (ORA lists layers topmost first).
fn emit_group<'a>(group: &'a LayerGroup, depth: usize, counter: &mut usize, out: &mut LayerFiles<'a>) {
    // children stored bottom-to-top; emit top-first
    for child in group.children.iter().rev() {
        match child {
            LayerNode::Image(li) => {
                let fname = format!("data/layer{}.png", *counter);
                *counter += 1;
                out.xml.push_str(&format!(
                    "{}<layer name=\"{}\" src=\"{}\" x=\"{}\" y=\"{}\" opacity=\"{}\" />\n",
                    "  ".repeat(depth),
                    xml_escape(&li.name),
                    fname,
                    li.left,
                    li.top,
                    li.opacity,
                ));
                out.files.push((fname, &li.image));
            }
            LayerNode::Group(g) => {
                out.xml.push_str(&format!(
                    "{}<stack name=\"{}\">\n",
                    "  ".repeat(depth),
                    xml_escape(&g.name)
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
