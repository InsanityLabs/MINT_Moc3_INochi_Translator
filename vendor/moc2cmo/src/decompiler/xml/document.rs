use crate::moc3::{Deformer, Moc3Model};

use super::{
    grid::{float, reference, reference_without_name},
    plan::ProjectPlan,
    writer::{XmlWriter, attr},
};

pub(super) fn write_main(
    xml: &mut XmlWriter,
    plan: &ProjectPlan,
    model: &Moc3Model,
    model_name: &str,
    icons: &[(u32, Vec<u8>)],
) {
    xml.start("CModelSource", &[attr("isDefaultKeyformLocked", "false")]);
    reference(xml, "CModelGuid", "guid", plan.model_guid);
    xml.text("s", &[attr("xs.n", "name")], model_name);
    xml.start("EditorEdition", &[attr("xs.n", "editorEdition")]);
    xml.text("i", &[attr("xs.n", "edition")], "15");
    xml.end("EditorEdition");
    write_canvas(xml, model);
    write_parameters(xml, plan, model);
    write_texture_manager(xml, plan);

    xml.text(
        "b",
        &[attr("xs.n", "useLegacyDrawOrder__testImpl")],
        "false",
    );
    write_drawable_set(xml, plan);
    write_deformer_set(xml, plan, model);
    write_affecter_set(xml, plan);
    write_part_set(xml, plan);
    write_physics_set(xml);
    reference(xml, "CPartSource", "rootPart", plan.root_part.source);

    xml.start("CParameterGroupSet", &[attr("xs.n", "parameterGroupSet")]);
    xml.start(
        "carray_list",
        &[
            attr("xs.n", "_groups"),
            attr("count", 1 + plan.param_groups.len()),
        ],
    );
    reference_without_name(xml, "CParameterGroup", plan.root_param_group.entity);
    for group in &plan.param_groups {
        reference_without_name(xml, "CParameterGroup", group.entity);
    }
    xml.end("carray_list");
    xml.end("CParameterGroupSet");
    reference(
        xml,
        "CParameterGroup",
        "rootParameterGroup",
        plan.root_param_group.entity,
    );
    write_model_info(xml, model);
    xml.empty(
        "hash_map",
        &[attr("xs.n", "modelOptions"), attr("count", 0), attr("keyType", "string")],
    );
    for (size, _) in icons {
        write_icon(xml, *size);
    }
    write_game_motion_set(xml);
    write_model_viewer_setting(xml);
    write_guides_setting(xml);
    xml.text("i", &[attr("xs.n", "targetVersionNo")], "9000000");
    xml.text(
        "i",
        &[attr("xs.n", "latestVersionOfLastModelerNo")],
        "5030000",
    );
    write_art_path_brush_setting(xml);
    write_random_pose_setting(xml, plan);
    write_motion_sync_setting(xml);
    xml.end("CModelSource");
}

fn write_icon(xml: &mut XmlWriter, size: u32) {
    xml.start("CImageIcon", &[attr("xs.n", &format!("_icon{size}"))]);
    xml.start(
        "CWritableImage",
        &[
            attr("xs.n", "image"),
            attr("width", &size.to_string()),
            attr("height", &size.to_string()),
            attr("type", "INT_ARGB"),
        ],
    );
    xml.empty(
        "file",
        &[
            attr("xs.n", "image"),
            attr("path", &format!("image_icon{size}.png")),
        ],
    );
    xml.end("CWritableImage");
    xml.end("CImageIcon");
}

fn write_physics_set(xml: &mut XmlWriter) {
    xml.start(
        "CPhysicsSettingsSourceSet",
        &[attr("xs.n", "physicsSettingsSourceSet")],
    );
    xml.empty(
        "carray_list",
        &[attr("xs.n", "_sourceCubismPhysics"), attr("count", 0)],
    );
    xml.empty("null", &[attr("xs.n", "selectedCubismPhysics")]);
    xml.text("i", &[attr("xs.n", "settingFPS")], "60");
    xml.end("CPhysicsSettingsSourceSet");
}

