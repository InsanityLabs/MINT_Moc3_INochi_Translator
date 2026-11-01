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
    write_physics_set(xml, plan, model);
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

fn write_physics_set(xml: &mut XmlWriter, plan: &ProjectPlan, model: &Moc3Model) {
    use serde_json::Value;
    let name_of = |v: &Value| v.as_str().unwrap_or("").to_string();
    let num = |v: &Value, d: f32| v.as_f64().map_or(d, |x| x as f32);
    let flag = |v: &Value| if v.as_bool().unwrap_or(false) { "true" } else { "false" };
    let param_ref = |id: &str| {
        model
            .parameters()
            .iter()
            .position(|p| p.id() == id)
            .map(|i| plan.parameters[i].guid)
    };
    let src_type = |t: &str| match t {
        "X" => "SRC_TO_X",
        "Y" => "SRC_TO_Y",
        _ => "SRC_TO_G_ANGLE",
    };
    let data_guid = |xml: &mut XmlWriter| {
        xml.empty(
            "CPhysicsDataGuid",
            &[
                attr("xs.n", "guid"),
                attr("uuid", &uuid::Uuid::new_v4().to_string()),
                attr("note", "(no debug info)"),
            ],
        )
    };
    let items = |v: &Value| -> Vec<Value> { v.as_array().cloned().unwrap_or_default() };
    let physics = plan.names.physics.as_ref();
    let settings: Vec<Value> = physics.map(|p| p.settings().to_vec()).unwrap_or_default();
    let dict: Vec<(String, String)> = physics
        .map(|p| items(&p.meta()["PhysicsDictionary"]))
        .unwrap_or_default()
        .iter()
        .map(|d| (name_of(&d["Id"]), name_of(&d["Name"])))
        .collect();

    xml.start(
        "CPhysicsSettingsSourceSet",
        &[attr("xs.n", "physicsSettingsSourceSet")],
    );
    xml.start(
        "carray_list",
        &[attr("xs.n", "_sourceCubismPhysics"), attr("count", settings.len())],
    );
    let mut first_uuid: Option<String> = None;
    for s in &settings {
        let id = name_of(&s["Id"]);
        let name = dict
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| id.clone());
        let setting_uuid = uuid::Uuid::new_v4().to_string();
        first_uuid.get_or_insert(setting_uuid.clone());
        xml.start("CPhysicsSettingsSource", &[]);
        xml.text("s", &[attr("xs.n", "name")], &name);
        xml.empty(
            "CPhysicsSettingsGuid",
            &[
                attr("xs.n", "guid"),
                attr("uuid", &setting_uuid),
                attr("note", "(no debug info)"),
            ],
        );
        xml.empty("CPhysicsSettingId", &[attr("xs.n", "id"), attr("idstr", &id)]);

        let inputs = items(&s["Input"]);
        xml.start(
            "carray_list",
            &[attr("xs.n", "inputs"), attr("count", inputs.len())],
        );
        for i in &inputs {
            xml.start("CPhysicsInput", &[]);
            data_guid(xml);
            match param_ref(&name_of(&i["Source"]["Id"])) {
                Some(r) => reference(xml, "CParameterGuid", "source", r),
                None => xml.empty("null", &[attr("xs.n", "source")]),
            }
            xml.text("f", &[attr("xs.n", "angleScale")], "0.0");
            write_vec2(xml, "translationScale", 0.0, 0.0);
            xml.text("f", &[attr("xs.n", "weight")], &fl(num(&i["Weight"], 0.0)));
            xml.empty(
                "CPhysicsSourceType",
                &[
                    attr("xs.n", "type"),
                    attr("v", src_type(i["Type"].as_str().unwrap_or(""))),
                ],
            );
            xml.text("b", &[attr("xs.n", "isReverse")], flag(&i["Reflect"]));
            xml.end("CPhysicsInput");
        }
        xml.end("carray_list");

        let outputs = items(&s["Output"]);
        xml.start(
            "carray_list",
            &[attr("xs.n", "outputs"), attr("count", outputs.len())],
        );
        for o in &outputs {
            let ty = o["Type"].as_str().unwrap_or("");
            let scale = num(&o["Scale"], 1.0);
            xml.start("CPhysicsOutput", &[]);
            data_guid(xml);
            match param_ref(&name_of(&o["Destination"]["Id"])) {
                Some(r) => reference(xml, "CParameterGuid", "destination", r),
                None => xml.empty("null", &[attr("xs.n", "destination")]),
            }
            xml.text(
                "i",
                &[attr("xs.n", "vertexIndex")],
                &o["VertexIndex"].as_i64().unwrap_or(0).to_string(),
            );
            let (tx, ty_) = match ty {
                "X" => (scale, 0.0),
                "Y" => (0.0, scale),
                _ => (0.0, 0.0),
            };
            write_vec2(xml, "translationScale", tx, ty_);
            xml.text(
                "f",
                &[attr("xs.n", "angleScale")],
                &fl(if ty == "Angle" { scale } else { 0.0 }),
            );
            xml.text("f", &[attr("xs.n", "weight")], &fl(num(&o["Weight"], 100.0)));
            xml.empty(
                "CPhysicsSourceType",
                &[attr("xs.n", "type"), attr("v", src_type(ty))],
            );
            xml.text("b", &[attr("xs.n", "isReverse")], flag(&o["Reflect"]));
            xml.end("CPhysicsOutput");
        }
        xml.end("carray_list");

        let verts = items(&s["Vertices"]);
        xml.start(
            "carray_list",
            &[attr("xs.n", "vertices"), attr("count", verts.len())],
        );
        for v in &verts {
            xml.start("CPhysicsVertex", &[]);
            data_guid(xml);
            write_vec2(
                xml,
                "position",
                num(&v["Position"]["X"], 0.0),
                num(&v["Position"]["Y"], 0.0),
            );
            for (k, f) in [
                ("mobility", "Mobility"),
                ("delay", "Delay"),
                ("acceleration", "Acceleration"),
                ("radius", "Radius"),
            ] {
                xml.text("f", &[attr("xs.n", k)], &fl(num(&v[f], 0.0)));
            }
            xml.end("CPhysicsVertex");
        }
        xml.end("carray_list");

        for (kind, key) in [("Position", "normalizedPosition"), ("Angle", "normalizedAngle")] {
            let n = &s["Normalization"][kind];
            for (suffix, field, d) in [
                ("ValueMax", "Maximum", 10.0),
                ("ValueMin", "Minimum", -10.0),
                ("DefaultValue", "Default", 0.0),
            ] {
                xml.text(
                    "f",
                    &[attr("xs.n", &format!("{key}{suffix}"))],
                    &fl(num(&n[field], d)),
                );
            }
        }
        xml.end("CPhysicsSettingsSource");
    }
    xml.end("carray_list");
    match first_uuid {
        Some(u) => xml.empty(
            "CPhysicsSettingsGuid",
            &[
                attr("xs.n", "selectedCubismPhysics"),
                attr("uuid", &u),
                attr("note", "(no debug info)"),
            ],
        ),
        None => xml.empty("null", &[attr("xs.n", "selectedCubismPhysics")]),
    }
    match physics.and_then(|p| p.meta()["Fps"].as_i64()) {
        Some(f) if f > 0 => xml.text("i", &[attr("xs.n", "settingFPS")], &f.to_string()),
        _ => xml.empty("null", &[attr("xs.n", "settingFPS")]),
    }
    xml.end("CPhysicsSettingsSourceSet");
}

