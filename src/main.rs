use chrono::{Local, Timelike};
use rand::prelude::*;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::env;
use std::process::Command;

use wallpaper_slideshow::{cache, discovery, exif, history, ImageFile};

const TIME_WINDOW: i32 = 1;

fn main() {
    setup_environment();

    let current_hour = Local::now().hour() as i32;
    println!("Current hour: {}", current_hour);

    let recent = history::load_recent();
    let all_images = discovery::find_images();
    println!("Found {} total images", all_images.len());

    let available: Vec<_> = all_images
        .iter()
        .filter(|img| {
            let basename = img.path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            !recent.contains(basename)
        })
        .cloned()
        .collect();

    let pool = if available.is_empty() {
        println!("All images used recently, resetting pool");
        all_images.clone()
    } else {
        available
    };

    println!("Processing {} available images", pool.len());

    let candidates = get_candidates_with_cache(&pool, &all_images);
    let selected = select_wallpaper(&candidates, current_hour);

    if let Some((path, hour)) = selected {
        println!(
            "Selected: {} (Hour: {})",
            path.display(),
            hour.map(|h| h.to_string()).unwrap_or_else(|| "N/A".into())
        );

        if let Some(basename) = path.file_name().and_then(|s| s.to_str()) {
            history::log(basename);
        }

        apply_wallpaper(&path.to_string_lossy());
    } else {
        eprintln!("No suitable wallpaper found");
    }
}

struct Candidate {
    path: std::path::PathBuf,
    hour: Option<u8>,
}

fn get_candidates_with_cache(pool: &[ImageFile], all: &[ImageFile]) -> Vec<Candidate> {
    match try_cached_candidates(pool, all) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Cache error, falling back to direct EXIF parsing: {}", e);
            pool.par_iter()
                .map(|img| Candidate {
                    path: img.path.clone(),
                    hour: exif::extract(&img.path).hour,
                })
                .collect()
        }
    }
}

fn try_cached_candidates(
    pool: &[ImageFile],
    all: &[ImageFile],
) -> Result<Vec<Candidate>, rusqlite::Error> {
    let conn = cache::open()?;
    let cached = cache::load_all(&conn)?;
    println!("Loaded {} entries from cache", cached.len());

    let current_paths: HashSet<String> = all
        .iter()
        .map(|img| img.path.to_string_lossy().to_string())
        .collect();

    let to_parse: Vec<_> = all
        .iter()
        .filter(|img| {
            let path_str = img.path.to_string_lossy();
            match cached.get(path_str.as_ref()) {
                Some(entry) => entry.mtime != img.mtime,
                None => true,
            }
        })
        .collect();

    println!(
        "Cache hit: {}, need to parse: {}",
        all.len() - to_parse.len(),
        to_parse.len()
    );

    let new_entries: Vec<(String, i64, Option<u8>)> = to_parse
        .par_iter()
        .map(|img| {
            let hour = exif::extract(&img.path).hour;
            (img.path.to_string_lossy().to_string(), img.mtime, hour)
        })
        .collect();

    if !new_entries.is_empty() {
        cache::insert(&conn, &new_entries)?;
        println!("Inserted {} new cache entries", new_entries.len());
    }

    cache::cleanup_stale(&conn, &current_paths, &cached)?;

    let new_map: HashMap<&str, Option<u8>> = new_entries
        .iter()
        .map(|(path, _, hour)| (path.as_str(), *hour))
        .collect();

    let candidates = pool
        .iter()
        .map(|img| {
            let path_str = img.path.to_string_lossy();
            let hour = new_map
                .get(path_str.as_ref())
                .copied()
                .flatten()
                .or_else(|| cached.get(path_str.as_ref()).and_then(|e| e.hour));

            Candidate {
                path: img.path.clone(),
                hour,
            }
        })
        .collect();

    Ok(candidates)
}

fn select_wallpaper(
    candidates: &[Candidate],
    current_hour: i32,
) -> Option<(std::path::PathBuf, Option<u8>)> {
    let mut best_match: Option<&Candidate> = None;
    let mut best_diff = 24;
    let mut time_window_matches: Vec<&Candidate> = Vec::new();

    for candidate in candidates {
        if let Some(image_hour) = candidate.hour {
            let diff = time_diff(current_hour, image_hour as i32);

            if diff <= TIME_WINDOW {
                time_window_matches.push(candidate);
            }

            if diff < best_diff {
                best_diff = diff;
                best_match = Some(candidate);
            }
        }
    }

    let selected = if !time_window_matches.is_empty() {
        println!(
            "Found {} images within {} hour window",
            time_window_matches.len(),
            TIME_WINDOW
        );
        time_window_matches.choose(&mut rand::rng()).copied()
    } else if let Some(best) = best_match {
        println!("Using best time match (diff: {} hours)", best_diff);
        Some(best)
    } else {
        println!("Choosing random image");
        candidates.choose(&mut rand::rng())
    };

    selected.map(|c| (c.path.clone(), c.hour))
}

/// wrap hours around 24
fn time_diff(current: i32, image: i32) -> i32 {
    let mut diff = (current - image + 24) % 24;
    if diff > 12 {
        diff = 24 - diff;
    }
    diff
}

fn setup_environment() {
    let uid = unsafe { libc::getuid() };
    let runtime_dir = format!("/run/user/{}", uid);
    env::set_var("XDG_RUNTIME_DIR", &runtime_dir);

    let hypr_dir = format!("{}/hypr", runtime_dir);
    if let Ok(entries) = std::fs::read_dir(&hypr_dir) {
        if let Some(Ok(entry)) = entries.into_iter().next() {
            if let Some(name) = entry.file_name().to_str() {
                env::set_var("HYPRLAND_INSTANCE_SIGNATURE", name);
            }
        }
    }

    if env::var("WAYLAND_DISPLAY").is_err() {
        env::set_var("WAYLAND_DISPLAY", "wayland-0");
    }
}

fn apply_wallpaper(path: &str) {
    let reload_arg = format!(",{}", path);
    if let Err(e) = Command::new("hyprctl")
        .args(["hyprpaper", "reload", &reload_arg])
        .status()
    {
        eprintln!("Failed to run hyprctl: {}", e);
    }

    let home = env::var("HOME").unwrap_or_else(|_| "/home/simon".to_string());
    let thaimeleon = format!("{}/.cargo/bin/thaimeleon", home);
    let config = format!("{}/.config/yolk/chameleon.rhai", home);

    if let Err(e) = Command::new(&thaimeleon)
        .args([path, "-w", &config])
        .status()
    {
        eprintln!("Failed to run thaimeleon: {}", e);
    }

    if let Err(e) = Command::new("/usr/bin/yolk").arg("sync").status() {
        eprintln!("Failed to run yolk: {}", e);
    }
}
