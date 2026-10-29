use uuid::Uuid;

use crate::moc3::Moc3Model;

use super::super::{
    constants::{DEFORMER_ROOT_UUID, PARAMETER_GROUP_ROOT_UUID},
    grid::{empty_shared, reference, reference_without_name, start_shared},
    plan::{GridPlan, ProjectPlan, RefId},
    writer::{XmlWriter, attr},
};

pub(super) fn write_identifiers(xml: &mut XmlWriter, plan: &ProjectPlan, model: &Moc3Model) {
    guid(
        xml,
        "CParameterGroupGuid",
        plan.parameter_group_guid,
        PARAMETER_GROUP_ROOT_UUID,
        "Parameter root",
    );
    guid(
        xml,
        "CModelGuid",
        plan.model_guid,
        &Uuid::new_v4().to_string(),
        "Decompiled model",
    );
    guid(
        xml,
        "CDeformerGuid",
        plan.root_deformer_guid,
        DEFORMER_ROOT_UUID,
        "ROOT",
    );

    start_shared(xml, "CoordType", plan.coord_type);
    xml.text("s", &[attr("xs.n", "coordName")], "DeformerLocal");
    xml.end("CoordType");
    start_shared(xml, "CoordType", plan.canvas_coord_type);
    xml.text("s", &[attr("xs.n", "coordName")], "Canvas");
    xml.end("CoordType");

    start_shared(xml, "CBlend_Normal", plan.blend_normal);
    xml.start("ACBlend", &[attr("xs.n", "super")]);
    xml.text("s", &[attr("xs.n", "displayName")], "Normal");
    xml.end("ACBlend");
    xml.end("CBlend_Normal");

    for (parameter, parameter_plan) in model.parameters().iter().zip(&plan.parameters) {
        random_guid(xml, "CParameterGuid", parameter_plan.guid, parameter.id());
    }

    random_guid(xml, "CPartGuid", plan.root_part.guid, "__RootPart__");
    write_form_guids(xml, &plan.root_part.grid, "Root Part");
    for (part, part_plan) in model.parts().iter().zip(&plan.parts) {
        random_guid(xml, "CPartGuid", part_plan.guid, part.id());
        write_form_guids(xml, &part_plan.grid, part.id());
    }
    for (index, deformer_plan) in plan.deformers.iter().enumerate() {
        random_guid(
            xml,
            "CDeformerGuid",
            deformer_plan.guid,
            &format!("Deformer{index}"),
        );
        write_form_guids(xml, &deformer_plan.grid, &format!("Deformer{index}"));
    }
    for (mesh, mesh_plan) in model.art_meshes().iter().zip(&plan.meshes) {
        random_guid(xml, "CDrawableGuid", mesh_plan.guid, mesh.id());
        random_guid(
            xml,
            "GEditableMeshGuid",
            mesh_plan.editable_mesh_guid,
            mesh.id(),
        );
        random_guid(
            xml,
            "CExtensionGuid",
            mesh_plan.editable_extension_guid,
            &format!("{} editable mesh", mesh.id()),
        );
        random_guid(
            xml,
            "CExtensionGuid",
            mesh_plan.generator_extension_guid,
            &format!("{} mesh generator", mesh.id()),
        );
        random_guid(
            xml,
            "CExtensionGuid",
            mesh_plan.texture_extension_guid,
            &format!("{} texture input", mesh.id()),
        );
        write_form_guids(xml, &mesh_plan.grid, mesh.id());
    }
    for (index, glue_plan) in plan.glues.iter().enumerate() {
        random_guid(
            xml,
            "CAffecterGuid",
            glue_plan.guid,
            &format!("Glue{index}"),
        );
        write_form_guids(xml, &glue_plan.grid, &format!("Glue{index}"));
    }
    for ((group_id, _, _), group_plan) in plan.names.parameter_groups.iter().zip(&plan.param_groups)
    {
        random_guid(xml, "CParameterGroupGuid", group_plan.guid, group_id);
    }
    write_param_group_entities(xml, plan, model);
}

