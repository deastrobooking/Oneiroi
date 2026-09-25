//! Bounded, atomic preset storage. Explicit saves run outside the render thread.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;

use serde::{Deserialize, Serialize};
use virtual_io::ThemeProject;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct NamedPreset {
    pub name: String,
    pub theme: ThemeProject,
}

#[derive(Clone, Serialize, Deserialize)]
struct LibraryFile {
    version: u32,
    presets: Vec<NamedPreset>,
    active: Option<ThemeProject>,
}

impl Default for LibraryFile {
    fn default() -> Self {
        Self {
            version: 1,
            presets: Vec::new(),
            active: None,
        }
    }
}

struct PendingSave {
    result: Receiver<Result<(), String>>,
    worker: JoinHandle<()>,
    candidate: LibraryFile,
}

#[derive(Default)]
pub(super) struct PresetLibrary {
    data: LibraryFile,
    path: Option<PathBuf>,
    pending: Option<PendingSave>,
    status: String,
}

impl PresetLibrary {
    pub fn load_default() -> Self {
        let path = std::env::var_os("HOME").map(|home| {
            let base = PathBuf::from(home);
            if cfg!(target_os = "macos") {
                base.join("Library/Application Support/VIRTUAL/appearance-presets.json")
            } else {
                base.join(".config/virtual/appearance-presets.json")
            }
        });
        match path {
            Some(path) => Self::load(path),
            None => Self {
                status: "Presets unavailable: home directory not found.".into(),
                data: LibraryFile::default(),
                path: None,
                pending: None,
            },
        }
    }

    fn load(path: PathBuf) -> Self {
        let result = (|| -> Result<LibraryFile, String> {
            match fs::metadata(&path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(LibraryFile::default());
                }
                Err(e) => return Err(e.to_string()),
                Ok(metadata) if metadata.len() > 1_048_576 => {
                    return Err("preset file exceeds 1 MiB".into());
                }
                Ok(_) => {}
            }
            let data: LibraryFile =
                serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            validate(&data)?;
            Ok(data)
        })();
        match result {
            Ok(data) => Self {
                data,
                path: Some(path),
                status: "Presets are shared across projects on this Mac.".into(),
                pending: None,
            },
            Err(error) => Self {
                status: format!("Cannot load presets: {error}. Existing file was left intact."),
                data: LibraryFile::default(),
                path: None,
                pending: None,
            },
        }
    }

    pub fn entries(&self) -> &[NamedPreset] {
        &self.data.presets
    }
    pub fn active(&self) -> Option<&ThemeProject> {
        self.data.active.as_ref()
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn save(&mut self, name: &str, theme: ThemeProject, overwrite: bool) -> bool {
        let name = name.trim();
        if !valid_name(name) {
            self.status = "Enter a name of 1–64 characters without control characters.".into();
            return false;
        }
        let mut candidate = self.data.clone();
        if let Some(existing) = candidate
            .presets
            .iter_mut()
            .find(|entry| entry.name.eq_ignore_ascii_case(name))
        {
            if !overwrite {
                self.status =
                    "That name already exists. Use Update selected or choose another name.".into();
                return false;
            }
            existing.theme = theme.clone();
        } else if overwrite {
            self.status = "Select an existing preset first.".into();
            return false;
        } else {
            if candidate.presets.len() >= 64 {
                self.status =
                    "The library is full (64 presets). Delete a preset before adding another."
                        .into();
                return false;
            }
            candidate.presets.push(NamedPreset {
                name: name.to_owned(),
                theme: theme.clone(),
            });
            candidate
                .presets
                .sort_by_key(|entry| entry.name.to_lowercase());
        }
        candidate.active = Some(theme);
        self.submit(candidate)
    }

    pub fn select(&mut self, theme: ThemeProject) {
        let mut candidate = self.data.clone();
        candidate.active = Some(theme);
        self.submit(candidate);
    }

    pub fn delete(&mut self, name: &str) {
        let mut candidate = self.data.clone();
        candidate.presets.retain(|entry| entry.name != name);
        self.submit(candidate);
    }

    fn submit(&mut self, candidate: LibraryFile) -> bool {
        if self.busy() {
            self.status = "A preset save is still in progress.".into();
            return false;
        }
        let Some(path) = self.path.clone() else {
            return false;
        };
        let (sender, result) = mpsc::channel();
        let data = candidate.clone();
        match std::thread::Builder::new()
            .name("virtual-theme-save".into())
            .spawn(move || {
                let _ = sender.send(write_atomic(&path, &data));
            }) {
            Ok(worker) => {
                self.pending = Some(PendingSave {
                    result,
                    worker,
                    candidate,
                });
                self.status = "Saving presets…".into();
                true
            }
            Err(error) => {
                self.status = format!("Could not save presets: {error}");
                false
            }
        }
    }

    pub fn poll(&mut self) {
        let Some(pending) = &self.pending else {
            return;
        };
        let result = match pending.result.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("preset worker stopped".into()),
        };
        let pending = self.pending.take().expect("pending preset save");
        let _ = pending.worker.join();
        match result {
            Ok(()) => {
                self.data = pending.candidate;
                self.status =
                    "Presets saved. Last loaded or saved appearance restores at startup.".into();
            }
            Err(error) => self.status = format!("Could not save presets: {error}"),
        }
    }
}