fn write_game_motion_set(xml: &mut XmlWriter) {
    xml.start("CGameMotionSet", &[attr("xs.n", "gameMotionSet")]);
    xml.empty(
        "carray_list",
        &[attr("xs.n", "gameMotions"), attr("count", 0)],
    );
    xml.empty(
        "carray_list",
        &[attr("xs.n", "gameMotionGroups"), attr("count", 0)],
    );
    xml.end("CGameMotionSet");
}

fn write_model_viewer_setting(xml: &mut XmlWriter) {
    xml.start(
        "ModelViewerSetting",
        &[attr("xs.n", "modelViewerSetting")],
    );
    xml.empty(
        "array_list",
        &[attr("xs.n", "trackCursorSettings"), attr("count", 0)],
    );
    xml.end("ModelViewerSetting");
}

fn write_guides_setting(xml: &mut XmlWriter) {
    xml.start("CGuidesSetting", &[attr("xs.n", "guides")]);
    xml.empty(
        "carray_list",
        &[attr("xs.n", "guidesModeling"), attr("count", 0)],
    );
    xml.end("CGuidesSetting");
}

fn write_art_path_brush_setting(xml: &mut XmlWriter) {
    xml.start(
        "CArtPathBrushSetting",
        &[attr("xs.n", "artPathBrushesSetting")],
    );
    xml.empty("carray_list", &[attr("xs.n", "brushes"), attr("count", 0)]);
    xml.end("CArtPathBrushSetting");
}

fn write_random_pose_setting(xml: &mut XmlWriter, plan: &ProjectPlan) {
    xml.start(
        "CRandomPoseSettingManager",
        &[attr("xs.n", "randomPoseSetting")],
    );
    xml.start("array_list", &[attr("xs.n", "_settings"), attr("count", 1)]);
    xml.start("CRandomPoseSetting", &[]);
    xml.start(
        "array_list",
        &[
            attr("xs.n", "parameters.keys"),
            attr("count", plan.parameters.len()),
        ],
    );
    for parameter in &plan.parameters {
        reference_without_name(xml, "CParameterId", parameter.id_entity);
    }
    xml.end("array_list");
    xml.start(
        "array_list",
        &[
            attr("xs.n", "parameters.values"),
            attr("count", plan.parameters.len()),
        ],
    );
    for _ in &plan.parameters {
        xml.start("CRandomPoseParamData", &[]);
        xml.text("b", &[attr("xs.n", "isEnable")], "true");
        xml.end("CRandomPoseParamData");
    }
    xml.end("array_list");
    xml.start(
        "array_list",
        &[
            attr("xs.n", "groups.keys"),
            attr("count", 1 + plan.param_groups.len()),
        ],
    );
    reference_without_name(xml, "CParameterGroupId", plan.root_param_group.id_entity);
    for group in &plan.param_groups {
        reference_without_name(xml, "CParameterGroupId", group.id_entity);
    }
    xml.end("array_list");
    xml.start(
        "array_list",
        &[
            attr("xs.n", "groups.values"),
            attr("count", 1 + plan.param_groups.len()),
        ],
    );
    for _ in 0..1 + plan.param_groups.len() {
        xml.start("CRandomPoseGroupData", &[]);
        xml.text("b", &[attr("xs.n", "isExpand")], "true");
        xml.end("CRandomPoseGroupData");
    }
    xml.end("array_list");
    xml.end("CRandomPoseSetting");
    xml.end("array_list");
    xml.text("i", &[attr("xs.n", "currentIndex")], "0");
    xml.end("CRandomPoseSettingManager");
}

fn write_motion_sync_setting(xml: &mut XmlWriter) {
    xml.start(
        "CMotionSyncSettingSourceSet",
        &[attr("xs.n", "motionSyncSettingsSet")],
    );
    xml.empty(
        "linked_set",
        &[
            attr("xs.n", "_settingSourceSetMotionSync"),
            attr("count", 0),
        ],
    );
    xml.end("CMotionSyncSettingSourceSet");
}