/// Emits the `CParameterGroup` entities (root plus display-info groups) and
/// the `CParameterId`/`CParameterGroupId` shared entities the editor
/// references from parameter sources and the random-pose settings.
fn write_param_group_entities(xml: &mut XmlWriter, plan: &ProjectPlan, model: &Moc3Model) {
    write_group_entity(
        xml,
        plan,
        model,
        &plan.root_param_group,
        "Root Parameter Group",
        None,
        "",
    );
    for ((group_id, _, name), group_plan) in
        plan.names.parameter_groups.iter().zip(&plan.param_groups)
    {
        write_group_entity(
            xml,
            plan,
            model,
            group_plan,
            if name.is_empty() { group_id } else { name },
            Some(group_id),
            group_id,
        );
    }
    for (parameter, parameter_plan) in model.parameters().iter().zip(&plan.parameters) {
        empty_shared(
            xml,
            "CParameterId",
            parameter_plan.id_entity,
            vec![attr("idstr", parameter.id())],
        );
    }
    empty_shared(
        xml,
        "CParameterGroupId",
        plan.root_param_group.id_entity,
        vec![attr("idstr", "ParamGroupRoot")],
    );
    for ((group_id, _, _), group_plan) in plan.names.parameter_groups.iter().zip(&plan.param_groups)
    {
        empty_shared(
            xml,
            "CParameterGroupId",
            group_plan.id_entity,
            vec![attr("idstr", group_id)],
        );
    }
}

/// Writes one `CParameterGroup` entity. `group_id` is `None` for the root
/// group, otherwise the display-info group whose children it collects.
fn write_group_entity(
    xml: &mut XmlWriter,
    plan: &ProjectPlan,
    model: &Moc3Model,
    group: &super::super::plan::ParamGroupPlan,
    name: &str,
    group_id: Option<&str>,
    _id: &str,
) {
    start_shared(xml, "CParameterGroup", group.entity);
    xml.text("s", &[attr("xs.n", "name")], name);
    xml.empty("s", &[attr("xs.n", "description")]);
    xml.text("b", &[attr("xs.n", "folderIsOpened")], "false");
    reference(xml, "CParameterGroupGuid", "guid", group.guid);
    match group_id.and_then(|id| {
        plan.names
            .parameter_groups
            .iter()
            .zip(&plan.param_groups)
            .find(|((gid, _, _), _)| gid == id)
            .and_then(|((_, parent, _), _)| {
                if parent.is_empty() {
                    None
                } else {
                    plan.names
                        .parameter_groups
                        .iter()
                        .zip(&plan.param_groups)
                        .find(|((gid, _, _), _)| gid == parent)
                        .map(|(_, p)| p.guid)
                }
            })
    }) {
        Some(parent) => reference(xml, "CParameterGroupGuid", "parentGroupGuid", parent),
        None if group_id.is_some() => reference(
            xml,
            "CParameterGroupGuid",
            "parentGroupGuid",
            plan.parameter_group_guid,
        ),
        None => xml.empty("null", &[attr("xs.n", "parentGroupGuid")]),
    }
    let child_params: Vec<RefId> = model
        .parameters()
        .iter()
        .zip(&plan.parameters)
        .filter(|(p, _)| match group_id {
            None => plan
                .names
                .parameter_group_of
                .get(p.id())
                .is_none_or(|g| !plan.names.parameter_groups.iter().any(|(id, _, _)| id == g)),
            Some(id) => plan.names.parameter_group_of.get(p.id()).is_some_and(|g| g == id),
        })
        .map(|(_, p)| p.guid)
        .collect();
    let child_groups: Vec<RefId> = plan
        .names
        .parameter_groups
        .iter()
        .zip(&plan.param_groups)
        .filter(|((_, parent, _), _)| match group_id {
            None => parent.is_empty()
                || !plan
                    .names
                    .parameter_groups
                    .iter()
                    .any(|(id, _, _)| id == parent),
            Some(id) => parent == id,
        })
        .map(|(_, p)| p.guid)
        .collect();
    xml.start(
        "carray_list",
        &[
            attr("xs.n", "_childGuids"),
            attr("count", child_params.len() + child_groups.len()),
        ],
    );
    for guid in child_params {
        reference_without_name(xml, "CParameterGuid", guid);
    }
    for guid in child_groups {
        reference_without_name(xml, "CParameterGroupGuid", guid);
    }
    xml.end("carray_list");
    reference(xml, "CParameterGroupId", "id", group.id_entity);
    xml.start(
        "CLabelColor",
        &[attr("xs.n", "labelColor"), attr("customizedColorInt", "-1")],
    );
    xml.empty(
        "CLabelColorType",
        &[attr("xs.n", "labelType"), attr("v", "UNDEFINED")],
    );
    xml.end("CLabelColor");
    xml.end("CParameterGroup");
}