impl Drop for PresetLibrary {
    fn drop(&mut self) {
        if let Some(pending) = self.pending.take() {
            let _ = pending.worker.join();
        }
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= 64 && !name.chars().any(char::is_control)
}

fn validate(data: &LibraryFile) -> Result<(), String> {
    if data.version != 1 || data.presets.len() > 64 {
        return Err("unsupported or oversized preset library".into());
    }
    let mut names = std::collections::BTreeSet::new();
    for entry in &data.presets {
        if !valid_name(&entry.name) || !names.insert(entry.name.to_lowercase()) {
            return Err("invalid or duplicate preset name".into());
        }
    }
    Ok(())
}

fn write_atomic(path: &Path, data: &LibraryFile) -> Result<(), String> {
    validate(data)?;
    let parent = path.parent().ok_or("preset directory unavailable")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(".appearance-{}.tmp", virtual_io::new_project_id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        serde_json::to_writer_pretty(&mut file, data)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok::<_, std::io::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_presets_survive_restart_update_and_delete() {
        let root =
            std::env::temp_dir().join(format!("virtual-themes-{}", virtual_io::new_project_id()));
        let path = root.join("presets.json");
        let mut library = PresetLibrary::load(path.clone());
        let mut theme = ThemeProject::default();
        theme
            .appearance
            .colors
            .insert("text".into(), [240, 220, 170]);
        theme.appearance.text_outline = 1.0;
        assert!(library.save("Warm stage", theme.clone(), false));
        drop(library); // accepted writes must finish even on immediate application exit
        let mut library = PresetLibrary::load(path.clone());
        assert_eq!(library.entries()[0].theme, theme);
        assert_eq!(library.active(), Some(&theme));
        assert!(!library.save("warm stage", theme.clone(), false));
        theme.appearance.element_outline = 2.5;
        assert!(library.save("Warm stage", theme.clone(), true));
        drop(library);
        let mut library = PresetLibrary::load(path.clone());
        assert_eq!(library.entries()[0].theme, theme);
        library.delete("Warm stage");
        drop(library);
        assert!(PresetLibrary::load(path).entries().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_library_is_not_overwritten() {
        let root =
            std::env::temp_dir().join(format!("virtual-themes-{}", virtual_io::new_project_id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("presets.json");
        fs::write(&path, b"broken").unwrap();
        let mut library = PresetLibrary::load(path.clone());
        assert!(!library.save("New", ThemeProject::default(), false));
        assert_eq!(fs::read(path).unwrap(), b"broken");
        fs::remove_dir_all(root).unwrap();
    }
}
