use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::config;
use crate::discovery;

pub fn load_recent() -> HashSet<String> {
    load_recent_with_size(config::HISTORY_SIZE)
}

pub fn load_recent_with_size(limit: usize) -> HashSet<String> {
    let path = config::history_log();
    let path = Path::new(&path);

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
        .map_while(Result::ok)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .take(limit)
        .collect()
}

pub fn log(basename: &str) {
    let path = config::history_log();
    if let Some(parent) = Path::new(&path).parent() {
        let _ = fs::create_dir_all(parent);
    }

    let mut file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Failed to open history log: {}", e);
            return;
        }
    };

    let _ = writeln!(file, "{}", basename);
}

pub struct WallpaperHistory {
    entries: Vec<String>,
    current_index: usize,
}

impl WallpaperHistory {
    pub fn load() -> Option<Self> {
        let path = config::history_log();
        let file = File::open(&path).ok()?;
        let entries: Vec<String> = BufReader::new(file).lines().map_while(Result::ok).collect();

        if entries.is_empty() {
            return None;
        }

        Some(Self {
            current_index: entries.len() - 1,
            entries,
        })
    }

    pub fn current_basename(&self) -> &str {
        &self.entries[self.current_index]
    }

    pub fn go_previous(&mut self) -> bool {
        if self.current_index > 0 {
            self.current_index -= 1;
            true
        } else {
            false
        }
    }

    pub fn go_next(&mut self) -> bool {
        if self.current_index < self.entries.len() - 1 {
            self.current_index += 1;
            true
        } else {
            false
        }
    }

    pub fn position_str(&self) -> String {
        format!("{}/{}", self.current_index + 1, self.entries.len())
    }

    pub fn current_path(&self) -> Option<PathBuf> {
        discovery::find_by_basename(self.current_basename())
    }
}
