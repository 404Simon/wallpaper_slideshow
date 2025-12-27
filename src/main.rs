use chrono::{Local, Timelike};
use rand::prelude::*;
use rayon::prelude::*;
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;
use walkdir::WalkDir;

const WALLPAPER_DIR: &str = "/home/simon/dotfiles/wallpapers/norway";
const HISTORY_LOG: &str = "/home/simon/.cache/wallpaper_history.log";
const CACHE_DB: &str = "/home/simon/.cache/wallpaper_exif_cache.db";
const HISTORY_SIZE: usize = 25;
const TIME_WINDOW: i32 = 1;

#[derive(Debug)]
struct ImageCandidate {
    path: PathBuf,
    hour: Option<u8>,
}

#[derive(Debug)]
struct CachedEntry {
    mtime: i64,
    hour: Option<u8>,
}

fn main() {
    setup_environment();

    let current_hour = Local::now().hour() as i32;
    println!("Current hour: {}", current_hour);

    let recent_wallpapers = load_history();

    let all_images: Vec<(PathBuf, i64)> = WalkDir::new(WALLPAPER_DIR)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && e.path()
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.eq_ignore_ascii_case("jpg") || s.eq_ignore_ascii_case("jpeg"))
                    .unwrap_or(false)
        })
        .filter_map(|e| {
            let mtime = get_mtime(e.path()).ok()?;
            Some((e.path().to_path_buf(), mtime))
        })
        .collect();

    println!("Found {} total images", all_images.len());

    let available_images: Vec<(PathBuf, i64)> = all_images
        .iter()
        .filter(|(img, _)| {
            let basename = img.file_name().and_then(|s| s.to_str()).unwrap_or("");
            !recent_wallpapers.contains(basename)
        })
        .cloned()
        .collect();

    let images_to_process = if available_images.is_empty() {
        println!("All images used recently, resetting pool");
        all_images.clone()
    } else {
        available_images
    };

    println!("Processing {} available images", images_to_process.len());

    let candidates = match get_candidates_with_cache(&images_to_process, &all_images) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Cache error, falling back to direct EXIF parsing: {}", e);
            // Fallback: parse all EXIF directly
            images_to_process
                .par_iter()
                .map(|(path, _)| ImageCandidate {
                    path: path.clone(),
                    hour: extract_hour_from_exif(path),
                })
                .collect()
        }
    };

    let mut best_match: Option<&ImageCandidate> = None;
    let mut best_diff = 24;
    let mut time_window_matches: Vec<&ImageCandidate> = Vec::new();

    for candidate in &candidates {
        if let Some(image_hour) = candidate.hour {
            let diff = calculate_time_diff(current_hour, image_hour as i32);

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

    if let Some(wallpaper) = selected {
        let path = wallpaper.path.to_string_lossy();
        println!(
            "Selected: {} (Hour: {}, Best diff: {})",
            path,
            wallpaper
                .hour
                .map(|h: u8| h.to_string())
                .unwrap_or_else(|| "N/A".to_string()),
            best_diff
        );

        if let Some(basename) = wallpaper
            .path
            .file_name()
            .and_then(|s: &std::ffi::OsStr| s.to_str())
        {
            log_to_history(basename);
        }

        apply_wallpaper(&path);
    } else {
        eprintln!("No suitable wallpaper found");
    }
}

fn get_mtime(path: &Path) -> Result<i64, std::io::Error> {
    let metadata = fs::metadata(path)?;
    let mtime = metadata
        .modified()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Ok(mtime)
}

fn get_candidates_with_cache(
    images_to_process: &[(PathBuf, i64)],
    all_images: &[(PathBuf, i64)],
) -> Result<Vec<ImageCandidate>, rusqlite::Error> {
    let conn = open_cache_db()?;

    let cache: HashMap<String, CachedEntry> = load_cache_to_memory(&conn)?;
    println!("Loaded {} entries from cache", cache.len());

    let current_paths: HashSet<String> = all_images
        .iter()
        .map(|(p, _)| p.to_string_lossy().to_string())
        .collect();

    let files_to_parse: Vec<&(PathBuf, i64)> = all_images
        .iter()
        .filter(|(path, mtime)| {
            let path_str = path.to_string_lossy();
            match cache.get(path_str.as_ref()) {
                Some(entry) => entry.mtime != *mtime, // File changed
                None => true,                         // Not in cache
            }
        })
        .collect();

    println!(
        "Cache hit: {}, need to parse: {}",
        all_images.len() - files_to_parse.len(),
        files_to_parse.len()
    );

    let new_entries: Vec<(String, i64, Option<u8>)> = files_to_parse
        .par_iter()
        .map(|(path, mtime)| {
            let hour = extract_hour_from_exif(path);
            (path.to_string_lossy().to_string(), *mtime, hour)
        })
        .collect();

    if !new_entries.is_empty() {
        insert_cache_entries(&conn, &new_entries)?;
        println!("Inserted {} new cache entries", new_entries.len());
    }

    cleanup_stale_entries(&conn, &current_paths, &cache)?;

    let new_entries_map: HashMap<&str, Option<u8>> = new_entries
        .iter()
        .map(|(path, _, hour)| (path.as_str(), *hour))
        .collect();

    let candidates: Vec<ImageCandidate> = images_to_process
        .iter()
        .map(|(path, _)| {
            let path_str = path.to_string_lossy();
            let hour = new_entries_map
                .get(path_str.as_ref())
                .copied()
                .flatten()
                .or_else(|| cache.get(path_str.as_ref()).and_then(|e| e.hour));

            ImageCandidate {
                path: path.clone(),
                hour,
            }
        })
        .collect();

    Ok(candidates)
}

fn open_cache_db() -> Result<Connection, rusqlite::Error> {
    if let Some(parent) = Path::new(CACHE_DB).parent() {
        let _ = fs::create_dir_all(parent);
    }

    let conn = Connection::open(CACHE_DB)?;

    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA cache_size = 10000;
        PRAGMA temp_store = MEMORY;
        ",
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS exif_cache (
            path TEXT PRIMARY KEY,
            mtime INTEGER NOT NULL,
            hour INTEGER
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_path ON exif_cache(path)",
        [],
    )?;

    Ok(conn)
}

fn load_cache_to_memory(
    conn: &Connection,
) -> Result<HashMap<String, CachedEntry>, rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT path, mtime, hour FROM exif_cache")?;
    let entries = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            CachedEntry {
                mtime: row.get(1)?,
                hour: row.get(2)?,
            },
        ))
    })?;

    let mut map = HashMap::new();
    for entry in entries {
        let (path, cached) = entry?;
        map.insert(path, cached);
    }
    Ok(map)
}

