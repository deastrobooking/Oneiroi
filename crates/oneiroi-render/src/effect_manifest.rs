//! Validated external effect package manifests.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const EFFECT_MANIFEST_FORMAT: &str = "oneiroi-effect";
pub const EFFECT_MANIFEST_VERSION: u32 = 2;
const MIN_EFFECT_MANIFEST_VERSION: u32 = 1;
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
    pub description: String,
    #[serde(default)]
    pub role: EffectPackageRole,
    /// Explicit placement metadata introduced by manifest v2. Manifest v1 is
    /// implicitly `master` and remains source compatible.
    #[serde(default)]
    pub targets: Vec<EffectPackageTarget>,
    /// Explicit shader binding contract introduced by manifest v2. Manifest v1
    /// implicitly uses `master-v1` when this field is absent.
    #[serde(default)]
    pub abi: Option<EffectPackageAbi>,
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
    #[serde(default)]
    pub presets: Vec<EffectPresetSchema>,
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

    pub fn resolved_targets(&self) -> Vec<EffectPackageTarget> {
        if self.version == 1 && self.role == EffectPackageRole::MasterEffect {
            vec![EffectPackageTarget::Master]
        } else {
            self.targets.clone()
        }
    }

    pub fn resolved_abi(&self) -> EffectPackageAbi {
        self.abi.unwrap_or(EffectPackageAbi::MasterV1)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectPackageRole {
    #[default]
    MasterEffect,
    MasterProcessor,
    Effect,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectPackageTarget {
    #[default]
    Master,
    Deck,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum EffectPackageAbi {
    #[default]
    #[serde(rename = "master-v1")]
    MasterV1,
    #[serde(rename = "deck-v1")]
    DeckV1,
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
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub control: EffectParameterControl,
    #[serde(default)]
    pub options: Vec<EffectParameterOption>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectParameterControl {
    #[default]
    Slider,
    Toggle,
    Choice,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectParameterOption {
    pub label: String,
    pub value: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectPresetSchema {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    pub values: BTreeMap<String, f32>,
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
    pub description: String,
    pub manifest_path: PathBuf,
    pub targets: Vec<EffectPackageTarget>,
    pub abi: EffectPackageAbi,
    pub parameters: Vec<EffectParameterSchema>,
    pub pass_count: usize,
    pub history: EffectHistoryResource,
    pub presets: Vec<EffectPresetSchema>,
}

impl EffectDescriptor {
    pub fn supports_target(&self, target: EffectPackageTarget) -> bool {
        self.targets.contains(&target)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectRegistry {
    pub effects: Vec<EffectDescriptor>,
    pub errors: Vec<String>,
    /// Manifest paths that were present but failed decode, schema or shader
    /// validation. Callers may use this to retain an already-compiled
    /// descriptor without confusing an intentional ID/role change with a
    /// transient invalid edit.
    pub failed_manifest_paths: Vec<PathBuf>,
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
                failed_manifest_paths: Vec::new(),
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
                if package.manifest.role == EffectPackageRole::MasterProcessor {
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
                let targets = package.manifest.resolved_targets();
                let abi = package.manifest.resolved_abi();
                registry.effects.push(EffectDescriptor {
                    id: package.manifest.id,
                    name: package.manifest.name,
                    description: package.manifest.description,
                    manifest_path: path,
                    targets,
                    abi,
                    parameters: package.manifest.parameters,
                    pass_count,
                    history,
                    presets: package.manifest.presets,
                });
            }
            Err(error) => {
                registry.failed_manifest_paths.push(path.clone());
                registry.errors.push(format!("{}: {error}", path.display()));
            }
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
    if !(MIN_EFFECT_MANIFEST_VERSION..=EFFECT_MANIFEST_VERSION).contains(&manifest.version) {
        return Err(EffectManifestError::Invalid(format!(
            "unsupported version {}; supported versions are {MIN_EFFECT_MANIFEST_VERSION}–{EFFECT_MANIFEST_VERSION}",
            manifest.version,
        )));
    }
    match manifest.version {
        1 => {
            if manifest.role == EffectPackageRole::Effect
                || !manifest.targets.is_empty()
                || matches!(manifest.abi, Some(EffectPackageAbi::DeckV1))
            {
                return Err(EffectManifestError::Invalid(
                    "manifest v1 is master-only; target metadata requires version 2".to_owned(),
                ));
            }
        }
        2 => {
            if manifest.role != EffectPackageRole::Effect {
                return Err(EffectManifestError::Invalid(
                    "manifest v2 selectable packages must use role \"effect\"".to_owned(),
                ));
            }
            let targets: HashSet<_> = manifest.targets.iter().copied().collect();
            if targets.len() != 1 || manifest.targets.len() != 1 {
                return Err(EffectManifestError::Invalid(
                    "manifest v2 must declare exactly one unique target".to_owned(),
                ));
            }
            let target = manifest.targets[0];
            let abi = manifest.abi.ok_or_else(|| {
                EffectManifestError::Invalid(
                    "manifest v2 must explicitly declare its shader ABI".to_owned(),
                )
            })?;
            if !matches!(
                (target, abi),
                (EffectPackageTarget::Master, EffectPackageAbi::MasterV1)
                    | (EffectPackageTarget::Deck, EffectPackageAbi::DeckV1)
            ) {
                return Err(EffectManifestError::Invalid(
                    "manifest target and shader ABI are incompatible".to_owned(),
                ));
            }
        }
        _ => unreachable!("version range checked above"),
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
    if manifest.version == 2
        && manifest.targets == [EffectPackageTarget::Deck]
        && (manifest.pass_entries().len() != 1
            || manifest.resources.history != EffectHistoryResource::None)
    {
        return Err(EffectManifestError::Invalid(
            "deck-v1 packages are stateless and must declare exactly one fragment pass".to_owned(),
        ));
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
        match parameter.control {
            EffectParameterControl::Slider if !parameter.options.is_empty() => {
                return Err(EffectManifestError::Invalid(format!(
                    "slider parameter {:?} cannot declare options",
                    parameter.id
                )));
            }
            EffectParameterControl::Toggle
                if !parameter.options.is_empty()
                    || parameter.minimum > 0.0
                    || parameter.maximum < 1.0 =>
            {
                return Err(EffectManifestError::Invalid(format!(
                    "toggle parameter {:?} must contain 0–1 and cannot declare options",
                    parameter.id
                )));
            }
            EffectParameterControl::Choice => {
                let mut values = Vec::new();
                if parameter.options.len() < 2
                    || parameter.options.iter().any(|option| {
                        option.label.trim().is_empty()
                            || !option.value.is_finite()
                            || !(parameter.minimum..=parameter.maximum).contains(&option.value)
                            || values.contains(&option.value)
                            || {
                                values.push(option.value);
                                false
                            }
                    })
                    || !values.contains(&parameter.default)
                {
                    return Err(EffectManifestError::Invalid(format!(
                        "choice parameter {:?} has invalid options",
                        parameter.id
                    )));
                }
            }
            EffectParameterControl::Slider | EffectParameterControl::Toggle => {}
        }
    }
    let mut preset_ids = HashSet::new();
    for preset in &manifest.presets {
        if !valid_id(&preset.id)
            || !preset_ids.insert(preset.id.as_str())
            || preset.label.trim().is_empty()
        {
            return Err(EffectManifestError::Invalid(format!(
                "preset id {:?} is invalid or duplicated",
                preset.id
            )));
        }
        for (parameter_id, value) in &preset.values {
            let Some(parameter) = manifest
                .parameters
                .iter()
                .find(|parameter| parameter.id == *parameter_id)
            else {
                return Err(EffectManifestError::Invalid(format!(
                    "preset {:?} targets unknown parameter {:?}",
                    preset.id, parameter_id
                )));
            };
            if !value.is_finite() || !(parameter.minimum..=parameter.maximum).contains(value) {
                return Err(EffectManifestError::Invalid(format!(
                    "preset {:?} value for {:?} is outside its range",
                    preset.id, parameter_id
                )));
            }
            if parameter.control == EffectParameterControl::Choice
                && !parameter
                    .options
                    .iter()
                    .any(|option| option.value == *value)
            {
                return Err(EffectManifestError::Invalid(format!(
                    "preset {:?} value for choice {:?} is not a declared option",
                    preset.id, parameter_id
                )));
            }
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
    fn bundled_registry_discovers_reference_and_algorithmic_effects() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../effects");
        let registry = discover_effect_packages(root);
        assert!(registry.errors.is_empty(), "{:?}", registry.errors);
        assert_eq!(registry.effects.len(), 6);
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
        for (id, parameter_count) in [
            ("recursive-2d", 14),
            ("fractal-volume", 16),
            ("hyper-recursion", 16),
        ] {
            let effect = registry
                .effects
                .iter()
                .find(|effect| effect.id == id)
                .unwrap_or_else(|| panic!("missing bundled algorithmic effect {id}"));
            assert!(effect.supports_target(EffectPackageTarget::Master), "{id}");
            assert_eq!(effect.abi, EffectPackageAbi::MasterV1, "{id}");
            assert_eq!(effect.pass_count, 1, "{id}");
            assert_eq!(effect.parameters.len(), parameter_count, "{id}");
            assert_eq!(effect.presets.len(), 3, "{id}");
            assert!(
                effect
                    .parameters
                    .iter()
                    .any(|parameter| parameter.control == EffectParameterControl::Choice),
                "{id} has no algorithm choice"
            );
            assert!(
                effect.parameters.iter().any(|parameter| {
                    parameter.id == "animate" && parameter.control == EffectParameterControl::Toggle
                }),
                "{id} has no animation toggle"
            );
        }
    }

    #[test]
    fn validates_v2_target_and_abi_compatibility() {
        let mut value: serde_json::Value = serde_json::from_str(&manifest(parameters())).unwrap();
        value["version"] = serde_json::json!(2);
        value["role"] = serde_json::json!("effect");
        value["targets"] = serde_json::json!(["deck"]);
        value["abi"] = serde_json::json!("deck-v1");
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
        let package = load_effect_package(&path).unwrap();
        assert_eq!(
            package.manifest.resolved_targets(),
            vec![EffectPackageTarget::Deck]
        );
        assert_eq!(package.manifest.abi, Some(EffectPackageAbi::DeckV1));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();

        value.as_object_mut().unwrap().remove("abi");
        let path = fixture(&serde_json::to_string(&value).unwrap(), "not reached");
        let error = load_effect_package(&path).unwrap_err().to_string();
        assert!(
            error.contains("explicitly declare its shader ABI"),
            "{error}"
        );
        fs::remove_dir_all(path.parent().unwrap()).unwrap();

        value["abi"] = serde_json::json!("master-v1");
        let path = fixture(&serde_json::to_string(&value).unwrap(), "not reached");
        assert!(matches!(
            load_effect_package(&path),
            Err(EffectManifestError::Invalid(_))
        ));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();

        value["abi"] = serde_json::json!("deck-v1");
        value["resources"] = serde_json::json!({"history": "previous_slot_output"});
        let path = fixture(&serde_json::to_string(&value).unwrap(), "not reached");
        assert!(matches!(
            load_effect_package(&path),
            Err(EffectManifestError::Invalid(_))
        ));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
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
    fn discovers_selectable_master_and_deck_packages_but_not_the_processor() {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "oneiroi-effect-registry-{}-{id}",
            std::process::id()
        ));
        for (name, role, version, target, abi) in [
            ("custom", "master_effect", 1, None, None),
            ("deck-custom", "effect", 2, Some("deck"), Some("deck-v1")),
            ("processor", "master_processor", 1, None, None),
        ] {
            let directory = root.join(name);
            fs::create_dir_all(&directory).unwrap();
            let mut value: serde_json::Value =
                serde_json::from_str(&manifest(parameters())).unwrap();
            value["id"] = serde_json::json!(name);
            value["name"] = serde_json::json!(name);
            value["role"] = serde_json::json!(role);
            value["version"] = serde_json::json!(version);
            if let Some(target) = target {
                value["targets"] = serde_json::json!([target]);
            }
            if let Some(abi) = abi {
                value["abi"] = serde_json::json!(abi);
            }
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
        assert_eq!(registry.effects.len(), 2);
        let custom = registry
            .effects
            .iter()
            .find(|effect| effect.id == "custom")
            .unwrap();
        assert!(custom.supports_target(EffectPackageTarget::Master));
        let deck = registry
            .effects
            .iter()
            .find(|effect| effect.id == "deck-custom")
            .unwrap();
        assert!(deck.supports_target(EffectPackageTarget::Deck));
        assert!(!deck.supports_target(EffectPackageTarget::Master));
        fs::remove_dir_all(root).unwrap();
    }
}
