mod geometry;
mod texture;
mod xml;

use std::collections::HashMap;
use std::path::Path;

use crate::{
    Error, Result, can3::Can3Project, cmo3::Cmo3Project, displayinfo3::DisplayInfo3,
    expression3::Expression3, moc3::Moc3Model, model3::Model3, motion3::Motion3,
    physics3::Physics3, pose3::Pose3, userdata3::UserData3,
};

pub use texture::Texture;

/// When set, every mesh/deformer keyform is written fully opaque so that
/// hidden or animation-only artwork is visible in the editor.
pub(crate) static FORCE_VISIBLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Display names and parameter grouping recovered from a `cdi3.json`
/// (display-info) document. Restores the names the editor originally showed
/// for parameters and parts, plus the parameter folder layout.
#[derive(Debug, Clone, Default)]
pub struct DisplayNames {
    /// Parameter id -> editor display name.
    pub parameter_names: HashMap<String, String>,
    /// Parameter groups as `(id, parent group id, name)` in file order.
    pub parameter_groups: Vec<(String, String, String)>,
    /// Parameter id -> parameter group id.
    pub parameter_group_of: HashMap<String, String>,
    /// Part id -> editor display name.
    pub part_names: HashMap<String, String>,
    /// Part ids to mark hidden in the editor (e.g. alternate arm sets
    /// that a pose3.json hides at startup).
    pub hidden_parts: std::collections::HashSet<String>,
}

impl DisplayNames {
    /// Builds display names from a parsed `cdi3.json` document.
    pub fn from_display_info(info: &DisplayInfo3) -> Self {
        Self {
            parameter_names: info
                .parameters()
                .iter()
                .filter(|p| !p.name.is_empty())
                .map(|p| (p.id.clone(), p.name.clone()))
                .collect(),
            parameter_groups: info
                .parameter_groups()
                .iter()
                .map(|g| (g.id.clone(), g.group_id.clone(), g.name.clone()))
                .collect(),
            parameter_group_of: info
                .parameters()
                .iter()
                .filter(|p| !p.group_id.is_empty())
                .map(|p| (p.id.clone(), p.group_id.clone()))
                .collect(),
            part_names: info
                .parts()
                .iter()
                .filter(|p| !p.name.is_empty())
                .map(|p| (p.id.clone(), p.name.clone()))
                .collect(),
            hidden_parts: Default::default(),
        }
    }
}

/// Converts MOC3 bytes into a CMO3 project.
#[derive(Debug, Clone)]
pub struct Decompiler {
    force_visible: bool,
    deformer_parts: Vec<Option<usize>>,
    draw_ranks: Vec<i32>,
    model_name: String,
    textures: Vec<Texture>,
    obfuscation_key: i32,
    display_names: DisplayNames,
    icons: Vec<(u32, Vec<u8>)>,
}

impl Default for Decompiler {
    fn default() -> Self {
        Self {
            force_visible: false,
            deformer_parts: Vec::new(),
            draw_ranks: Vec::new(),
            model_name: "Decompiled Model".into(),
            textures: Vec::new(),
            obfuscation_key: 42,
            display_names: DisplayNames::default(),
            icons: Vec::new(),
        }
    }
}

impl Decompiler {
    /// Creates a decompiler with default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Forces all keyform opacities to 1 so hidden artwork is visible.
    pub fn set_force_visible(&mut self, on: bool) {
        self.force_visible = on;
    }

    /// Default-pose stacking rank per art mesh (0 = back); keeps draw order
    /// correct across MOC3 draw-order groups.
    pub fn set_draw_ranks(&mut self, ranks: Vec<i32>) {
        self.draw_ranks = ranks;
    }

    /// Parent part index per deformer (MOC3 deformer order).
    pub fn set_deformer_parts(&mut self, parts: Vec<Option<usize>>) {
        self.deformer_parts = parts;
    }

