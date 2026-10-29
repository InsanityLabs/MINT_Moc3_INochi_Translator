use mint_core::{model::Model, moc3::Moc3};
use std::path::{Path, PathBuf};
use std::fs;

fn load_moc(path: &Path) -> Moc3 {
    let data = std::fs::read(path).expect("read moc3");
    match Moc3::parse(&data) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("parse error: {e}");
            std::process::exit(1);
        }
    }
}

/// Load textures referenced by `<name>.model3.json` next to the moc3.
fn load_textures(moc_path: &Path) -> Vec<image::RgbaImage> {
    let stem = moc_path.with_extension("");
    let model3 = {
        let p = stem.with_extension("model3.json");
        if p.exists() {
            Some(p)
        } else {
            // fall back to <name>.model3.json alongside
            let p2 = moc_path.with_file_name(format!(
                "{}.model3.json",
                moc_path.file_stem().unwrap().to_string_lossy()
            ));
            p2.exists().then_some(p2)
        }
    };
    let Some(model3) = model3 else {
        eprintln!("warning: no model3.json found; no textures loaded");
        return Vec::new();
    };
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&model3).expect("read model3.json")).expect("parse model3.json");
    let dir = model3.parent().unwrap_or(Path::new("."));
    let mut out = Vec::new();
    if let Some(texs) = json["FileReferences"]["Textures"].as_array() {
        for t in texs {
            let rel = t.as_str().unwrap_or_default();
            let p = dir.join(rel.replace('\\', "/"));
            match image::open(&p) {
                Ok(img) => out.push(img.into_rgba8()),
                Err(e) => eprintln!("warning: can't load {}: {e}", p.display()),
            }
        }
    }
    out
}

