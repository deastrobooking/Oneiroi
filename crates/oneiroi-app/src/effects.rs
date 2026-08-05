//! Effect resource roots, manifest watching and registry refresh.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use oneiroi_render::{
    EffectDescriptor, EffectPackageTarget, EffectRegistry, discover_effect_packages,
};

use super::State;

const EFFECT_PATH_ENV: &str = "ONEIROI_EFFECT_PATH";

/// Resolve the shipped effect directory without relying on the process launch
/// directory. A macOS app bundle keeps resources under `Contents/Resources`;
/// development builds fall back to the checked-out workspace captured by
/// Cargo. The active launch workspace is an additional package root, but it
/// cannot impersonate the shipped processor or override bundled package IDs.
pub(crate) fn effect_resource_roots(workspace: &Path) -> Vec<PathBuf> {
    let executable = std::env::current_exe().ok();
    let workspace_root = workspace.join("effects");
    let mut bundled_candidates = Vec::new();
    if let Some(executable_directory) = executable.as_deref().and_then(Path::parent) {
        if let Some(contents_directory) = executable_directory.parent() {
            bundled_candidates.push(contents_directory.join("Resources/effects"));
        }
        bundled_candidates.push(executable_directory.join("effects"));
    }
    let development_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("effects");
    bundled_candidates.push(development_root.clone());

    let bundled_root = bundled_candidates
        .into_iter()
        .find(|root| root.join("master-effects/effect.json").is_file())
        .unwrap_or(development_root);

    let mut roots = vec![bundled_root];
    if workspace_root.is_dir() {
        roots.push(workspace_root);
    }
    if let Some(paths) = std::env::var_os(EFFECT_PATH_ENV) {
        roots.extend(std::env::split_paths(&paths).filter(|path| !path.as_os_str().is_empty()));
    }
    if let Some(user_root) = user_effect_root()
        && user_root.is_dir()
    {
        roots.push(user_root);
    }
    deduplicate_paths(roots)
}

fn user_effect_root() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support/Oneiroi/effects"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .map(|root| root.join("oneiroi/effects"))
    }
}

fn deduplicate_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    paths
        .into_iter()
        .filter(|path| {
            let identity = path.canonicalize().unwrap_or_else(|_| path.clone());
            seen.insert(identity)
        })
        .collect()
}

pub(crate) fn discover_effect_registry(roots: &[PathBuf]) -> EffectRegistry {
    let mut combined = EffectRegistry::default();
    let mut ids = HashSet::new();
    for root in roots {
        let registry = discover_effect_packages(root);
        combined.errors.extend(registry.errors);
        combined
            .failed_manifest_paths
            .extend(registry.failed_manifest_paths);
        for effect in registry.effects {
            if ids.insert(effect.id.clone()) {
                combined.effects.push(effect);
            } else {
                combined.errors.push(format!(
                    "duplicate effect id {:?} in resource root {}",
                    effect.id,
                    root.display()
                ));
            }
        }
    }
    combined
        .effects
        .sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    combined.errors.sort();
    combined.errors.dedup();
    combined.failed_manifest_paths.sort();
    combined.failed_manifest_paths.dedup();
    combined
}

fn retain_last_known_effects(
    registry: &mut EffectRegistry,
    previous: &[EffectDescriptor],
    roots: &[PathBuf],
) {
    for effect in previous {
        if !effect.manifest_path.is_file()
            || !path_is_in_roots(&effect.manifest_path, roots)
            || !registry
                .failed_manifest_paths
                .iter()
                .any(|failed| same_path(failed, &effect.manifest_path))
        {
            continue;
        }
        let Some(previous_precedence) = package_precedence(&effect.manifest_path, roots) else {
            continue;
        };
        let mut shadowed_paths = Vec::new();
        registry.effects.retain(|candidate| {
            let is_shadowed = candidate.id == effect.id
                && package_precedence(&candidate.manifest_path, roots)
                    .is_some_and(|precedence| precedence > previous_precedence);
            if is_shadowed {
                shadowed_paths.push(candidate.manifest_path.clone());
            }
            !is_shadowed
        });
        for path in shadowed_paths {
            registry.errors.push(format!(
                "lower-priority duplicate effect id {:?} at {} remains shadowed while {} uses its last-known-good descriptor",
                effect.id,
                path.display(),
                effect.manifest_path.display(),
            ));
        }
        if !registry
            .effects
            .iter()
            .any(|candidate| candidate.id == effect.id)
        {
            registry.effects.push(effect.clone());
        }
    }
    registry
        .effects
        .sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    registry.errors.sort();
    registry.errors.dedup();
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = left.canonicalize().unwrap_or_else(|_| left.to_path_buf());
    let right = right.canonicalize().unwrap_or_else(|_| right.to_path_buf());
    left == right
}

fn path_is_in_roots(path: &Path, roots: &[PathBuf]) -> bool {
    package_precedence(path, roots).is_some()
}

fn package_precedence(path: &Path, roots: &[PathBuf]) -> Option<(usize, PathBuf)> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    roots.iter().enumerate().find_map(|(index, root)| {
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        path.strip_prefix(root)
            .ok()
            .map(|relative| (index, relative.to_path_buf()))
    })
}

