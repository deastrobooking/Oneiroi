//! Effect manifest paths, watching and registry refresh.

use std::path::PathBuf;

use oneiroi_render::discover_effect_packages;

use super::State;

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
        let registry = discover_effect_packages(self.workspace.join("effects"));
        self.ui.effect_registry_status = if registry.errors.is_empty() {
            format!("{} custom effect package(s)", registry.effects.len())
        } else {
            format!(
                "{} custom effect package(s), {} rejected · {}",
                registry.effects.len(),
                registry.errors.len(),
                registry.errors.join(" · ")
            )
        };
        self.ui.effect_packages = registry.effects;
        self.watch_effect_manifest();
    }
}
