//! Validated external effect package manifests.

use std::collections::HashSet;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const EFFECT_MANIFEST_FORMAT: &str = "oneiroi-effect";
pub const EFFECT_MANIFEST_VERSION: u32 = 1;
const MAX_PARAMETERS: usize = 32;
const MAX_PACKAGES: usize = 128;
pub const MAX_EFFECT_PASSES: usize = 2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectManifest {
    pub format: String,
    pub version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub role: EffectPackageRole,
    pub shader: PathBuf,
    #[serde(default = "default_vertex_entry")]
    pub vertex_entry: String,
    #[serde(default = "default_fragment_entry")]
    pub fragment_entry: String,
    #[serde(default)]
    pub passes: Vec<EffectPassSchema>,
    #[serde(default)]
    pub resources: EffectResourceSchema,
    pub parameters: Vec<EffectParameterSchema>,
}

impl EffectManifest {
    pub fn pass_entries(&self) -> Vec<&str> {
        if self.passes.is_empty() {
            vec![self.fragment_entry.as_str()]
        } else {
            self.passes
                .iter()
                .map(|pass| pass.fragment_entry.as_str())
                .collect()
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectPackageRole {
    #[default]
    MasterEffect,
    MasterProcessor,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectHistoryResource {
    #[default]
    None,
    PreviousSlotOutput,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct EffectResourceSchema {
    #[serde(default)]
    pub history: EffectHistoryResource,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectParameterSchema {
    pub id: String,
    pub label: String,
    pub minimum: f32,
    pub maximum: f32,
    pub default: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectPassSchema {
    pub fragment_entry: String,
}

#[derive(Clone, Debug)]
pub struct ValidatedEffectPackage {
    pub manifest: EffectManifest,
    pub manifest_path: PathBuf,
    pub shader_path: PathBuf,
    pub shader_source: String,
    pub fingerprint: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectDescriptor {
    pub id: String,
    pub name: String,
    pub manifest_path: PathBuf,
    pub parameters: Vec<EffectParameterSchema>,
    pub pass_count: usize,
    pub history: EffectHistoryResource,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectRegistry {
    pub effects: Vec<EffectDescriptor>,
    pub errors: Vec<String>,
}

#[derive(Debug, Error)]
pub enum EffectManifestError {
    #[error("read effect manifest {path}: {source}")]
    ReadManifest {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("decode effect manifest {path}: {source}")]
    DecodeManifest {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("invalid effect manifest: {0}")]
    Invalid(String),
    #[error("read effect shader {path}: {source}")]
    ReadShader {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("WGSL parse failed: {0}")]
    ParseShader(String),
    #[error("WGSL validation failed: {0}")]
    ValidateShader(String),
}

pub fn load_effect_package(
    manifest_path: impl AsRef<Path>,
) -> Result<ValidatedEffectPackage, EffectManifestError> {
    let manifest_path = manifest_path.as_ref();
    let manifest_source =
        fs::read_to_string(manifest_path).map_err(|source| EffectManifestError::ReadManifest {
            path: manifest_path.to_path_buf(),
            source,
        })?;
    let manifest: EffectManifest = serde_json::from_str(&manifest_source).map_err(|source| {
        EffectManifestError::DecodeManifest {
            path: manifest_path.to_path_buf(),
            source,
        }
    })?;
    validate_manifest(&manifest)?;
    let parent = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let shader_path = parent.join(&manifest.shader);
    let shader_source =
        fs::read_to_string(&shader_path).map_err(|source| EffectManifestError::ReadShader {
            path: shader_path.clone(),
            source,
        })?;
    validate_shader(&manifest, &shader_source)?;
    let mut hasher = DefaultHasher::new();
    manifest_source.hash(&mut hasher);
    shader_source.hash(&mut hasher);
    Ok(ValidatedEffectPackage {
        manifest,
        manifest_path: manifest_path.to_path_buf(),
        shader_path,
        shader_source,
        fingerprint: hasher.finish(),
    })
}

pub fn discover_effect_packages(root: impl AsRef<Path>) -> EffectRegistry {
    let root = root.as_ref();
    let mut manifest_paths = Vec::new();
    if root.join("effect.json").is_file() {
        manifest_paths.push(root.join("effect.json"));
    }
    match fs::read_dir(root) {
        Ok(entries) => {
            for entry in entries.flatten().take(MAX_PACKAGES) {
                let path = entry.path().join("effect.json");
                if path.is_file() {
                    manifest_paths.push(path);
                }
            }
        }
        Err(error) => {
            return EffectRegistry {
                effects: Vec::new(),
                errors: vec![format!("scan effect directory {}: {error}", root.display())],
            };
        }
    }
    manifest_paths.sort();
    manifest_paths.dedup();

    let mut registry = EffectRegistry::default();
    let mut ids = HashSet::new();
    for path in manifest_paths {
        match load_effect_package(&path) {
            Ok(package) => {
                if package.manifest.role != EffectPackageRole::MasterEffect {
                    continue;
                }
                if !ids.insert(package.manifest.id.clone()) {
                    registry.errors.push(format!(
                        "duplicate effect id {:?} at {}",
                        package.manifest.id,
                        path.display()
                    ));
                    continue;
                }
                let pass_count = package.manifest.pass_entries().len();
                let history = package.manifest.resources.history;
                registry.effects.push(EffectDescriptor {
                    id: package.manifest.id,
                    name: package.manifest.name,
                    manifest_path: path,
                    parameters: package.manifest.parameters,
                    pass_count,
                    history,
                });
            }
            Err(error) => registry.errors.push(format!("{}: {error}", path.display())),
        }
    }
    registry
        .effects
        .sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    registry
}

fn validate_manifest(manifest: &EffectManifest) -> Result<(), EffectManifestError> {
    if manifest.format != EFFECT_MANIFEST_FORMAT {
        return Err(EffectManifestError::Invalid(format!(
            "format must be {EFFECT_MANIFEST_FORMAT:?}"
        )));
    }
    if manifest.version != EFFECT_MANIFEST_VERSION {
        return Err(EffectManifestError::Invalid(format!(
            "unsupported version {}",
            manifest.version
        )));
    }
    if !valid_id(&manifest.id) {
        return Err(EffectManifestError::Invalid(
            "id must contain 1–64 lowercase ASCII letters, digits or hyphens".to_owned(),
        ));
    }
    if manifest.name.trim().is_empty() || manifest.name.len() > 128 {
        return Err(EffectManifestError::Invalid(
            "name must contain 1–128 characters".to_owned(),
        ));
    }
    if manifest.shader.extension().and_then(|value| value.to_str()) != Some("wgsl")
        || manifest
            .shader
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(EffectManifestError::Invalid(
            "shader must be a package-relative .wgsl path without traversal".to_owned(),
        ));
    }
    if !valid_shader_identifier(&manifest.vertex_entry)
        || !valid_shader_identifier(&manifest.fragment_entry)
        || manifest
            .passes
            .iter()
            .any(|pass| !valid_shader_identifier(&pass.fragment_entry))
    {
        return Err(EffectManifestError::Invalid(
            "entry points must be valid WGSL identifiers".to_owned(),
        ));
    }
    if manifest.passes.len() > MAX_EFFECT_PASSES {
        return Err(EffectManifestError::Invalid(format!(
            "custom effect pass count cannot exceed {MAX_EFFECT_PASSES}"
        )));
    }
    if manifest.role == EffectPackageRole::MasterProcessor && manifest.passes.len() > 1 {
        return Err(EffectManifestError::Invalid(
            "master_processor packages must declare exactly one pipeline".to_owned(),
        ));
    }
    if manifest.parameters.is_empty() || manifest.parameters.len() > MAX_PARAMETERS {
        return Err(EffectManifestError::Invalid(format!(
            "parameter count must be 1–{MAX_PARAMETERS}"
        )));
    }
    let mut ids = HashSet::new();
    for parameter in &manifest.parameters {
        if !valid_id(&parameter.id) || !ids.insert(parameter.id.as_str()) {
            return Err(EffectManifestError::Invalid(format!(
                "parameter id {:?} is invalid or duplicated",
                parameter.id
            )));
        }
        if parameter.label.trim().is_empty()
            || !parameter.minimum.is_finite()
            || !parameter.maximum.is_finite()
            || !parameter.default.is_finite()
            || parameter.minimum >= parameter.maximum
            || !(parameter.minimum..=parameter.maximum).contains(&parameter.default)
        {
            return Err(EffectManifestError::Invalid(format!(
                "parameter {:?} has an invalid label or range",
                parameter.id
            )));
        }
    }
    Ok(())
}

fn validate_shader(
    manifest: &EffectManifest,
    shader_source: &str,
) -> Result<(), EffectManifestError> {
    let module = naga::front::wgsl::parse_str(shader_source)
        .map_err(|error| EffectManifestError::ParseShader(error.emit_to_string(shader_source)))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|error| EffectManifestError::ValidateShader(error.to_string()))?;
    let vertex_found = module.entry_points.iter().any(|entry| {
        entry.name == manifest.vertex_entry && entry.stage == naga::ShaderStage::Vertex
    });
    let fragments_found = manifest.pass_entries().into_iter().all(|fragment_entry| {
        module
            .entry_points
            .iter()
            .any(|entry| entry.name == fragment_entry && entry.stage == naga::ShaderStage::Fragment)
    });
    if !vertex_found || !fragments_found {
        return Err(EffectManifestError::Invalid(
            "declared vertex/fragment entry points do not exist with the required stages"
                .to_owned(),
        ));
    }
    Ok(())
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_shader_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn default_vertex_entry() -> String {
    "vs_main".to_owned()
}

fn default_fragment_entry() -> String {
    "fs_main".to_owned()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn fixture(manifest: &str, shader: &str) -> PathBuf {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "oneiroi-effect-manifest-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("effect.json"), manifest).unwrap();
        fs::write(directory.join("effect.wgsl"), shader).unwrap();
        directory.join("effect.json")
    }

    fn manifest(parameters: &str) -> String {
        format!(
            r#"{{
  "format": "oneiroi-effect",
  "version": 1,
  "id": "master-effects",
  "name": "Master effects",
  "shader": "effect.wgsl",
  "parameters": [{parameters}]
}}"#
        )
    }

    fn parameters() -> &'static str {
        r#"
    {"id":"radius","label":"Radius","minimum":0.0,"maximum":32.0,"default":8.0},
    {"id":"mix","label":"Mix","minimum":0.0,"maximum":1.0,"default":1.0},
    {"id":"feedback","label":"Feedback","minimum":0.0,"maximum":0.99,"default":0.85}
        "#
    }

    #[test]
    fn loads_a_valid_manifest_and_shader_contract() {
        let path = fixture(
            &manifest(parameters()),
            r#"
@vertex fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    return vec4(f32(index), 0.0, 0.0, 1.0);
}
@fragment fn fs_main() -> @location(0) vec4<f32> {
    return vec4(1.0);
}
"#,
        );
        let package = load_effect_package(&path).unwrap();
        assert_eq!(package.manifest.id, "master-effects");
        assert_ne!(package.fingerprint, 0);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn bundled_master_effect_package_stays_valid() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../effects/master-effects/effect.json");
        let package = load_effect_package(path).unwrap();
        assert_eq!(package.manifest.id, "master-effects");
    }

    #[test]
    fn bundled_registry_discovers_chromatic_split() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../effects");
        let registry = discover_effect_packages(root);
        assert!(registry.errors.is_empty(), "{:?}", registry.errors);
        assert_eq!(registry.effects.len(), 3);
        assert!(
            registry
                .effects
                .iter()
                .any(|effect| { effect.id == "chromatic-split" && effect.pass_count == 1 })
        );
        assert!(
            registry
                .effects
                .iter()
                .any(|effect| { effect.id == "spectral-echo" && effect.pass_count == 2 })
        );
        assert!(registry.effects.iter().any(|effect| {
            effect.id == "temporal-melt"
                && effect.history == EffectHistoryResource::PreviousSlotOutput
        }));
    }

    #[test]
    fn rejects_duplicate_parameters_and_malformed_wgsl() {
        let duplicated = format!("{},{}", parameters(), parameters());
        let path = fixture(&manifest(&duplicated), "not wgsl");
        assert!(matches!(
            load_effect_package(&path),
            Err(EffectManifestError::Invalid(_))
        ));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();

        let path = fixture(&manifest(parameters()), "not wgsl");
        assert!(matches!(
            load_effect_package(&path),
            Err(EffectManifestError::ParseShader(_))
        ));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn rejects_shader_path_traversal() {
        let mut value: serde_json::Value = serde_json::from_str(&manifest(parameters())).unwrap();
        value["shader"] = serde_json::json!("../effect.wgsl");
        let path = fixture(&serde_json::to_string(&value).unwrap(), "not used");
        assert!(matches!(
            load_effect_package(&path),
            Err(EffectManifestError::Invalid(_))
        ));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn rejects_unbounded_or_missing_pass_entry_points() {
        let mut value: serde_json::Value = serde_json::from_str(&manifest(parameters())).unwrap();
        value["passes"] = serde_json::json!([
            {"fragment_entry":"fs_main"},
            {"fragment_entry":"fs_main"},
            {"fragment_entry":"fs_main"}
        ]);
        let path = fixture(&serde_json::to_string(&value).unwrap(), "not used");
        assert!(matches!(
            load_effect_package(&path),
            Err(EffectManifestError::Invalid(_))
        ));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();

        value["passes"] = serde_json::json!([{"fragment_entry":"missing"}]);
        let path = fixture(
            &serde_json::to_string(&value).unwrap(),
            r#"
@vertex fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    return vec4(f32(index), 0.0, 0.0, 1.0);
}
@fragment fn fs_main() -> @location(0) vec4<f32> {
    return vec4(1.0);
}
"#,
        );
        assert!(matches!(
            load_effect_package(&path),
            Err(EffectManifestError::Invalid(_))
        ));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn discovers_only_custom_master_effects() {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "oneiroi-effect-registry-{}-{id}",
            std::process::id()
        ));
        for (name, role) in [
            ("custom", "master_effect"),
            ("processor", "master_processor"),
        ] {
            let directory = root.join(name);
            fs::create_dir_all(&directory).unwrap();
            let mut value: serde_json::Value =
                serde_json::from_str(&manifest(parameters())).unwrap();
            value["id"] = serde_json::json!(name);
            value["name"] = serde_json::json!(name);
            value["role"] = serde_json::json!(role);
            fs::write(
                directory.join("effect.json"),
                serde_json::to_vec(&value).unwrap(),
            )
            .unwrap();
            fs::write(
                directory.join("effect.wgsl"),
                r#"
@vertex fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    return vec4(f32(index), 0.0, 0.0, 1.0);
}
@fragment fn fs_main() -> @location(0) vec4<f32> {
    return vec4(1.0);
}
"#,
            )
            .unwrap();
        }
        let registry = discover_effect_packages(&root);
        assert!(registry.errors.is_empty(), "{:?}", registry.errors);
        assert_eq!(registry.effects.len(), 1);
        assert_eq!(registry.effects[0].id, "custom");
        fs::remove_dir_all(root).unwrap();
    }
}
