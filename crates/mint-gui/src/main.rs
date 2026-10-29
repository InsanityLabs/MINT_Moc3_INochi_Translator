use eframe::egui::{self, Color32, RichText};
use image::RgbaImage;
use mint_core::{Model, Moc3};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq)]
enum Level {
    Info,
    Warn,
    Error,
}

struct Issue {
    level: Level,
    text: String,
}

struct Loaded {
    path: PathBuf,
    info: Vec<(String, String)>,
    issues: Vec<Issue>,
    preview: RgbaImage,
}

#[derive(Clone)]
struct Opts {
    psd: bool,
    ora: bool,
    cmo3: bool,
    inochi: bool,
    all_layers: bool,
    cmo3_visible: bool,
    out_dir: PathBuf,
}

type Slot<T> = Arc<Mutex<Option<T>>>;

fn load_textures(moc_path: &Path) -> Vec<RgbaImage> {
    let model3 = moc_path.with_extension("model3.json");
    let dir = moc_path.parent().unwrap_or(Path::new("."));
    let Ok(bytes) = fs::read(&model3) else { return Vec::new() };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return Vec::new() };
    json["FileReferences"]["Textures"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| image::open(dir.join(t.as_str()?.replace('\\', "/"))).ok())
                .map(|i| i.into_rgba8())
                .collect()
        })
        .unwrap_or_default()
}

/// Every file the model3.json points at, as (label, relative path).
fn referenced_files(moc_path: &Path) -> Option<Vec<(String, String)>> {
    let bytes = fs::read(moc_path.with_extension("model3.json")).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let r = &json["FileReferences"];
    let mut out = Vec::new();
    for (key, label) in [("Physics", "physics"), ("Pose", "pose"), ("UserData", "user data"), ("DisplayInfo", "display info")] {
        if let Some(s) = r[key].as_str() {
            out.push((label.to_string(), s.to_string()));
        }
    }
    for t in r["Textures"].as_array().into_iter().flatten().filter_map(|t| t.as_str()) {
        out.push(("texture".into(), t.to_string()));
    }
    for e in r["Expressions"].as_array().into_iter().flatten().filter_map(|e| e["File"].as_str()) {
        out.push(("expression".into(), e.to_string()));
    }
    if let Some(m) = r["Motions"].as_object() {
        for list in m.values().filter_map(|v| v.as_array()) {
            for f in list.iter().filter_map(|e| e["File"].as_str()) {
                out.push(("motion".into(), f.to_string()));
            }
        }
    }
    Some(out)
}

fn analyze(path: &Path) -> Result<Loaded, String> {
    let data = fs::read(path).map_err(|e| format!("cannot read file: {e}"))?;
    let moc = Moc3::parse(&data).map_err(|e| format!("not a valid moc3: {e}"))?;
    let textures = load_textures(path);
    let mut issues = Vec::new();
    let mut warn = |level, text: String| issues.push(Issue { level, text });

    match referenced_files(path) {
        None => warn(Level::Warn, "No model3.json next to the moc3: textures can't be loaded.".into()),
        Some(files) => {
            let dir = path.parent().unwrap_or(Path::new("."));
            for (label, rel) in files {
                if !dir.join(rel.replace('\\', "/")).exists() {
                    let lvl = if label == "texture" { Level::Error } else { Level::Warn };
                    warn(lvl, format!("Missing {label} file: {rel}"));
                }
            }
        }
    }
    if textures.is_empty() {
        warn(Level::Error, "No textures loaded: exports will be blank.".into());
    }
    let tex_n = textures.len() as u32;
    let bad_tex = moc.art_meshes.iter().filter(|a| a.texture_no >= tex_n).count();
    if bad_tex > 0 && !textures.is_empty() {
        warn(Level::Error, format!("{bad_tex} mesh(es) use a texture page that isn't loaded."));
    }
    let empty = moc.art_meshes.iter().filter(|a| a.vertex_count <= 0 || a.position_index_count < 3).count();
    if empty > 0 {
        warn(Level::Warn, format!("{empty} mesh(es) have no triangles."));
    }
    let lost_masks = moc.art_meshes.iter().filter(|a| a.inverted_mask && a.mask_count == 0).count();
    if lost_masks > 0 {
        warn(Level::Warn, format!("{lost_masks} inverted-mask mesh(es) have masks stored in a section we don't read (Cubism 5.3 offscreen)."));
    }
    let off: Vec<&str> = moc.parts.iter().zip(&moc.offscreen_out_parts).filter(|(_, o)| **o).map(|(p, _)| p.id.as_str()).collect();
    if !off.is_empty() {
        warn(Level::Warn, format!("Offscreen \"Out\" parts are hidden by default: {}", off.join(", ")));
    }
    let hidden = mint_core::emit::hidden_parts(&moc, Some(path));
    let arms: Vec<&String> = hidden.iter().filter(|h| !off.contains(&h.as_str())).collect();
    if !arms.is_empty() {
        let names: Vec<&str> = arms.iter().map(|s| s.as_str()).collect();
        warn(Level::Info, format!("Alternate parts hidden by default (pose): {}", names.join(", ")));
    }
    if moc.version >= 6 {
        warn(Level::Info, "Cubism 5.3 (v6) model: blend modes and offscreen drawing are approximated.".into());
    }
    if issues.iter().all(|i| i.level == Level::Info) {
        issues.insert(0, Issue { level: Level::Info, text: "No problems found.".into() });
    }

    let mut model = Model::new(&moc);
    let mut preview = mint_core::render::render(&mut model, &textures);
    let (w, h) = preview.dimensions();
    let m = w.max(h);
    if m > 2048 {
        let s = 2048.0 / m as f32;
        preview = image::imageops::resize(&preview, (w as f32 * s) as u32, (h as f32 * s) as u32, image::imageops::FilterType::Triangle);
    }

    let info = vec![
        ("Version".into(), format!("moc3 v{}", moc.version)),
        ("Canvas".into(), format!("{} x {}", moc.canvas.width, moc.canvas.height)),
        ("Art meshes".into(), moc.art_meshes.len().to_string()),
        ("Parts".into(), moc.parts.len().to_string()),
        ("Deformers".into(), moc.deformers.len().to_string()),
        ("Parameters".into(), moc.parameters.len().to_string()),
        ("Textures".into(), textures.len().to_string()),
    ];
    Ok(Loaded { path: path.to_path_buf(), info, issues, preview })
}