fn write_canvas(xml: &mut XmlWriter, model: &Moc3Model) {
    let size = model.canvas().size();
    xml.start("CImageCanvas", &[attr("xs.n", "canvas")]);
    xml.text(
        "i",
        &[attr("xs.n", "pixelWidth")],
        &pixel_dimension(size[0]),
    );
    xml.text(
        "i",
        &[attr("xs.n", "pixelHeight")],
        &pixel_dimension(size[1]),
    );
    xml.empty("CColor", &[attr("xs.n", "background")]);
    xml.end("CImageCanvas");
}

fn write_parameters(xml: &mut XmlWriter, plan: &ProjectPlan, model: &Moc3Model) {
    xml.start("CParameterSourceSet", &[attr("xs.n", "parameterSourceSet")]);
    xml.start(
        "carray_list",
        &[
            attr("xs.n", "_sources"),
            attr("count", model.parameters().len()),
        ],
    );
    for (parameter, parameter_plan) in model.parameters().iter().zip(&plan.parameters) {
        xml.start("CParameterSource", &[]);
        xml.text("i", &[attr("xs.n", "decimalPlaces")], "3");
        reference(xml, "CParameterGuid", "guid", parameter_plan.guid);
        xml.text("f", &[attr("xs.n", "snapEpsilon")], "0.001");
        xml.text(
            "f",
            &[attr("xs.n", "minValue")],
            &float(parameter.minimum()),
        );
        xml.text(
            "f",
            &[attr("xs.n", "maxValue")],
            &float(parameter.maximum()),
        );
        xml.text(
            "f",
            &[attr("xs.n", "defaultValue")],
            &float(parameter.default()),
        );
        xml.text("b", &[attr("xs.n", "isRepeat")], "false");
        reference(xml, "CParameterId", "id", parameter_plan.id_entity);
        xml.empty("Type", &[attr("xs.n", "paramType"), attr("v", "NORMAL")]);
        let display_name = plan
            .names
            .parameter_names
            .get(parameter.id())
            .map_or(parameter.id(), String::as_str);
        xml.text("s", &[attr("xs.n", "name")], display_name);
        xml.empty("s", &[attr("xs.n", "description")]);
        xml.text("b", &[attr("xs.n", "combined")], "false");
        let group_guid = plan
            .names
            .parameter_group_of
            .get(parameter.id())
            .and_then(|group_id| {
                plan.names
                    .parameter_groups
                    .iter()
                    .zip(&plan.param_groups)
                    .find(|((id, _, _), _)| id == group_id)
                    .map(|(_, group)| group.guid)
            })
            .unwrap_or(plan.parameter_group_guid);
        reference(xml, "CParameterGroupGuid", "parentGroupGuid", group_guid);
        xml.end("CParameterSource");
    }
    xml.end("carray_list");
    xml.end("CParameterSourceSet");
}

fn write_texture_manager(xml: &mut XmlWriter, plan: &ProjectPlan) {
    xml.start("CTextureManager", &[attr("xs.n", "textureManager")]);
    xml.start("TextureImageGroup", &[attr("xs.n", "textureList")]);
    xml.empty("carray_list", &[attr("xs.n", "children"), attr("count", 0)]);
    xml.end("TextureImageGroup");
    xml.start(
        "carray_list",
        &[attr("xs.n", "_rawImages"), attr("count", 1)],
    );
    xml.start("LayeredImageWrapper", &[]);
    reference(xml, "CLayeredImage", "image", plan.layered_image);
    xml.text("l", &[attr("xs.n", "importedTimeMSec")], "0");
    xml.text("l", &[attr("xs.n", "lastModifiedTimeMSec")], "0");
    xml.text("b", &[attr("xs.n", "isReplaced")], "false");
    xml.end("LayeredImageWrapper");
    xml.end("carray_list");
    xml.start(
        "carray_list",
        &[attr("xs.n", "_modelImageGroups"), attr("count", 1)],
    );
    reference_without_name(xml, "CModelImageGroup", plan.image_group);
    xml.end("carray_list");
    xml.start(
        "carray_list",
        &[
            attr("xs.n", "_textureAtlases"),
            attr("count", plan.pages.len()),
        ],
    );
    for page in &plan.pages {
        reference_without_name(xml, "CTextureAtlas", page.texture_atlas);
    }
    xml.end("carray_list");
    xml.text(
        "b",
        &[attr("xs.n", "isTextureInputModelImageMode")],
        "false",
    );
    xml.text("i", &[attr("xs.n", "previewReductionRatio")], "1");
    xml.empty(
        "carray_list",
        &[
            attr("xs.n", "artPathBrushUsingLayeredImageIds"),
            attr("count", 0),
        ],
    );
    xml.end("CTextureManager");
}

