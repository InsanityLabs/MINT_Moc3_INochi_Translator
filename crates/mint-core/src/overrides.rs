//! User edits kept next to a model (`<name>.mint.json`): renamed parts, layers
//! and parameters, which X/Y parameters are combined, and (optionally) which
//! parts/layers are hidden by default. Exporters apply them; nothing here can
//! add or remove model data.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overrides {
    pub part_names: HashMap<String, String>,
    pub mesh_names: HashMap<String, String>,
    pub param_names: HashMap<String, String>,
    /// `None` = automatic (Cubism's standard pairs); `Some` = exactly these
    /// `(x parameter id, y parameter id)` pairs.
    pub pairs: Option<Vec<(String, String)>>,
    /// Editor groups of 3 or more adjacent parameters (parameter ids). Cubism and
    /// Inochi only have two-axis controls, so exports use the first two members
    /// of each group (through `pairs`) and leave the rest as separate sliders.
    pub groups: Vec<Vec<String>>,
    pub hidden_parts: HashSet<String>,
    pub hidden_meshes: HashSet<String>,
    /// Use `hidden_parts` / `hidden_meshes` as the default visibility in exports.
    pub apply_visibility: bool,
}

fn map_to_json(m: &HashMap<String, String>) -> Value {
    let mut keys: Vec<&String> = m.keys().collect();
    keys.sort();
    Value::Object(keys.into_iter().map(|k| (k.clone(), Value::String(m[k].clone()))).collect())
}

fn json_to_map(v: &Value) -> HashMap<String, String> {
    v.as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| v.as_str().filter(|s| !s.is_empty()).map(|s| (k.clone(), s.to_string())))
        .collect()
}

fn set_to_json(s: &HashSet<String>) -> Value {
    let mut v: Vec<&String> = s.iter().collect();
    v.sort();
    json!(v)
}

fn json_to_set(v: &Value) -> HashSet<String> {
    v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect()
}

impl Overrides {
    pub fn path_for(moc_path: &Path) -> PathBuf {
        moc_path.with_extension("mint.json")
    }

    pub fn is_empty(&self) -> bool {
        *self == Overrides::default()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "version": 1,
            "part_names": map_to_json(&self.part_names),
            "mesh_names": map_to_json(&self.mesh_names),
            "param_names": map_to_json(&self.param_names),
            "pairs": self.pairs.as_ref().map(|p| p.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>()),
            "groups": self.groups,
            "hidden_parts": set_to_json(&self.hidden_parts),
            "hidden_meshes": set_to_json(&self.hidden_meshes),
            "apply_visibility": self.apply_visibility,
        })
    }

    pub fn from_json(v: &Value) -> Overrides {
        Overrides {
            part_names: json_to_map(&v["part_names"]),
            mesh_names: json_to_map(&v["mesh_names"]),
            param_names: json_to_map(&v["param_names"]),
            pairs: v["pairs"].as_array().map(|a| {
                a.iter()
                    .filter_map(|p| Some((p[0].as_str()?.to_string(), p[1].as_str()?.to_string())))
                    .collect()
            }),
            groups: v["groups"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|g| g.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect())
                .collect(),
            hidden_parts: json_to_set(&v["hidden_parts"]),
            hidden_meshes: json_to_set(&v["hidden_meshes"]),
            apply_visibility: v["apply_visibility"].as_bool().unwrap_or(false),
        }
    }

    pub fn from_bytes(b: &[u8]) -> Option<Overrides> {
        serde_json::from_slice::<Value>(b).ok().map(|v| Overrides::from_json(&v))
    }

    pub fn load(moc_path: &Path) -> Option<Overrides> {
        std::fs::read(Self::path_for(moc_path)).ok().and_then(|b| Self::from_bytes(&b))
    }

    pub fn save(&self, moc_path: &Path) -> std::io::Result<()> {
        let path = Self::path_for(moc_path);
        if self.is_empty() {
            return match std::fs::remove_file(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        std::fs::write(path, serde_json::to_vec_pretty(&self.to_json()).unwrap_or_default())
    }

    pub fn part_name<'a>(&'a self, id: &'a str) -> &'a str {
        self.part_names.get(id).map_or(id, String::as_str)
    }

    pub fn mesh_name<'a>(&'a self, id: &'a str) -> &'a str {
        self.mesh_names.get(id).map_or(id, String::as_str)
    }

    /// Explicit parameter pairs as indices, if the user chose them.
    pub fn pair_indices(&self, moc: &crate::Moc3) -> Option<Vec<(usize, usize)>> {
        let find = |id: &str| moc.parameters.iter().position(|p| p.id == id);
        self.pairs.as_ref().map(|p| p.iter().filter_map(|(a, b)| Some((find(a)?, find(b)?))).collect())
    }

    /// Hide the parts / layers the user switched off (when that is enabled).
    pub fn apply_to_model(&self, model: &mut crate::Model) {
        if self.apply_visibility {
            model.hide_more(&self.hidden_parts, &self.hidden_meshes);
        }
    }
}
