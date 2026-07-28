//! Inspect the import path and performance rating for movie files.

use oneiroi_media::probe_movie;

fn main() {
    let paths = std::env::args_os().skip(1).collect::<Vec<_>>();
    if paths.is_empty() {
        eprintln!("usage: cargo run -p oneiroi-media --example probe_movie -- <movie> [...]");
        std::process::exit(2);
    }

    let mut failed = false;
    for path in paths {
        match probe_movie(&path) {
            Ok(movie) => {
                let frame_rate = movie
                    .frame_rate
                    .map(|rate| {
                        format!("{:.3} fps", rate.numerator as f64 / rate.denominator as f64)
                    })
                    .unwrap_or_else(|| "unknown fps".to_owned());
                println!(
                    "{}\n  {} · {}x{} · {} · {:?}\n  {:?}: {}",
                    movie.path.display(),
                    movie.codec,
                    movie.visible_extent[0],
                    movie.visible_extent[1],
                    frame_rate,
                    movie.decode_path,
                    movie.health,
                    movie.health_reason
                );
            }
            Err(error) => {
                failed = true;
                eprintln!("{}: {error}", std::path::Path::new(&path).display());
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
}