fn export(path: &Path, o: &Opts, log: &Arc<Mutex<Vec<String>>>) -> Result<(), String> {
    let say = |s: String| log.lock().unwrap().push(s);
    let e = |x: std::io::Error| x.to_string();
    let name = path.file_stem().unwrap().to_string_lossy().to_string();
    fs::create_dir_all(&o.out_dir).map_err(e)?;
    let stem = o.out_dir.join(&name);
    let data = fs::read(path).map_err(e)?;
    let moc = Moc3::parse(&data).map_err(|x| x.to_string())?;
    let textures = load_textures(path);

    if o.psd || o.ora {
        say("Building layers...".into());
        let mut m = Model::new(&moc);
        let merged = mint_core::render::render(&mut m, &textures);
        let mut m2 = Model::new(&moc);
        let (root, w, h) = mint_core::emit::build_layers(&mut m2, &textures, o.all_layers);
        if o.ora {
            let p = stem.with_extension("ora");
            let f = std::io::BufWriter::new(fs::File::create(&p).map_err(e)?);
            mint_core::emit::ora::write_ora(&root, &merged, w, h, f).map_err(e)?;
            say(format!("wrote {}", p.display()));
        }
        if o.psd {
            let p = stem.with_extension("psd");
            let f = std::io::BufWriter::new(fs::File::create(&p).map_err(e)?);
            mint_core::emit::psd::write_psd(&root, &merged, w, h, f).map_err(e)?;
            say(format!("wrote {}", p.display()));
        }
    }
    if o.cmo3 {
        say("Building Cubism project...".into());
        let mut m = Model::new(&moc);
        let out = mint_core::emit::export_cmo3(&moc, path, &mut m, &textures, o.cmo3_visible)?;
        let p = stem.with_extension("cmo3");
        fs::write(&p, out.cmo3).map_err(e)?;
        say(format!("wrote {}", p.display()));
        if let Some(can3) = out.can3 {
            let p = stem.with_extension("can3");
            fs::write(&p, can3).map_err(e)?;
            say(format!("wrote {}", p.display()));
        }
    }
    if o.inochi {
        say("Building Inochi2D puppet...".into());
        for ext in ["inp", "inx"] {
            let p = stem.with_extension(ext);
            let f = std::io::BufWriter::new(fs::File::create(&p).map_err(e)?);
            let mut m = Model::new(&moc);
            mint_core::inp::write_inp_ext(&moc, &mut m, &textures, &mint_core::inp::sidecars(path), f).map_err(e)?;
            say(format!("wrote {}", p.display()));
        }
    }
    Ok(())
}

