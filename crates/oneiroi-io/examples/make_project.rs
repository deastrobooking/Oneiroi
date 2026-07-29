use std::path::PathBuf;

use oneiroi_io::{ProjectFile, save_project_atomic};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let output = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("usage: make_project OUTPUT.oneiroi [DECK_A ... DECK_D]")?;
    let mut project = ProjectFile::default();
    for (index, path) in arguments.take(4).map(PathBuf::from).enumerate() {
        project.decks[index].clips[0] = Some(path);
        project.decks[index].active_slot = Some(0);
    }
    save_project_atomic(output, &project)?;
    Ok(())
}