    /// Sets the display name stored in the generated project.
    pub fn set_model_name(&mut self, name: impl Into<String>) {
        self.model_name = name.into();
    }

    /// Replaces the texture atlas pages in MOC3 texture-index order.
    ///
    /// Missing pages are replaced with a transparent placeholder. Extra pages
    /// are retained in the generated project.
    pub fn set_textures(&mut self, textures: impl IntoIterator<Item = Texture>) {
        self.textures = textures.into_iter().collect();
    }

    /// Adds one texture atlas page.
    pub fn push_texture(&mut self, texture: Texture) {
        self.textures.push(texture);
    }

    /// Sets the signed CAFF XOR key.
    pub fn set_obfuscation_key(&mut self, key: i32) {
        self.obfuscation_key = key;
    }

    /// Sets the editor display names recovered from a `cdi3.json` document.
    pub fn set_display_names(&mut self, names: DisplayNames) {
        self.display_names = names;
    }

    /// Sets the model icon PNGs emitted as `_icon64`/`_icon32`/`_icon16`.
    ///
    /// Each entry is `(edge length in pixels, png bytes)`; only 64, 32 and 16
    /// are wired into the document.
    pub fn set_icons(&mut self, icons: impl IntoIterator<Item = (u32, Vec<u8>)>) {
        self.icons = icons.into_iter().collect();
    }

    /// Parses the MOC3 model and builds an unencoded CMO3 project.
    ///
    /// The project contains a readable `main.xml`, the supplied atlas pages,
    /// and `moc2cmo.model.json` with the recovered runtime data.
    ///
    /// # Errors
    ///
    /// Returns an error when the MOC3 data is invalid or a project resource
    /// cannot be serialized.
    pub fn decompile_project(&self, moc3: &[u8]) -> Result<Cmo3Project> {
        let model = Moc3Model::parse(moc3)?;
        let textures = texture::complete_texture_set(&model, &self.textures);
        FORCE_VISIBLE.store(self.force_visible, std::sync::atomic::Ordering::Relaxed);
        let main_xml = xml::generate(
            &model,
            &self.model_name,
            &textures,
            &self.deformer_parts,
            &self.draw_ranks,
            &self.display_names,
            &self.icons,
        )?;
        let mut project = Cmo3Project::new(main_xml);
        project.set_obfuscation_key(self.obfuscation_key);
        for (index, texture) in textures.iter().enumerate() {
            project.insert_resource(format!("imageFileBuf_{index}.png"), texture.png().to_vec());
        }
        for (size, png) in &self.icons {
            project.insert_resource(format!("image_icon{size}.png"), png.clone());
        }
        let manifest = serde_json::to_vec_pretty(&model).map_err(|error| {
            Error::InvalidCmo3(format!("manifest serialization failed: {error}"))
        })?;
        project.insert_resource("moc2cmo.model.json", manifest);
        Ok(project)
    }

    /// Decompiles a MOC3 model into encoded `.cmo3` bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the input cannot be parsed or the CMO3 archive
    /// cannot be encoded.
    pub fn decompile(&self, moc3: &[u8]) -> Result<Vec<u8>> {
        self.decompile_project(moc3)?.encode()
    }

    /// Decompiles and writes a `.cmo3` file.
    ///
    /// # Errors
    ///
    /// Returns an error when conversion fails or the destination cannot be
    /// written.
    pub fn decompile_to_file(&self, moc3: &[u8], path: impl AsRef<Path>) -> Result<()> {
        self.decompile_project(moc3)?.write_to(path)
    }
}

/// Decompiles a MOC3 model with default options.
///
/// Texture pages are replaced with transparent placeholders. Use
/// [`Decompiler`] when atlas PNGs are available.
///
/// # Errors
///
/// Returns an error when the input cannot be parsed or the CMO3 archive cannot
/// be encoded.
pub fn decompile(moc3: &[u8]) -> Result<Vec<u8>> {
    Decompiler::new().decompile(moc3)
}