fn write_drawable_set(xml: &mut XmlWriter, plan: &ProjectPlan) {
    xml.start("CDrawableSourceSet", &[attr("xs.n", "drawableSourceSet")]);
    xml.start(
        "carray_list",
        &[attr("xs.n", "_sources"), attr("count", plan.meshes.len())],
    );
    for mesh in &plan.meshes {
        reference_without_name(xml, "CArtMeshSource", mesh.source);
    }
    xml.end("carray_list");
    xml.end("CDrawableSourceSet");
}

fn write_deformer_set(xml: &mut XmlWriter, plan: &ProjectPlan, model: &Moc3Model) {
    xml.start("CDeformerSourceSet", &[attr("xs.n", "deformerSourceSet")]);
    xml.start(
        "carray_list",
        &[
            attr("xs.n", "_sources"),
            attr("count", plan.deformers.len()),
        ],
    );
    for (deformer, deformer_plan) in model.deformers().iter().zip(&plan.deformers) {
        let tag = match deformer {
            Deformer::Warp(_) => "CWarpDeformerSource",
            Deformer::Rotation(_) => "CRotationDeformerSource",
        };
        reference_without_name(xml, tag, deformer_plan.source);
    }
    xml.end("carray_list");
    xml.end("CDeformerSourceSet");
}

fn write_affecter_set(xml: &mut XmlWriter, plan: &ProjectPlan) {
    xml.start("CAffecterSourceSet", &[attr("xs.n", "affecterSourceSet")]);
    xml.start(
        "carray_list",
        &[attr("xs.n", "_sources"), attr("count", plan.glues.len())],
    );
    for glue in &plan.glues {
        reference_without_name(xml, "CGlueSource", glue.source);
    }
    xml.end("carray_list");
    xml.end("CAffecterSourceSet");
}

fn write_part_set(xml: &mut XmlWriter, plan: &ProjectPlan) {
    xml.start("CPartSourceSet", &[attr("xs.n", "partSourceSet")]);
    xml.start(
        "carray_list",
        &[
            attr("xs.n", "_sources"),
            attr("count", plan.parts.len() + 1),
        ],
    );
    reference_without_name(xml, "CPartSource", plan.root_part.source);
    for part in &plan.parts {
        reference_without_name(xml, "CPartSource", part.source);
    }
    xml.end("carray_list");
    xml.end("CPartSourceSet");
}

fn write_model_info(xml: &mut XmlWriter, model: &Moc3Model) {
    let origin = model.canvas().origin();
    xml.start("CModelInfo", &[attr("xs.n", "modelInfo")]);
    xml.text(
        "f",
        &[attr("xs.n", "pixelsPerUnit")],
        &float(model.canvas().pixels_per_unit()),
    );
    xml.start("CPoint", &[attr("xs.n", "originInPixels")]);
    xml.text("i", &[attr("xs.n", "x")], &pixel_coordinate(origin[0]));
    xml.text("i", &[attr("xs.n", "y")], &pixel_coordinate(origin[1]));
    xml.end("CPoint");
    xml.start(
        "CEffectParameterGroups",
        &[attr("xs.n", "_effectParameterGroups")],
    );
    xml.empty(
        "hash_map",
        &[
            attr("xs.n", "_parameterGroups"),
            attr("count", 0),
            attr("keyType", "string"),
        ],
    );
    xml.end("CEffectParameterGroups");
    xml.end("CModelInfo");
}

fn pixel_dimension(value: f32) -> String {
    if value.is_finite() {
        value.abs().round().max(1.0).to_string()
    } else {
        "1".into()
    }
}

fn pixel_coordinate(value: f32) -> String {
    if value.is_finite() {
        value.round().to_string()
    } else {
        "0".into()
    }
}