/// Read the raw PNG bytes of each texture referenced by `<name>.model3.json`.
fn load_texture_pngs(moc_path: &Path) -> Vec<Vec<u8>> {
    let stem = moc_path.with_extension("");
    let model3 = stem.with_extension("model3.json");
    let dir = model3.parent().unwrap_or(Path::new("."));
    let mut out = Vec::new();
    if !model3.exists() {
        return out;
    }
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&model3).unwrap()).unwrap();
    if let Some(texs) = json["FileReferences"]["Textures"].as_array() {
        for t in texs {
            let rel = t.as_str().unwrap_or_default();
            let p = dir.join(rel.replace('\\', "/"));
            if let Ok(b) = std::fs::read(&p) {
                out.push(b);
            }
        }
    }
    out
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: mint-cli <command> <moc3> [args]");
        eprintln!("commands: info parts meshes params deformers render <out.png>");
        std::process::exit(2);
    }
    let cmd = args[1].as_str();
    let path = PathBuf::from(&args[2]);
    if cmd == "cmo3info" {
        let xml = moc2cmo::cmo3::Cmo3Project::main_xml_from_bytes(&fs::read(&path)?).expect("not a readable cmo3");
        let s = String::from_utf8_lossy(&xml);
        for key in ["fileFormatVersion=\"", "targetVersionNo\">", "latestVersionOfLastModelerNo\">", "isQuadTransform\">", "edition\">"] {
            let mut n = 0;
            for (i, _) in s.match_indices(key) {
                if n < 3 { println!("{}{}", key, s[i + key.len()..].chars().take_while(|c| *c != '"' && *c != '<').collect::<String>()); }
                n += 1;
            }
            println!("  ({} occurrences of {})", n, key);
        }
        return Ok(());
    }
    if cmd == "cmo3dump" {
        let archive = moc2cmo::caff::decode_archive(&fs::read(&path)?).expect("not a readable cmo3");
        let out_dir = args.get(3).map(PathBuf::from);
        println!("key: {}", archive.key);
        for e in &archive.entries {
            println!("{}  {} bytes  [{}]", e.path, e.bytes.len(), e.tag);
            if let Some(dir) = &out_dir {
                let dest = dir.join(&e.path);
                if let Some(p) = dest.parent() { fs::create_dir_all(p)?; }
                fs::write(&dest, &e.bytes)?;
            }
        }
        return Ok(());
    }
    let moc = load_moc(&path);
    match cmd {
        "info" => {
            let c = &moc.canvas;
            println!("version:      {}", moc.version);
            println!("canvas:       {}x{} ppu={} origin=({},{}) reverseY={}",
                c.width, c.height, c.pixels_per_unit, c.origin_x, c.origin_y, c.reverse_y);
            println!("parts:        {}", moc.parts.len());
            println!("deformers:    {} ({} warp, {} rotation)",
                moc.deformers.len(), moc.warp_deformers.len(), moc.rotation_deformers.len());
            println!("art meshes:   {}", moc.art_meshes.len());
            println!("parameters:   {}", moc.parameters.len());
            println!("keyforms:     {} part, {} warp, {} rot, {} mesh",
                moc.part_keyforms.len(), moc.warp_deformer_keyforms.len(),
                moc.rotation_deformer_keyforms.len(), moc.art_mesh_keyforms.len());
            println!("kf positions: {}", moc.keyform_positions.len());
            println!("uvs:          {}", moc.uvs.len());
            println!("pos indices:  {}", moc.position_indices.len());
            println!("masks:        {}", moc.drawable_masks.len());
            println!("bindings:     {} kfBindings, {} paramBindings, {} pbi, {} keys",
                moc.keyform_bindings.len(), moc.parameter_bindings.len(),
                moc.parameter_binding_indices.len(), moc.keys.len());
        }
        "parts" => {
            for (i, p) in moc.parts.iter().enumerate() {
                println!("{i:3} {:?} parent={} kf=[{},+{}) vis={} en={}",
                    p.id, p.parent_part, p.keyform_sources_begin,
                    p.keyform_sources_count, p.visible, p.enabled);
            }
        }
        "meshes" => {
            for (i, a) in moc.art_meshes.iter().enumerate() {
                println!("{i:3} {:?} tex={} verts={} tris={} uvb={} pib={} masks={}+{} blend={} pp={} pd={}",
                    a.id, a.texture_no, a.vertex_count, a.position_index_count / 3,
                    a.uv_begin, a.position_index_begin, a.mask_begin, a.mask_count,
                    a.blend_mode, a.parent_part, a.parent_deformer);
            }
        }
        "params" => {
            for (i, p) in moc.parameters.iter().enumerate() {
                println!("{i:3} {:?} [{}, {}] def={} bindings={}+{}",
                    p.id, p.min, p.max, p.default,
                    p.binding_sources_begin, p.binding_sources_count);
            }
        }
        "order" => {
            let mut model = Model::new(&moc);
            for (pos, m) in model.render_order().into_iter().enumerate() {
                let (_, op, d) = model.art_mesh(m);
                println!("{pos} mesh{m} {} pp={} op={op:.2} do={d}", moc.art_meshes[m].id, moc.art_meshes[m].parent_part);
            }
        }
        "maskdump" => {
            println!("all masks: {:?}", moc.drawable_masks);
            for (i, a) in moc.art_meshes.iter().enumerate() {
                if a.mask_count > 0 {
                    let b = a.mask_begin as usize;
                    println!("{i} {:?} masked_by {:?} inv={}", a.id, &moc.drawable_masks[b..b + a.mask_count as usize], a.inverted_mask);
                }
            }
        }
        "quads" => {
            println!("warp deformers: {} quad-source: {}", moc.warp_deformers.len(), moc.warp_deformer_quad_source.iter().filter(|&&q| q).count());
        }
        "bindings" => {
            for (i, a) in moc.art_meshes.iter().enumerate() {
                let b: Vec<String> = mint_core::inp::obj_bindings(&moc, a.keyform_binding_source).into_iter()
                    .map(|(p, k)| format!("{}:{:?}", moc.parameters[p].id, k.map(|x| moc.keys[x]).collect::<Vec<_>>())).collect();
                println!("M	{}	{}	{}", i, a.id, b.join(";"));
            }
            for (i, d) in moc.deformers.iter().enumerate() {
                let b: Vec<String> = mint_core::inp::obj_bindings(&moc, mint_core::inp::deformer_kbs(&moc, d)).into_iter()
                    .map(|(p, k)| format!("{}:{:?}", moc.parameters[p].id, k.map(|x| moc.keys[x]).collect::<Vec<_>>())).collect();
                let nk = match d.ty { mint_core::moc3::DeformerType::Warp => moc.warp_deformers[d.specific_index as usize].keyform_sources_count, _ => moc.rotation_deformers[d.specific_index as usize].keyform_sources_count };
                println!("D	{}	{}	{}	{}", i, d.id, b.join(";"), nk);
            }
        }
        "cmo3xml" => {
            let data = fs::read(&path)?;
            let force = args.get(4).map(|s| s == "--all").unwrap_or(false);
            let p = mint_core::emit::cmo3_project(&moc, &data, &mint_core::emit::texture_pngs(Path::new(&path)), force).expect("cmo3");
            fs::write(&args[3], p.main_xml())?;
        }
        "deformers" => {
            for (i, d) in moc.deformers.iter().enumerate() {
                println!("{i:3} {:?} ty={:?} spec={} pp={} pd={}",
                    d.id, d.ty, d.specific_index, d.parent_part, d.parent_deformer);
            }
        }
        "dumpverts" => {
            let mut model = Model::new(&moc);
            let n = moc.art_meshes.len();
            let limit: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(n);
            for mi in 0..n.min(limit) {
                let (verts, op, dor) = model.art_mesh(mi);
                let mut min = [f32::MAX; 2];
                let mut max = [f32::MIN; 2];
                for v in &verts {
                    min[0] = min[0].min(v.x);
                    min[1] = min[1].min(v.y);
                    max[0] = max[0].max(v.x);
                    max[1] = max[1].max(v.y);
                }
                println!("mesh{mi} bbox=({:.3},{:.3})-({:.3},{:.3}) op={op:.2} do={dor:.1}",
                    min[0], min[1], max[0], max[1]);
            }
        }
        "render" => {
            let out = args.get(3).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("render.png"));
            let textures = load_textures(&path);
            eprintln!("textures: {}", textures.len());
            let mut model = Model::new(&moc);
            let img = mint_core::render::render(&mut model, &textures);
            img.save(&out).expect("save png");
            println!("wrote {}", out.display());
        }
        "export" => {
            let stem = args.get(3).map(PathBuf::from).unwrap_or_else(|| {
                PathBuf::from(format!("{}_extracted", path.file_stem().unwrap().to_string_lossy()))
            });
            let export_all = args.get(4).map(|s| s == "--all").unwrap_or(false);
            let textures = load_textures(&path);
            let mut model = Model::new(&moc);
            let merged = mint_core::render::render(&mut model, &textures);
            let mut model2 = Model::new(&moc);
            let (root, w, h) = mint_core::emit::build_layers(&mut model2, &textures, export_all);

            let ora_path = stem.with_extension("ora");
            let f = std::io::BufWriter::new(std::fs::File::create(&ora_path)?);
            mint_core::emit::ora::write_ora(&root, &merged, w, h, f)?;
            println!("wrote {}", ora_path.display());

            let psd_path = stem.with_extension("psd");
            let f = std::io::BufWriter::new(std::fs::File::create(&psd_path)?);
            mint_core::emit::psd::write_psd(&root, &merged, w, h, f)?;
            println!("wrote {}", psd_path.display());

            // cmo3 export via moc2cmo — uses the model3.json manifest when
            // present (restores names, embeds aux JSON, builds a can3)
            let cmo3_path = stem.with_extension("cmo3");
            match mint_core::emit::export_cmo3(&moc, &path, &mut model, &textures, args.iter().any(|a| a == "--cmo3-visible")) {
                Ok(out) => {
                    fs::write(&cmo3_path, out.cmo3)?;
                    println!("wrote {}", cmo3_path.display());
                    if let Some(can3) = out.can3 {
                        let can3_path = stem.with_extension("can3");
                        fs::write(&can3_path, can3)?;
                        println!("wrote {}", can3_path.display());
                    }
                }
                Err(e) => eprintln!("cmo3 export failed: {}", e),
            }

            // Cubism imports physics/expressions/motions as separate files: ship them beside the cmo3.
            let side_dir = stem.with_file_name(format!("{}_cubism", stem.file_name().unwrap().to_string_lossy()));
            if let Some(src_dir) = Path::new(&path).parent() {
                let mut copied = 0;
                for e in fs::read_dir(src_dir)?.flatten() {
                    let n = e.file_name().to_string_lossy().to_string();
                    if [".physics3.json", ".cdi3.json", ".exp3.json", ".pose3.json", ".motion3.json", ".userdata3.json", ".vtube.json"].iter().any(|s| n.ends_with(s)) {
                        fs::create_dir_all(&side_dir)?;
                        fs::copy(e.path(), side_dir.join(&n))?;
                        copied += 1;
                    }
                }
                if copied > 0 { println!("copied {} physics/expression/motion files to {}", copied, side_dir.display()); }
            }

            // inp export (Inochi2D) — same TRNSRTS container for both
            // .inp (runtime) and .inx (creator project)
            let inp_path = stem.with_extension("inp");
            let f = std::io::BufWriter::new(std::fs::File::create(&inp_path)?);
            mint_core::inp::write_inp_ext(&moc, &mut model, &textures, &mint_core::inp::sidecars(Path::new(&path)), f)?;
            println!("wrote {}", inp_path.display());
            let inx_path = stem.with_extension("inx");
            let f = std::io::BufWriter::new(std::fs::File::create(&inx_path)?);
            let mut model3 = Model::new(&moc);
            mint_core::inp::write_inp_ext(&moc, &mut model3, &textures, &mint_core::inp::sidecars(Path::new(&path)), f)?;
            println!("wrote {}", inx_path.display());
        }
        _ => {
            eprintln!("unknown command {cmd}");
            std::process::exit(2);
        }
    }
    Ok(())
}