pub(crate) fn effect_registry_status(registry: &EffectRegistry, roots: &[PathBuf]) -> String {
    let master_count = registry
        .effects
        .iter()
        .filter(|effect| effect.supports_target(EffectPackageTarget::Master))
        .count();
    let deck_count = registry
        .effects
        .iter()
        .filter(|effect| effect.supports_target(EffectPackageTarget::Deck))
        .count();
    let summary = format!(
        "{master_count} master package(s), {deck_count} deck package candidate(s) from {} resource root(s)",
        roots.len()
    );
    if registry.errors.is_empty() {
        summary
    } else {
        format!(
            "{summary}, {} rejected · {}",
            registry.errors.len(),
            registry.errors.join(" · ")
        )
    }
}

pub(crate) fn partition_effect_packages(
    effects: Vec<EffectDescriptor>,
) -> (Vec<EffectDescriptor>, Vec<EffectDescriptor>) {
    let mut master = Vec::new();
    let mut deck = Vec::new();
    for effect in effects {
        if effect.supports_target(EffectPackageTarget::Master) {
            master.push(effect.clone());
        }
        if effect.supports_target(EffectPackageTarget::Deck) {
            deck.push(effect);
        }
    }
    (master, deck)
}

pub(crate) fn bundled_processor_manifest(roots: &[PathBuf]) -> Option<PathBuf> {
    roots
        .first()
        .map(|root| root.join("master-effects/effect.json"))
        .filter(|path| path.is_file())
}

impl State {
    pub(crate) fn resolved_effect_manifest_path(&self) -> PathBuf {
        let path = PathBuf::from(&self.ui.effect_manifest_path);
        if path.is_absolute() {
            path
        } else {
            self.workspace.join(path)
        }
    }

    pub(crate) fn watch_effect_manifest(&mut self) {
        let paths = self.effect_manifest_paths();
        self.master_effect_processor.watch_effect_manifests(paths);
        self.ui.effect_reload_status = self.master_effect_processor.reload_status().to_owned();
        self.compositor.watch_deck_effect_manifests(
            self.ui
                .deck_effect_packages
                .iter()
                .map(|effect| effect.manifest_path.clone())
                .collect(),
        );
        self.ui.deck_effect_reload_status = self.compositor.deck_effect_reload_status().to_owned();
    }