struct App {
    path: Option<PathBuf>,
    loaded: Option<Loaded>,
    texture: Option<egui::TextureHandle>,
    zoom: Option<f32>,
    opts: Opts,
    loading: Slot<Result<Loaded, String>>,
    is_loading: bool,
    busy: Arc<AtomicBool>,
    log: Arc<Mutex<Vec<String>>>,
    about: bool,
}

const ABOUT_LIBS: &[(&str, &str)] = &[
    ("moc2cmo (vendored, modified)", "MIT — Eatgrapes"),
    ("egui / eframe", "MIT OR Apache-2.0 — Emil Ernerfeldt"),
    ("image", "MIT OR Apache-2.0 — image-rs"),
    ("zip", "MIT — zip-rs"),
    ("rfd", "MIT — PolyMeilex"),
    ("serde / serde_json", "MIT OR Apache-2.0 — serde-rs"),
    ("byteorder", "Unlicense OR MIT — BurntSushi"),
    ("flate2 + zlib-rs", "MIT OR Apache-2.0 — rust-lang / trifectatech"),
    ("quick-xml", "MIT — tafia"),
    ("thiserror", "MIT OR Apache-2.0 — dtolnay"),
];

impl Default for App {
    fn default() -> Self {
        Self {
            path: None,
            loaded: None,
            texture: None,
            zoom: None,
            opts: Opts { psd: true, ora: true, cmo3: true, inochi: true, all_layers: true, cmo3_visible: false, out_dir: PathBuf::new() },
            loading: Arc::new(Mutex::new(None)),
            is_loading: false,
            busy: Arc::new(AtomicBool::new(false)),
            log: Arc::new(Mutex::new(vec!["Open a .moc3 file (or drop one on the window).".into()])),
            about: false,
        }
    }
}