/// Decompiles a MOC3 model with default options and writes it to disk.
///
/// Texture pages are replaced with transparent placeholders. Use
/// [`Decompiler`] when atlas PNGs are available.
///
/// # Errors
///
/// Returns an error when conversion fails or the destination cannot be
/// written.
pub fn decompile_to_file(moc3: &[u8], path: impl AsRef<Path>) -> Result<()> {
    Decompiler::new().decompile_to_file(moc3, path)
}

/// The in-memory projects produced from one `model3.json` manifest.
#[derive(Debug, Clone)]
pub struct Model3Decompilation {
    /// Manifest file stem used for generated filenames.
    pub model_name: String,
    /// Generated editable CMO3 project, including referenced JSON resources.
    pub cmo3: Cmo3Project,
    /// Generated editable CAN3 project, including referenced sound resources.
    pub can3: Can3Project,
}

/// Reads a Cubism `model3.json` manifest and builds matching projects in memory.
///
/// All paths in the manifest are resolved relative to the manifest's directory.
/// The returned projects can be encoded or written by the caller.
///
/// # Errors
///
/// Returns an error when a referenced model, texture, auxiliary JSON, motion,
/// sound, or project resource cannot be read or parsed.
pub fn decompile_model3(model3_path: impl AsRef<Path>) -> Result<Model3Decompilation> {
    build_model3_projects(model3_path.as_ref(), Decompiler::new())
}

/// Reads a `model3.json` manifest using a caller-configured decompiler.
///
/// Behaves like [`decompile_model3`] but honours the decompiler's textures,
/// visibility, draw-rank, and deformer-parent options. Display info parsed
/// from the manifest is applied on top of any configured display names.
pub fn decompile_model3_with(
    model3_path: impl AsRef<Path>,
    decompiler: Decompiler,
) -> Result<Model3Decompilation> {
    build_model3_projects(model3_path.as_ref(), decompiler)
}

/// Reads a Cubism `model3.json` manifest and writes matching CMO3 and CAN3 files.
///
/// All paths in the manifest are resolved relative to the manifest's directory.
/// The output directory is created when it does not exist. The generated files
/// use the manifest file stem as their base name.
///
/// # Errors
///
/// Returns an error when a referenced model, texture, motion, or output file
/// cannot be read or written.
pub fn decompile_model3_to_files(
    model3_path: impl AsRef<Path>,
    output_directory: impl AsRef<Path>,
) -> Result<()> {
    let projects = decompile_model3(model3_path.as_ref())?;
    let output_directory = output_directory.as_ref();
    std::fs::create_dir_all(output_directory).map_err(|source| Error::Io {
        path: output_directory.to_path_buf(),
        source,
    })?;
    projects
        .cmo3
        .write_to(output_directory.join(format!("{}.cmo3", projects.model_name)))?;
    projects
        .can3
        .write_to(output_directory.join(format!("{}.can3", projects.model_name)))
}

