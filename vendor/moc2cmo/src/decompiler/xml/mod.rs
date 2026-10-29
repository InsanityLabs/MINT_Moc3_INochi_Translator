mod constants;
mod document;
mod entities;
mod grid;
mod plan;
mod texture;
mod writer;

use crate::{Result, moc3::Moc3Model};

use self::{
    constants::{IMPORT_INSTRUCTIONS, VERSION_INSTRUCTIONS},
    plan::ProjectPlan,
    writer::{XmlWriter, attr},
};
use super::{DisplayNames, Texture, geometry::EvaluatedGeometry};

pub(super) fn generate(
    model: &Moc3Model,
    model_name: &str,
    textures: &[Texture],
    deformer_parts: &[Option<usize>],
    draw_ranks: &[i32],
    names: &DisplayNames,
    icons: &[(u32, Vec<u8>)],
) -> Result<Vec<u8>> {
    let plan = ProjectPlan::new(model, textures.len(), deformer_parts, draw_ranks, names.clone());
    let geometry = EvaluatedGeometry::evaluate(model, textures)?;
    let mut xml = XmlWriter::new();
    xml.declaration("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    for (name, version) in VERSION_INSTRUCTIONS {
        xml.declaration(&format!("<?version {name}:{version}?>"));
    }
    for import in IMPORT_INSTRUCTIONS {
        xml.declaration(&format!("<?import {import}?>"));
    }
    xml.start("root", &[attr("fileFormatVersion", "503040001")]);
    xml.start("shared", &[]);
    entities::write_shared(&mut xml, &plan, model, &geometry)?;
    texture::write_texture_shared(&mut xml, &plan, model, textures);
    xml.end("shared");
    xml.start("main", &[]);
    document::write_main(&mut xml, &plan, model, model_name, icons);
    xml.end("main");
    xml.end("root");
    Ok(xml.finish())
}
