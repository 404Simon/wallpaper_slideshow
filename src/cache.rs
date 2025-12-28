use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::config;

#[derive(Debug, Clone)]
pub struct CachedEntry {
    pub mtime: i64,
    pub hour: Option<u8>,
}

pub fn open() -> Result<Connection, rusqlite::Error> {
    let db_path = config::cache_db();
    if let Some(parent) = Path::new(&db_path).parent() {
        let _ = fs::create_dir_all(parent);
    }

    let conn = Connection::open(&db_path)?;

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

pub fn load_all(conn: &Connection) -> Result<HashMap<String, CachedEntry>, rusqlite::Error> {
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

pub fn insert(
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

pub fn cleanup_stale(
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