fn build_model3_projects(model3_path: &Path, mut decompiler: Decompiler) -> Result<Model3Decompilation> {
    let base_directory = model3_path.parent().unwrap_or_else(|| Path::new("."));
    let manifest_bytes = std::fs::read(model3_path).map_err(|source| Error::Io {
        path: model3_path.to_path_buf(),
        source,
    })?;
    let manifest = Model3::from_json_bytes(&manifest_bytes)?;
    let references = manifest.file_references();
    let moc3_path = base_directory.join(&references.moc);
    let moc3 = std::fs::read(&moc3_path).map_err(|source| Error::Io {
        path: moc3_path.clone(),
        source,
    })?;

    for path in [
        references.physics.clone(),
        references.pose.clone(),
        references.user_data.clone(),
        references.display_info.clone(),
    ]
    .into_iter()
    .flatten()
    {
        let path = base_directory.join(path);
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("warning: skipping missing referenced file {}", path.display());
            continue;
        };
        let path_text = path.to_string_lossy();
        if path_text.ends_with(".physics3.json") {
            Physics3::from_json_bytes(&bytes)?;
        } else if path_text.ends_with(".pose3.json") {
            Pose3::from_json_bytes(&bytes)?;
        } else if path_text.ends_with(".userdata3.json") {
            UserData3::from_json_bytes(&bytes)?;
        } else if path_text.ends_with(".cdi3.json") {
            let info = DisplayInfo3::from_json_bytes(&bytes)?;
            let hidden = std::mem::take(&mut decompiler.display_names.hidden_parts);
            let mut names = DisplayNames::from_display_info(&info);
            names.hidden_parts = hidden;
            decompiler.set_display_names(names);
        } else {
            let _ = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|error| {
                Error::InvalidJson {
                    format: "model3 auxiliary JSON",
                    message: error.to_string(),
                }
            })?;
        }
    }
    for expression in &references.expressions {
        let path = base_directory.join(&expression.file);
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("warning: skipping missing expression {}", path.display());
            continue;
        };
        Expression3::from_json_bytes(&bytes)?;
    }

    let model_name = model3_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("model");
    decompiler.set_model_name(model_name);
    if decompiler.textures.is_empty() {
        for texture in &references.textures {
            let path = base_directory.join(texture);
            let bytes = std::fs::read(&path).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
            decompiler.push_texture(Texture::from_png(bytes)?);
        }
    }

    let mut cmo3 = decompiler.decompile_project(&moc3)?;
    cmo3.insert_resource("moc2cmo.model3.json", manifest_bytes);
    for resource in referenced_json_paths(references) {
        let path = base_directory.join(&resource);
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        cmo3.insert_resource(resource, bytes);
    }
    // Preserve auxiliary JSON the manifest does not reference (e.g. VTube
    // Studio expression files dropped from model3.json) as project resources.
    const AUX_SUFFIXES: [&str; 6] = [
        ".exp3.json",
        ".vtube.json",
        ".pose3.json",
        ".userdata3.json",
        ".cdi3.json",
        ".physics3.json",
    ];
    if let Ok(entries) = std::fs::read_dir(base_directory) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !AUX_SUFFIXES.iter().any(|suffix| name.ends_with(suffix)) {
                continue;
            }
            if let Ok(bytes) = std::fs::read(entry.path()) {
                cmo3.insert_resource(name, bytes);
            }
        }
    }
    let mut motions = Vec::new();
    for (group, entries) in &references.motions {
        for (index, entry) in entries.iter().enumerate() {
            let path = base_directory.join(&entry.file);
            let Ok(bytes) = std::fs::read(&path) else {
                eprintln!("warning: skipping missing motion {}", path.display());
                continue;
            };
            let motion = Motion3::from_json_bytes(&bytes)?;
            motions.push(crate::can3::MotionInstance {
                name: format!("{group}_{index}"),
                motion,
                fade_in_time: entry.fade_in_time,
                fade_out_time: entry.fade_out_time,
            });
        }
    }
    let model_file_name = format!("{model_name}.cmo3");
    let mut can3 =
        Can3Project::from_model3(model_name, &model_file_name, &motions, manifest.groups())?;
    for entries in references.motions.values() {
        for entry in entries {
            if let Some(sound) = &entry.sound {
                let path = base_directory.join(sound);
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                can3.insert_resource(sound.clone(), bytes)?;
            }
        }
    }
    Ok(Model3Decompilation {
        model_name: model_name.to_owned(),
        cmo3,
        can3,
    })
}

fn referenced_json_paths(references: &crate::model3::Model3References) -> Vec<String> {
    let mut paths = Vec::new();
    for path in [
        references.physics.clone(),
        references.pose.clone(),
        references.user_data.clone(),
        references.display_info.clone(),
    ]
    .into_iter()
    .flatten()
    {
        paths.push(path);
    }
    paths.extend(
        references
            .expressions
            .iter()
            .map(|expression| expression.file.clone()),
    );
    paths
}
