use regex::Regex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassicalWorkEntry {
    pub work_title: String,
    pub movement_title: String,
    pub recording_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackRecord {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub work_title: String,
    pub movement_title: String,
    pub recording_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CuratedCollection {
    pub name: String,
    #[serde(rename = "cat")]
    pub category: String,
    #[serde(rename = "cover")]
    pub cover_file: String,
    #[serde(rename = "hue")]
    pub hue_rgb: [u8; 3],
    #[serde(rename = "tag")]
    pub tag_line: String,
    #[serde(rename = "desc")]
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RawCuratedEntry {
    cat: String,
    cover: String,
    hue: [u8; 3],
    tag: String,
    desc: String,
}

struct ClassicalPatterns {
    p1: Regex,
    p2: Regex,
    p3: Regex,
    p4: Regex,
}

fn patterns() -> &'static ClassicalPatterns {
    static PATS: OnceLock<ClassicalPatterns> = OnceLock::new();
    PATS.get_or_init(|| ClassicalPatterns {
        p1: Regex::new(
            r"(?i)^(.+?)\s*(?:\u{2014}|\u{2013}|:|\s-\s)\s*((?:[IVX]{1,5}\b\.?|\d+\.|No\.\s*\d+|Scene\b|Act\b|Part\b|Var\.|Variation\b|Prelude\b|Postlude\b|Interlude\b).*)$",
        )
        .expect("valid pattern 1"),
        p2: Regex::new(r"^(.+?)\s+-\s+(\d+\s+[A-Za-z].*)$").expect("valid pattern 2"),
        p3: Regex::new(r"(?i)^(.+?)\s+((?:Movement|Mov\.|Mvt\.?)\s*(?:No\.?)?\s*\d+.*)$")
            .expect("valid pattern 3"),
        p4: Regex::new(r"(?i)^(.+?)\s+(Var\.\s*\d+.*)$").expect("valid pattern 4"),
    })
}

fn normalize_work_key(work: &str) -> String {
    work.trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse classical Work and Movement matching all 4 regex rules from Verdant's `classical_work.js`.
pub fn parse_work_and_movement(title: &str) -> ClassicalWorkEntry {
    let raw = title.trim();
    if raw.is_empty() {
        return ClassicalWorkEntry {
            work_title: String::new(),
            movement_title: String::new(),
            recording_key: String::new(),
        };
    }

    let p = patterns();
    for re in [&p.p1, &p.p2, &p.p3, &p.p4] {
        if let Some(caps) = re.captures(raw) {
            let w = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            let mvt = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
            let min_len = if std::ptr::eq(re, &p.p2) { 6 } else { 4 };
            if w.chars().count() >= min_len && !mvt.is_empty() {
                return ClassicalWorkEntry {
                    work_title: w.to_string(),
                    movement_title: mvt.to_string(),
                    recording_key: normalize_work_key(w),
                };
            }
        }
    }

    ClassicalWorkEntry {
        work_title: raw.to_string(),
        movement_title: String::new(),
        recording_key: normalize_work_key(raw),
    }
}

pub fn load_curated_collections(json_str: &str) -> Result<Vec<CuratedCollection>, serde_json::Error> {
    let raw_map: BTreeMap<String, RawCuratedEntry> = serde_json::from_str(json_str)?;
    Ok(raw_map
        .into_iter()
        .map(|(name, e)| CuratedCollection {
            name,
            category: e.cat,
            cover_file: e.cover,
            hue_rgb: e.hue,
            tag_line: e.tag,
            description: e.desc,
        })
        .collect())
}

/// Deduplicate tracks while preserving initial playlist order.
pub fn dedupe_tracks(tracks: &[TrackRecord]) -> Vec<TrackRecord> {
    let mut seen = HashSet::with_capacity(tracks.len());
    let mut out = Vec::with_capacity(tracks.len());
    for t in tracks {
        let key = t.path.replace('\\', "/").trim().to_lowercase();
        if seen.insert(key) {
            out.push(t.clone());
        }
    }
    out
}

/// Disk-backed cover cache with 2x2 RGBA mosaic builder for playlist shelves.
pub struct DiskArtCache {
    root_dir: PathBuf,
}

impl DiskArtCache {
    pub fn new(root_dir: impl AsRef<Path>) -> Self {
        Self {
            root_dir: root_dir.as_ref().to_path_buf(),
        }
    }

    pub fn cache_path_for(&self, album_or_path_key: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(album_or_path_key.as_bytes());
        let hex = format!("{:x}", hasher.finalize());
        self.root_dir.join(format!("{}.rgba", &hex[..24]))
    }

    /// Compose 4 square RGBA tiles (each `tile_size x tile_size`) into a `(2*tile_size) x (2*tile_size)` RGBA mosaic.
    pub fn build_mosaic_2x2(tiles: [&[u8]; 4], tile_size: usize) -> Vec<u8> {
        let expected_tile_bytes = tile_size * tile_size * 4;
        let out_dim = tile_size * 2;
        let mut out = vec![0u8; out_dim * out_dim * 4];

        for (idx, tile) in tiles.iter().enumerate() {
            let qx = (idx % 2) * tile_size;
            let qy = (idx / 2) * tile_size;
            if tile.len() < expected_tile_bytes {
                continue;
            }
            for row in 0..tile_size {
                let src_start = row * tile_size * 4;
                let src_end = src_start + tile_size * 4;
                let dst_start = ((qy + row) * out_dim + qx) * 4;
                let dst_end = dst_start + tile_size * 4;
                out[dst_start..dst_end].copy_from_slice(&tile[src_start..src_end]);
            }
        }
        out
    }
}

/// SQLite FTS5 music library with Classical Work/Cross-Recording indexing.
pub struct LibraryDb {
    conn: Connection,
}

impl LibraryDb {
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init_schema(conn)
    }

    pub fn open(path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init_schema(conn)
    }

    fn init_schema(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS tracks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                album TEXT NOT NULL,
                duration_ms INTEGER NOT NULL,
                work_title TEXT NOT NULL,
                movement_title TEXT NOT NULL,
                recording_key TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_tracks_recording_key ON tracks(recording_key);
            CREATE VIRTUAL TABLE IF NOT EXISTS tracks_fts USING fts5(
                title,
                artist,
                album,
                work_title,
                movement_title,
                path,
                content='tracks',
                content_rowid='id',
                tokenize='unicode61'
            );
            CREATE TRIGGER IF NOT EXISTS tracks_ai AFTER INSERT ON tracks BEGIN
                INSERT INTO tracks_fts(rowid, title, artist, album, work_title, movement_title, path)
                VALUES (new.id, new.title, new.artist, new.album, new.work_title, new.movement_title, new.path);
            END;
            CREATE TRIGGER IF NOT EXISTS tracks_ad AFTER DELETE ON tracks BEGIN
                INSERT INTO tracks_fts(tracks_fts, rowid, title, artist, album, work_title, movement_title, path)
                VALUES ('delete', old.id, old.title, old.artist, old.album, old.work_title, old.movement_title, old.path);
            END;
            CREATE TRIGGER IF NOT EXISTS tracks_au AFTER UPDATE ON tracks BEGIN
                INSERT INTO tracks_fts(tracks_fts, rowid, title, artist, album, work_title, movement_title, path)
                VALUES ('delete', old.id, old.title, old.artist, old.album, old.work_title, old.movement_title, old.path);
                INSERT INTO tracks_fts(rowid, title, artist, album, work_title, movement_title, path)
                VALUES (new.id, new.title, new.artist, new.album, new.work_title, new.movement_title, new.path);
            END;
            "#,
        )?;
        Ok(Self { conn })
    }

    pub fn upsert_track(
        &mut self,
        path: &str,
        title: &str,
        artist: &str,
        album: &str,
        duration_ms: u64,
    ) -> rusqlite::Result<i64> {
        let cw = parse_work_and_movement(title);
        self.conn.execute(
            r#"
            INSERT INTO tracks (path, title, artist, album, duration_ms, work_title, movement_title, recording_key)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(path) DO UPDATE SET
                title=excluded.title,
                artist=excluded.artist,
                album=excluded.album,
                duration_ms=excluded.duration_ms,
                work_title=excluded.work_title,
                movement_title=excluded.movement_title,
                recording_key=excluded.recording_key
            "#,
            params![
                path,
                title,
                artist,
                album,
                duration_ms as i64,
                cw.work_title,
                cw.movement_title,
                cw.recording_key,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn upsert_batch(
        &mut self,
        items: &[(&str, &str, &str, &str, u64)],
    ) -> rusqlite::Result<usize> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                r#"
                INSERT INTO tracks (path, title, artist, album, duration_ms, work_title, movement_title, recording_key)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                ON CONFLICT(path) DO UPDATE SET
                    title=excluded.title,
                    artist=excluded.artist,
                    album=excluded.album,
                    duration_ms=excluded.duration_ms,
                    work_title=excluded.work_title,
                    movement_title=excluded.movement_title,
                    recording_key=excluded.recording_key
                "#,
            )?;
            for &(path, title, artist, album, duration_ms) in items {
                let cw = parse_work_and_movement(title);
                stmt.execute(params![
                    path,
                    title,
                    artist,
                    album,
                    duration_ms as i64,
                    cw.work_title,
                    cw.movement_title,
                    cw.recording_key,
                ])?;
            }
        }
        tx.commit()?;
        Ok(items.len())
    }

    pub fn search_fts(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<TrackRecord>> {
        let tokens: Vec<String> = query
            .split_whitespace()
            .filter_map(|tok| {
                let cleaned: String = tok
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if cleaned.is_empty() {
                    None
                } else {
                    Some(format!("{cleaned}*"))
                }
            })
            .collect();
        if tokens.is_empty() {
            return self.list_all(limit);
        }
        let fts_expr = tokens.join(" ");
        let mut stmt = self.conn.prepare_cached(
            r#"
            SELECT t.id, t.path, t.title, t.artist, t.album, t.duration_ms, t.work_title, t.movement_title, t.recording_key
            FROM tracks_fts f
            JOIN tracks t ON t.id = f.rowid
            WHERE tracks_fts MATCH ?1
            ORDER BY bm25(tracks_fts)
            LIMIT ?2
            "#,
        )?;
        let rows = stmt.query_map(params![fts_expr, limit as i64], Self::row_to_track)?;
        rows.collect()
    }

    pub fn list_all(&self, limit: usize) -> rusqlite::Result<Vec<TrackRecord>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, path, title, artist, album, duration_ms, work_title, movement_title, recording_key FROM tracks ORDER BY id LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], Self::row_to_track)?;
        rows.collect()
    }

    /// Group different albums/artists recording the same classical work.
    pub fn cross_recording_index(&self, recording_key: &str) -> rusqlite::Result<BTreeMap<String, Vec<TrackRecord>>> {
        let key = normalize_work_key(recording_key);
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, path, title, artist, album, duration_ms, work_title, movement_title, recording_key FROM tracks WHERE recording_key = ?1 ORDER BY album, id",
        )?;
        let rows = stmt.query_map(params![key], Self::row_to_track)?;
        let mut grouped: BTreeMap<String, Vec<TrackRecord>> = BTreeMap::new();
        for r in rows {
            let track = r?;
            let rec_label = format!("{} — {}", track.artist, track.album);
            grouped.entry(rec_label).or_default().push(track);
        }
        Ok(grouped)
    }

    pub fn track_count(&self) -> rusqlite::Result<usize> {
        let count: i64 = self.conn.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))?;
        Ok(count as usize)
    }

    pub fn list_works(&self) -> rusqlite::Result<Vec<(String, usize)>> {
        let mut stmt = self.conn.prepare(
            "SELECT work_title, COUNT(*) FROM tracks WHERE movement_title != '' GROUP BY recording_key ORDER BY work_title",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize)))?;
        let mut counts: HashMap<String, usize> = HashMap::new();
        let mut out = Vec::new();
        for row in rows {
            let (w, c) = row?;
            *counts.entry(w.clone()).or_insert(0) += c;
            out.push((w, c));
        }
        Ok(out)
    }

    fn row_to_track(r: &rusqlite::Row<'_>) -> rusqlite::Result<TrackRecord> {
        Ok(TrackRecord {
            id: r.get(0)?,
            path: r.get(1)?,
            title: r.get(2)?,
            artist: r.get(3)?,
            album: r.get(4)?,
            duration_ms: r.get::<_, i64>(5)? as u64,
            work_title: r.get(6)?,
            movement_title: r.get(7)?,
            recording_key: r.get(8)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn test_all_four_classical_work_patterns() {
        let c1 = parse_work_and_movement("Symphony No. 3 in F major, Op. 90 — I. Allegro con brio");
        assert_eq!(c1.work_title, "Symphony No. 3 in F major, Op. 90");
        assert_eq!(c1.movement_title, "I. Allegro con brio");

        let c2 = parse_work_and_movement("Piano Concerto No.1 in F sharp minor, Op.1 - 1 Vivace");
        assert_eq!(c2.work_title, "Piano Concerto No.1 in F sharp minor, Op.1");
        assert_eq!(c2.movement_title, "1 Vivace");

        let c3 = parse_work_and_movement("Cello Suite No. 1 Movement No. 3 Courante");
        assert_eq!(c3.work_title, "Cello Suite No. 1");
        assert_eq!(c3.movement_title, "Movement No. 3 Courante");

        let c4 = parse_work_and_movement("Low Mist Var. 1 (Day 2)");
        assert_eq!(c4.work_title, "Low Mist");
        assert_eq!(c4.movement_title, "Var. 1 (Day 2)");
    }

    #[test]
    fn test_fts5_search_and_cross_recording_index() {
        let mut db = LibraryDb::open_in_memory().unwrap();
        let mut batch = Vec::new();
        let owned: Vec<(String, String, String, String)> = (0..1000)
            .map(|i| {
                (
                    format!("/music/track_{i:04}.flac"),
                    format!("Etude No. {i} — I. Allegro"),
                    "Artist Sample".to_string(),
                    "Complete Works".to_string(),
                )
            })
            .collect();
        for (p, t, ar, al) in &owned {
            batch.push((p.as_str(), t.as_str(), ar.as_str(), al.as_str(), 240_000u64));
        }
        batch.push((
            "/music/brahms_mehta_1.flac",
            "Brahms Symphony No. 3 — I. Allegro con brio",
            "Zubin Mehta",
            "Munich 2024",
            580_000,
        ));
        batch.push((
            "/music/brahms_furt_1.flac",
            "Brahms Symphony No. 3 — I. Allegro con brio",
            "Wilhelm Furtwangler",
            "Berlin 1949",
            610_000,
        ));
        db.upsert_batch(&batch).unwrap();

        let t0 = Instant::now();
        let hits = db.search_fts("Brahms Allegro", 20).unwrap();
        assert!(t0.elapsed().as_millis() < 50);
        assert_eq!(hits.len(), 2);

        let cross = db.cross_recording_index("Brahms Symphony No. 3").unwrap();
        assert_eq!(cross.len(), 2);
        assert!(cross.contains_key("Zubin Mehta — Munich 2024"));
        assert!(cross.contains_key("Wilhelm Furtwangler — Berlin 1949"));
    }

    #[test]
    fn test_curated_collections_mosaic_and_dedupe() {
        let json = r#"{
            "Brahms - Piano Concertos": {
                "cat": "Classical",
                "cover": "pl_01.png",
                "hue": [72, 92, 76],
                "tag": "CLASSICAL · EMI",
                "desc": "Kovacevich & Sawallisch"
            }
        }"#;
        let cols = load_curated_collections(json).unwrap();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].hue_rgb, [72, 92, 76]);

        let t0 = vec![10u8; 4 * 4 * 4];
        let t1 = vec![20u8; 4 * 4 * 4];
        let t2 = vec![30u8; 4 * 4 * 4];
        let t3 = vec![40u8; 4 * 4 * 4];
        let mosaic = DiskArtCache::build_mosaic_2x2([&t0, &t1, &t2, &t3], 4);
        assert_eq!(mosaic.len(), 8 * 8 * 4);
        assert_eq!(mosaic[0], 10);
        assert_eq!(mosaic[4 * 4], 20);
        assert_eq!(mosaic[(4 * 8) * 4], 30);
        assert_eq!(mosaic[(4 * 8 + 4) * 4], 40);

        let rec = TrackRecord {
            id: 1,
            path: "C:\\Music\\a.flac".into(),
            title: "A".into(),
            artist: "B".into(),
            album: "C".into(),
            duration_ms: 1000,
            work_title: "A".into(),
            movement_title: String::new(),
            recording_key: "a".into(),
        };
        let mut dup = rec.clone();
        dup.path = "c:/music/a.flac".into();
        let deduped = dedupe_tracks(&[rec, dup]);
        assert_eq!(deduped.len(), 1);
    }
}
