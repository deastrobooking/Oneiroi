//! Branded paths and non-destructive access to pre-VIRTUAL show data.
use std::path::{Path, PathBuf};

pub(crate) fn is_project_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("virtual") || extension.eq_ignore_ascii_case("oneiroi")
        })
}

pub(crate) fn journal_directories(workspace: &Path) -> [PathBuf; 2] {
    [
        workspace.join(".virtual/session"),
        workspace.join(".oneiroi/session"),
    ]
}

pub(crate) fn untitled_recovery(workspace: &Path) -> Option<PathBuf> {
    [
        virtual_io::autosave_path(None, workspace),
        workspace.join(".oneiroi-untitled.autosave"),
    ]
    .into_iter()
    .filter(|path| path.is_file())
    .max_by_key(|path| {
        path.metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_current_and_legacy_project_extensions() {
        for name in [
            "show.virtual",
            "show.VIRTUAL",
            "show.oneiroi",
            "show.ONEIROI",
        ] {
            assert!(is_project_path(Path::new(name)), "{name}");
        }
        for name in ["show.mov", "virtual", "show.virtual.mov"] {
            assert!(!is_project_path(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn legacy_recovery_remains_available_without_moving_it() {
        let workspace =
            std::env::temp_dir().join(format!("virtual-paths-{}", virtual_io::new_project_id()));
        std::fs::create_dir_all(&workspace).unwrap();
        assert_eq!(untitled_recovery(&workspace), None);
        let legacy = workspace.join(".oneiroi-untitled.autosave");
        std::fs::write(&legacy, b"legacy").unwrap();
        assert_eq!(untitled_recovery(&workspace), Some(legacy.clone()));
        let current = virtual_io::autosave_path(None, &workspace);
        std::fs::write(&current, b"current").unwrap();
        assert!(untitled_recovery(&workspace).is_some());
        assert!(legacy.is_file());
        std::fs::remove_dir_all(workspace).unwrap();
    }
}