impl App {
    fn open(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.opts.out_dir = path.parent().unwrap_or(Path::new(".")).join(format!("{}_export", path.file_stem().unwrap().to_string_lossy()));
        self.path = Some(path.clone());
        self.loaded = None;
        self.texture = None;
        self.is_loading = true;
        self.log.lock().unwrap().clear();
        self.log.lock().unwrap().push(format!("Loading {}...", path.display()));
        let slot = self.loading.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            *slot.lock().unwrap() = Some(analyze(&path));
            ctx.request_repaint();
        });
    }

    fn start_export(&mut self, ctx: &egui::Context) {
        let Some(path) = self.path.clone() else { return };
        self.busy.store(true, Ordering::SeqCst);
        self.log.lock().unwrap().clear();
        let (opts, log, busy, ctx) = (self.opts.clone(), self.log.clone(), self.busy.clone(), ctx.clone());
        std::thread::spawn(move || {
            match export(&path, &opts, &log) {
                Ok(()) => log.lock().unwrap().push("Done.".into()),
                Err(e) => log.lock().unwrap().push(format!("FAILED: {e}")),
            }
            busy.store(false, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if let Some(f) = ctx.input(|i| i.raw.dropped_files.first().and_then(|f| f.path.clone())) {
            self.open(ctx, f);
        }
        if let Some(res) = self.loading.lock().unwrap().take() {
            self.is_loading = false;
            match res {
                Ok(l) => {
                    let (w, h) = l.preview.dimensions();
                    let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], l.preview.as_raw());
                    self.texture = Some(ctx.load_texture("preview", img, egui::TextureOptions::LINEAR));
                    self.log.lock().unwrap().clear();
                    self.log.lock().unwrap().push("Ready.".into());
                    self.loaded = Some(l);
                }
                Err(e) => {
                    self.log.lock().unwrap().push(format!("ERROR: {e}"));
                }
            }
        }
        let busy = self.busy.load(Ordering::SeqCst);
        if busy || self.is_loading {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open .moc3...").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("MOC3", &["moc3"]).pick_file() {
                        self.open(ctx, p);
                    }
                }
                ui.separator();
                if ui.button("About").clicked() {
                    self.about = true;
                }
                match &self.path {
                    Some(p) => ui.label(p.display().to_string()),
                    None => ui.label(RichText::new("No model loaded").weak()),
                };
                if self.is_loading {
                    ui.spinner();
                }
            });
        });

        egui::TopBottomPanel::bottom("bottom").resizable(true).default_height(110.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let can = self.loaded.is_some() && !busy && (self.opts.psd || self.opts.ora || self.opts.cmo3 || self.opts.inochi);
                if ui.add_enabled(can, egui::Button::new(RichText::new("Export").strong()).min_size(egui::vec2(90.0, 28.0))).clicked() {
                    self.start_export(ctx);
                }
                if busy {
                    ui.spinner();
                }
                if !busy && self.log.lock().unwrap().last().is_some_and(|l| l == "Done.") && ui.button("Open output folder").clicked() {
                    let _ = std::process::Command::new("explorer").arg(&self.opts.out_dir).spawn();
                }
            });
            egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                for l in self.log.lock().unwrap().iter() {
                    let c = if l.starts_with("FAILED") || l.starts_with("ERROR") { Color32::LIGHT_RED } else { ui.visuals().text_color() };
                    ui.label(RichText::new(l).color(c).monospace());
                }
            });
        });

        egui::SidePanel::left("side").resizable(true).default_width(320.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if let Some(l) = &self.loaded {
                    ui.heading("Model");
                    egui::Grid::new("info").num_columns(2).show(ui, |ui| {
                        for (k, v) in &l.info {
                            ui.label(RichText::new(k).weak());
                            ui.label(v);
                            ui.end_row();
                        }
                    });
                    ui.add_space(8.0);
                    ui.heading("Checks");
                    for i in &l.issues {
                        let (icon, c) = match i.level {
                            Level::Info => ("i", Color32::LIGHT_BLUE),
                            Level::Warn => ("!", Color32::from_rgb(240, 190, 60)),
                            Level::Error => ("x", Color32::LIGHT_RED),
                        };
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(format!("[{icon}]")).color(c).monospace());
                            ui.label(&i.text);
                        });
                    }
                    ui.add_space(8.0);
                }
                ui.heading("Export");
                ui.checkbox(&mut self.opts.psd, "PSD (Krita / Photoshop)");
                ui.checkbox(&mut self.opts.ora, "ORA (Krita / GIMP)");
                ui.checkbox(&mut self.opts.cmo3, "Cubism project (.cmo3 + .can3)");
                ui.checkbox(&mut self.opts.inochi, "Inochi2D (.inp + .inx)");
                ui.add_space(4.0);
                ui.checkbox(&mut self.opts.all_layers, "Include hidden / animation-only layers")
                    .on_hover_text("PSD and ORA: show every mesh, even ones invisible at the default pose.");
                ui.checkbox(&mut self.opts.cmo3_visible, "Force everything visible in .cmo3")
                    .on_hover_text("Breaks hair/clothing toggles. Leave off unless you only want to see all art.");
                ui.add_space(4.0);
                ui.label(RichText::new("Output folder").weak());
                ui.label(self.opts.out_dir.display().to_string());
                if ui.button("Change...").clicked() {
                    if let Some(d) = rfd::FileDialog::new().pick_folder() {
                        self.opts.out_dir = d;
                    }
                }
            });
        });

        if self.about {
            let mut open = true;
            egui::Window::new("About MINT").open(&mut open).resizable(false).show(ctx, |ui| {
                ui.label(RichText::new("MINT — Moc3 INochi Translator").strong());
                ui.label("Recovers editable projects (PSD, ORA, cmo3, Inochi2D) from compiled moc3 models.");
                ui.label("Clean-room implementation — no Live2D code is used.");
                ui.add_space(4.0);
                ui.label("License: PolyForm Noncommercial 1.0.0 — noncommercial use only.");
                ui.separator();
                ui.label(RichText::new("Third-party components").strong());
                for (name, lic) in ABOUT_LIBS {
                    ui.label(format!("{name} — {lic}"));
                }
            });
            if !open {
                self.about = false;
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(tex) = &self.texture else {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new(if self.is_loading { "Loading..." } else { "Drop a .moc3 here" }).weak().size(20.0));
                });
                return;
            };
            let size = tex.size_vec2();
            ui.horizontal(|ui| {
                ui.label("Preview (default pose)");
                if ui.button("Fit").clicked() {
                    self.zoom = None;
                }
                let mut z = self.zoom.unwrap_or(1.0);
                if ui.add(egui::Slider::new(&mut z, 0.1..=3.0).text("zoom")).changed() {
                    self.zoom = Some(z);
                }
            });
            let avail = ui.available_size();
            let scale = self.zoom.unwrap_or_else(|| (avail.x / size.x).min(avail.y / size.y).min(1.0));
            egui::ScrollArea::both().show(ui, |ui| {
                ui.add(egui::Image::new((tex.id(), size * scale)));
            });
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 800.0]).with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native("MINT", options, Box::new(|_| Ok(Box::new(App::default()))))
}