fn insert_cache_entries(
    conn: &Connection,
    entries: &[(String, i64, Option<u8>)],
) -> Result<(), rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;

    {
        let mut stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO exif_cache (path, mtime, hour) VALUES (?1, ?2, ?3)",
        )?;

        for (path, mtime, hour) in entries {
            stmt.execute(params![path, mtime, hour])?;
        }
    }

    tx.commit()?;
    Ok(())
}

fn cleanup_stale_entries(
    conn: &Connection,
    current_paths: &HashSet<String>,
    cache: &HashMap<String, CachedEntry>,
) -> Result<(), rusqlite::Error> {
    let stale_paths: Vec<&String> = cache
        .keys()
        .filter(|path| !current_paths.contains(*path))
        .collect();

    if stale_paths.is_empty() {
        return Ok(());
    }

    println!("Removing {} stale cache entries", stale_paths.len());

    let tx = conn.unchecked_transaction()?;

    {
        let mut stmt = tx.prepare_cached("DELETE FROM exif_cache WHERE path = ?1")?;
        for path in &stale_paths {
            stmt.execute([path])?;
        }
    }

    tx.commit()?;
    Ok(())
}

fn setup_environment() {
    let uid = unsafe { libc::getuid() };
    let runtime_dir = format!("/run/user/{}", uid);
    env::set_var("XDG_RUNTIME_DIR", &runtime_dir);

    let hypr_dir = format!("{}/hypr", runtime_dir);
    if let Ok(entries) = fs::read_dir(&hypr_dir) {
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

fn load_history() -> HashSet<String> {
    let path = Path::new(HISTORY_LOG);
    if !path.exists() {
        return HashSet::new();
    }

    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return HashSet::new(),
    };

    let reader = BufReader::new(file);
    reader
        .lines()
        .filter_map(|line| line.ok())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .take(HISTORY_SIZE)
        .collect()
}

fn extract_hour_from_exif(path: &Path) -> Option<u8> {
    let exif = rexif::parse_file(path).ok()?;

    for entry in exif.entries {
        if entry.tag == rexif::ExifTag::DateTimeOriginal {
            if let rexif::TagValue::Ascii(ref s) = entry.value {
                // Format: "YYYY:MM:DD HH:MM:SS"
                if s.len() >= 13 {
                    let hour_str = &s[11..13];
                    if let Ok(hour) = hour_str.parse::<u8>() {
                        if hour <= 23 {
                            return Some(hour);
                        }
                    }
                }
            }
        }
    }
    None
}

fn calculate_time_diff(current: i32, image: i32) -> i32 {
    let mut diff = (current - image + 24) % 24;
    if diff > 12 {
        diff = 24 - diff;
    }
    diff
}

fn log_to_history(basename: &str) {
    if let Some(parent) = Path::new(HISTORY_LOG).parent() {
        let _ = fs::create_dir_all(parent);
    }

    let mut file = match fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(HISTORY_LOG)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Failed to open history log: {}", e);
            return;
        }
    };

    let _ = writeln!(file, "{}", basename);
}

fn apply_wallpaper(path: &str) {
    let reload_arg = format!(",{}", path);
    let status = Command::new("hyprctl")
        .args(&["hyprpaper", "reload", &reload_arg])
        .status();

    if let Err(e) = status {
        eprintln!("Failed to run hyprctl: {}", e);
    }

    let home = env::var("HOME").unwrap_or_else(|_| "/home/simon".to_string());
    let thaimeleon_path = format!("{}/.cargo/bin/thaimeleon", home);
    let config_path = format!("{}/.config/yolk/chameleon.rhai", home);

    let status = Command::new(thaimeleon_path)
        .args(&[path, "-w", &config_path])
        .status();

    if let Err(e) = status {
        eprintln!("Failed to run thaimeleon: {}", e);
    }

    let status = Command::new("/usr/bin/yolk").arg("sync").status();

    if let Err(e) = status {
        eprintln!("Failed to run yolk: {}", e);
    }
}