fn write_vec2(xml: &mut XmlWriter, name: &str, x: f32, y: f32) {
    xml.start("GVector2", &[attr("xs.n", name)]);
    xml.text("f", &[attr("xs.n", "x")], &fl(x));
    xml.text("f", &[attr("xs.n", "y")], &fl(y));
    xml.end("GVector2");
}

fn fl(value: f32) -> String {
    let s = float(value);
    if s.contains('.') || s.contains('e') { s } else { format!("{s}.0") }
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
    let ids: Vec<&str> = model.parameters().iter().map(|p| p.id()).collect();
    let guess = crate::decompiler::GUESS_PAIRS.load(std::sync::atomic::Ordering::Relaxed);
    let is_pair = |a: &str, b: &str| {
        matches!(
            (a, b),
            ("ParamAngleX", "ParamAngleY")
                | ("ParamEyeBallX", "ParamEyeBallY")
                | ("ParamMouthForm", "ParamMouthOpenY")
        ) || (guess && matches!((a.strip_suffix('X'), b.strip_suffix('Y')), (Some(x), Some(y)) if x == y))
    };
    for (index, (parameter, parameter_plan)) in model.parameters().iter().zip(&plan.parameters).enumerate() {
        let combined = ids.get(index + 1).is_some_and(|next| is_pair(ids[index], next));
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
        xml.text("b", &[attr("xs.n", "combined")], if combined { "true" } else { "false" });
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