pub(super) fn start_parameter_controllable(
    xml: &mut XmlWriter,
    local_name: &str,
    parent_part: Option<RefId>,
    grid: RefId,
) {
    start_parameter_controllable_vis(xml, local_name, parent_part, grid, true);
}

pub(super) fn start_parameter_controllable_vis(
    xml: &mut XmlWriter,
    local_name: &str,
    parent_part: Option<RefId>,
    grid: RefId,
    visible: bool,
) {
    xml.start("ACParameterControllableSource", &[attr("xs.n", "super")]);
    xml.text("s", &[attr("xs.n", "localName")], local_name);
    xml.text("b", &[attr("xs.n", "isVisible")], if visible { "true" } else { "false" });
    xml.text("b", &[attr("xs.n", "isLocked")], "false");
    if let Some(parent_part) = parent_part {
        reference(xml, "CPartGuid", "parentGuid", parent_part);
    } else {
        xml.empty("null", &[attr("xs.n", "parentGuid")]);
    }
    reference(xml, "KeyformGridSource", "keyformGridSource", grid);
    xml.start(
        "KeyFormMorphTargetSet",
        &[attr("xs.n", "keyformMorphTargetSet")],
    );
    xml.empty(
        "carray_list",
        &[attr("xs.n", "_morphTargets"), attr("count", 0)],
    );
    xml.start(
        "MorphTargetBlendWeightConstraintSet",
        &[attr("xs.n", "blendWeightConstraintSet")],
    );
    xml.empty(
        "carray_list",
        &[attr("xs.n", "_constraints"), attr("count", 0)],
    );
    xml.end("MorphTargetBlendWeightConstraintSet");
    xml.end("KeyFormMorphTargetSet");
}

pub(super) fn write_form_header(
    xml: &mut XmlWriter,
    form_guid: RefId,
    source_tag: &str,
    source: RefId,
) {
    xml.start("ACForm", &[attr("xs.n", "super")]);
    reference(xml, "CFormGuid", "guid", form_guid);
    xml.text("b", &[attr("xs.n", "isAnimatedForm")], "false");
    xml.text("b", &[attr("xs.n", "isLocalAnimatedForm")], "false");
    reference(xml, source_tag, "_source", source);
    xml.empty("null", &[attr("xs.n", "name")]);
    xml.empty("s", &[attr("xs.n", "notes")]);
    xml.end("ACForm");
}

pub(super) fn color(xml: &mut XmlWriter, name: &str, rgb: [f32; 3]) {
    xml.empty(
        "CFloatColor",
        &[
            attr("xs.n", name),
            attr("red", super::super::grid::float(rgb[0])),
            attr("green", super::super::grid::float(rgb[1])),
            attr("blue", super::super::grid::float(rgb[2])),
            attr("alpha", "1.0"),
        ],
    );
}

pub(super) fn form_position(model: &Moc3Model, position: [f32; 2], is_canvas: bool) -> [f32; 2] {
    if !is_canvas {
        return position;
    }
    let scale = model.canvas().pixels_per_unit();
    let origin = model.canvas().origin();
    [
        position[0] * scale + origin[0],
        position[1] * scale + origin[1],
    ]
}

pub(super) fn form_coord_type(plan: &ProjectPlan, is_canvas: bool) -> RefId {
    if is_canvas {
        plan.canvas_coord_type
    } else {
        plan.coord_type
    }
}

fn write_form_guids(xml: &mut XmlWriter, grid: &GridPlan, owner: &str) {
    for (index, form) in grid.forms.iter().copied().enumerate() {
        random_guid(xml, "CFormGuid", form, &format!("{owner} keyform {index}"));
    }
}

fn random_guid(xml: &mut XmlWriter, tag: &str, id: RefId, note: &str) {
    guid(xml, tag, id, &Uuid::new_v4().to_string(), note);
}

fn guid(xml: &mut XmlWriter, tag: &str, id: RefId, uuid: &str, note: &str) {
    empty_shared(xml, tag, id, vec![attr("uuid", uuid), attr("note", note)]);
}