    pub(crate) fn effect_manifest_paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![self.resolved_effect_manifest_path()];
        paths.extend(
            self.ui
                .effect_packages
                .iter()
                .map(|effect| effect.manifest_path.clone()),
        );
        paths.sort();
        paths.dedup();
        paths
    }

    pub(crate) fn refresh_effect_registry(&mut self) {
        let roots = effect_resource_roots(&self.workspace);
        let mut registry = discover_effect_registry(&roots);
        let mut previous = self.ui.effect_packages.clone();
        previous.extend(self.ui.deck_effect_packages.iter().cloned());
        retain_last_known_effects(&mut registry, &previous, &roots);
        self.ui.effect_registry_status = effect_registry_status(&registry, &roots);
        (self.ui.effect_packages, self.ui.deck_effect_packages) =
            partition_effect_packages(registry.effects);
        self.watch_effect_manifest();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_effects_do_not_depend_on_the_launch_directory() {
        let unrelated_workspace = std::env::temp_dir().join(format!(
            "oneiroi-unrelated-launch-directory-{}",
            std::process::id()
        ));
        let roots = effect_resource_roots(&unrelated_workspace);
        let processor_manifest = bundled_processor_manifest(&roots)
            .expect("launch-independent bundled processor manifest");
        let bundled_root = processor_manifest
            .parent()
            .and_then(Path::parent)
            .expect("bundled effects root");
        let registry = discover_effect_packages(bundled_root);
        let ids: HashSet<_> = registry
            .effects
            .iter()
            .map(|effect| effect.id.as_str())
            .collect();

        assert!(registry.errors.is_empty(), "{:?}", registry.errors);
        for id in ["recursive-2d", "fractal-volume", "hyper-recursion"] {
            assert!(ids.contains(id), "missing bundled algorithmic effect {id}");
        }
    }

    #[test]
    fn active_workspace_can_add_effect_packages_without_replacing_bundled_resources() {
        let workspace = std::env::temp_dir().join(format!(
            "oneiroi-workspace-effect-root-{}",
            std::process::id()
        ));
        let workspace_root = workspace.join("effects");
        std::fs::create_dir_all(workspace_root.join("master-effects")).unwrap();
        std::fs::write(
            workspace_root.join("master-effects/effect.json"),
            "not a bundled manifest",
        )
        .unwrap();

        let roots = effect_resource_roots(&workspace);
        let workspace_identity = workspace_root.canonicalize().unwrap();
        assert!(roots.iter().any(|root| {
            root.canonicalize()
                .is_ok_and(|identity| identity == workspace_identity)
        }));
        let bundled_manifest = bundled_processor_manifest(&roots).unwrap();
        assert!(!bundled_manifest.starts_with(&workspace_root));

        std::fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn target_partition_keeps_deck_candidates_out_of_the_master_runtime() {
        let bundled_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("effects");
        let registry = discover_effect_packages(bundled_root);
        let mut deck_candidate = registry.effects[0].clone();
        deck_candidate.id = "deck-candidate".to_owned();
        deck_candidate.targets = vec![EffectPackageTarget::Deck];
        let expected_master_count = registry.effects.len();
        let expected_deck_count = registry
            .effects
            .iter()
            .filter(|effect| effect.supports_target(EffectPackageTarget::Deck))
            .count()
            + 1;
        let mut candidates = registry.effects;
        candidates.push(deck_candidate);

        let (master, deck) = partition_effect_packages(candidates);

        assert_eq!(master.len(), expected_master_count);
        assert_eq!(deck.len(), expected_deck_count);
        assert!(deck.iter().any(|effect| effect.id == "deck-candidate"));
        assert!(
            master
                .iter()
                .all(|effect| effect.supports_target(EffectPackageTarget::Master))
        );
    }

    #[test]
    fn invalid_refresh_keeps_the_previous_descriptor_until_the_package_is_removed() {
        let root = std::env::temp_dir().join(format!(
            "oneiroi-last-known-effect-root-{}",
            std::process::id()
        ));
        let package_root = root.join("recursive-2d");
        std::fs::create_dir_all(&package_root).unwrap();
        let bundled_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("effects/recursive-2d");
        let manifest_path = package_root.join("effect.json");
        std::fs::copy(bundled_root.join("effect.json"), &manifest_path).unwrap();
        std::fs::copy(
            bundled_root.join("recursive_2d.wgsl"),
            package_root.join("recursive_2d.wgsl"),
        )
        .unwrap();

        let previous = discover_effect_packages(&root);
        assert_eq!(previous.effects.len(), 1);
        std::fs::write(&manifest_path, "{not valid json").unwrap();
        let mut refreshed = discover_effect_packages(&root);
        assert!(refreshed.effects.is_empty());
        assert!(!refreshed.errors.is_empty());
        retain_last_known_effects(
            &mut refreshed,
            &previous.effects,
            std::slice::from_ref(&root),
        );
        assert_eq!(refreshed.effects, previous.effects);

        let renamed_manifest = std::fs::read_to_string(bundled_root.join("effect.json"))
            .unwrap()
            .replacen(
                "\"id\": \"recursive-2d\"",
                "\"id\": \"recursive-renamed\"",
                1,
            );
        std::fs::write(&manifest_path, renamed_manifest).unwrap();
        let mut renamed = discover_effect_packages(&root);
        assert!(renamed.errors.is_empty(), "{:?}", renamed.errors);
        retain_last_known_effects(&mut renamed, &previous.effects, std::slice::from_ref(&root));
        assert_eq!(renamed.effects.len(), 1);
        assert_eq!(renamed.effects[0].id, "recursive-renamed");

        std::fs::remove_file(&manifest_path).unwrap();
        let mut removed = discover_effect_packages(&root);
        retain_last_known_effects(&mut removed, &previous.effects, std::slice::from_ref(&root));
        assert!(removed.effects.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_high_priority_package_keeps_precedence_over_a_duplicate() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_root = std::env::temp_dir().join(format!(
            "oneiroi-duplicate-lkg-root-{}-{nonce}",
            std::process::id()
        ));
        let high_root = temp_root.join("high");
        let low_root = temp_root.join("low");
        let bundled_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("effects/recursive-2d");
        for root in [&high_root, &low_root] {
            let package_root = root.join("recursive-2d");
            std::fs::create_dir_all(&package_root).unwrap();
            std::fs::copy(
                bundled_root.join("effect.json"),
                package_root.join("effect.json"),
            )
            .unwrap();
            std::fs::copy(
                bundled_root.join("recursive_2d.wgsl"),
                package_root.join("recursive_2d.wgsl"),
            )
            .unwrap();
        }
        let roots = vec![high_root.clone(), low_root.clone()];
        let previous = discover_effect_registry(&roots);
        assert_eq!(previous.effects.len(), 1);
        assert!(previous.effects[0].manifest_path.starts_with(&high_root));

        let high_manifest = high_root.join("recursive-2d/effect.json");
        std::fs::write(&high_manifest, "{not valid json").unwrap();
        let mut refreshed = discover_effect_registry(&roots);
        assert_eq!(refreshed.effects.len(), 1);
        assert!(refreshed.effects[0].manifest_path.starts_with(&low_root));

        retain_last_known_effects(&mut refreshed, &previous.effects, &roots);

        assert_eq!(refreshed.effects.len(), 1);
        assert!(same_path(
            &refreshed.effects[0].manifest_path,
            &previous.effects[0].manifest_path
        ));
        assert!(
            refreshed
                .errors
                .iter()
                .any(|error| error.contains("uses its last-known-good descriptor")),
            "{:?}",
            refreshed.errors
        );
        std::fs::remove_dir_all(temp_root).unwrap();
    }
}
