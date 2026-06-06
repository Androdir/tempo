//! Automatic proof-of-output detection.
//!
//! A polling folder watcher: every tick it scans the user's enabled watched
//! folders for **new or newly-modified files** (metadata only — contents are
//! never read) and records an `output_event`. Polling (vs. an OS file-watch
//! dependency) keeps it robust, dependency-free, and easy to unit-test.

use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use rusqlite::{params, Connection};
use tauri::{AppHandle, Emitter};

use crate::db::Db;
use crate::models::WatchedFolder;
use crate::settings;

const TICK: Duration = Duration::from_secs(20);
const MAX_DEPTH: usize = 5;
const SCAN_CAP: usize = 4000;
const SKIP_DIRS: &[&str] = &[
    "node_modules", "target", "dist", "build", ".next", "venv", "__pycache__", ".cache", "vendor",
];

/// File metadata for a detection candidate (no contents read).
#[derive(Clone)]
pub struct FileMeta {
    pub path: String,
    pub name: String,
    pub ext: String, // lowercased, no dot
    pub size: i64,
    pub modified: DateTime<Utc>,
    pub created: Option<DateTime<Utc>>,
}

/// A file that matched a folder's rules and should be recorded.
pub struct DetectedOutput {
    pub folder_path: String,
    pub file_path: String,
    pub file_name: String,
    pub extension: String,
    pub file_size: i64,
    pub event_type: String,
    pub project: Option<String>,
    pub modified_at: String,
    pub created_at: Option<String>,
    pub day: String,
}

/// Map a folder's declared type + a file extension to an output event type
/// (the "default detection rules"). Extension-specific rules win; otherwise the
/// folder's own type is used.
pub fn classify_output(folder_type: &str, ext: &str) -> String {
    let by_ext = match ext {
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" => Some("video_export"),
        "prproj" | "drp" | "veg" | "fcpxml" | "aep" => Some("editing_project_changed"),
        "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "java" | "c" | "cpp" | "cc" | "h"
        | "hpp" | "cs" | "rb" | "php" | "swift" | "kt" | "css" | "scss" | "html" | "sql" | "sh"
        | "vue" | "svelte" | "lua" | "dart" => Some("code_change"),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "psd" | "ai" | "mp3" | "wav" | "aac" => {
            Some("content_asset")
        }
        _ => None,
    };
    if let Some(e) = by_ext {
        return e.to_string();
    }
    // Documents depend on intent: study folders → study_material, else document.
    if matches!(ext, "pdf" | "docx" | "pptx" | "doc" | "odt" | "epub") {
        return if folder_type == "study_material" {
            "study_material".to_string()
        } else {
            "document_created".to_string()
        };
    }
    match folder_type {
        "download" | "study_material" | "video_export" | "code_change" | "document_created" => {
            folder_type.to_string()
        }
        _ => "other".to_string(),
    }
}

/// Pure detection: given a folder's rules and the current file listing, return
/// the files that should become output events. Testable without a filesystem.
pub fn detect_events(folder: &WatchedFolder, files: &[FileMeta], now: DateTime<Utc>) -> Vec<DetectedOutput> {
    if !folder.enabled {
        return Vec::new();
    }
    // Only files modified after the folder was added (avoids importing history).
    let since = DateTime::parse_from_rfc3339(&folder.created_at)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| now - chrono::Duration::days(3650));
    let settle = chrono::Duration::seconds(folder.debounce_seconds.max(0));
    let exts: Vec<String> = folder.extensions.iter().map(|e| e.trim_start_matches('.').to_ascii_lowercase()).collect();

    let mut out = Vec::new();
    for f in files {
        if !exts.is_empty() && !exts.iter().any(|e| e == &f.ext) {
            continue;
        }
        if f.size < folder.min_size_bytes {
            continue;
        }
        if f.modified < since {
            continue; // predates the folder being watched
        }
        if f.modified > now - settle {
            continue; // not yet settled (debounce window)
        }
        out.push(DetectedOutput {
            folder_path: folder.path.clone(),
            file_path: f.path.clone(),
            file_name: f.name.clone(),
            extension: f.ext.clone(),
            file_size: f.size,
            event_type: classify_output(&folder.output_type, &f.ext),
            project: folder.project.clone(),
            modified_at: f.modified.to_rfc3339(),
            created_at: f.created.map(|c| c.to_rfc3339()),
            day: f.modified.with_timezone(&Local).format("%Y-%m-%d").to_string(),
        });
    }
    out
}

fn file_meta(p: &Path) -> Option<FileMeta> {
    let md = std::fs::metadata(p).ok()?;
    if !md.is_file() {
        return None;
    }
    let modified: DateTime<Utc> = md.modified().ok()?.into();
    let created: Option<DateTime<Utc>> = md.created().ok().map(Into::into);
    let name = p.file_name()?.to_string_lossy().to_string();
    let ext = p
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    Some(FileMeta {
        path: p.to_string_lossy().to_string(),
        name,
        ext,
        size: md.len() as i64,
        modified,
        created,
    })
}

/// Shallow-recursive scan (bounded depth, junk dirs skipped). Metadata only.
fn scan_folder(root: &Path) -> Vec<FileMeta> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if out.len() >= SCAN_CAP {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                if depth + 1 > MAX_DEPTH {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                stack.push((entry.path(), depth + 1));
            } else if ft.is_file() {
                if let Some(fm) = file_meta(&entry.path()) {
                    out.push(fm);
                }
                if out.len() >= SCAN_CAP {
                    break;
                }
            }
        }
    }
    out
}

pub fn load_folders(conn: &Connection) -> Vec<WatchedFolder> {
    let mut stmt = match conn.prepare(
        "SELECT id, path, label, project, output_type, enabled, extensions, min_size_bytes,
                debounce_seconds, created_at
         FROM watched_folders ORDER BY id",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map([], |r| {
        let exts: String = r.get(6)?;
        Ok(WatchedFolder {
            id: r.get(0)?,
            path: r.get(1)?,
            label: r.get(2)?,
            project: r.get(3)?,
            output_type: r.get(4)?,
            enabled: r.get::<_, i64>(5)? != 0,
            extensions: serde_json::from_str(&exts).unwrap_or_default(),
            min_size_bytes: r.get(7)?,
            debounce_seconds: r.get(8)?,
            created_at: r.get(9)?,
        })
    });
    match rows {
        Ok(rs) => rs.filter_map(Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}

/// Insert a detected output (idempotent on file_path+modified_at). Returns 1 if new.
pub fn insert_event(conn: &Connection, d: &DetectedOutput) -> i64 {
    conn.execute(
        "INSERT OR IGNORE INTO output_events
           (timestamp, day, folder_path, file_path, file_name, extension, file_size,
            event_type, project, created_at, modified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            Utc::now().to_rfc3339(),
            d.day,
            d.folder_path,
            d.file_path,
            d.file_name,
            d.extension,
            d.file_size,
            d.event_type,
            d.project,
            d.created_at,
            d.modified_at,
        ],
    )
    .unwrap_or(0) as i64
}

/// Scan all enabled folders once and persist any new outputs. Returns how many
/// were newly recorded. Shared by the worker and the manual "Scan now" command.
pub fn scan_once(db: &Db) -> i64 {
    let folders: Vec<WatchedFolder> = {
        let Ok(conn) = db.lock() else { return 0 };
        if !settings::get_bool(&conn, settings::OUTPUT_WATCH_ENABLED, true) {
            return 0;
        }
        load_folders(&conn).into_iter().filter(|f| f.enabled).collect()
    };
    if folders.is_empty() {
        return 0;
    }
    let now = Utc::now();
    let mut detected: Vec<DetectedOutput> = Vec::new();
    for f in &folders {
        let files = scan_folder(Path::new(&f.path)); // FS I/O outside the lock
        detected.extend(detect_events(f, &files, now));
    }
    if detected.is_empty() {
        return 0;
    }
    let Ok(conn) = db.lock() else { return 0 };
    let mut inserted = 0;
    for d in &detected {
        inserted += insert_event(&conn, d);
    }
    if inserted > 0 {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let _ = crate::aggregate::link_outputs_for_day(&conn, &today);
    }
    inserted
}

pub fn start(db: Db, app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(TICK);
        let inserted = scan_once(&db);
        if inserted > 0 {
            let _ = app.emit("outputs-updated", inserted);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(output_type: &str) -> WatchedFolder {
        WatchedFolder {
            id: 1,
            path: "/x".into(),
            label: "Exports".into(),
            project: Some("Video content business".into()),
            output_type: output_type.into(),
            enabled: true,
            extensions: vec![],
            min_size_bytes: 0,
            debounce_seconds: 5,
            created_at: "2020-01-01T00:00:00+00:00".into(),
        }
    }

    fn file(name: &str, ext: &str, size: i64, mins_ago: i64, now: DateTime<Utc>) -> FileMeta {
        FileMeta {
            path: format!("/x/{name}"),
            name: name.into(),
            ext: ext.into(),
            size,
            modified: now - chrono::Duration::minutes(mins_ago),
            created: None,
        }
    }

    #[test]
    fn classifies_extensions() {
        assert_eq!(classify_output("video_export", "mp4"), "video_export");
        assert_eq!(classify_output("video_export", "prproj"), "editing_project_changed");
        assert_eq!(classify_output("code_change", "rs"), "code_change");
        assert_eq!(classify_output("study_material", "pdf"), "study_material");
        assert_eq!(classify_output("download", "pdf"), "document_created");
        assert_eq!(classify_output("download", "zip"), "download");
        assert_eq!(classify_output("video_export", "png"), "content_asset");
    }

    #[test]
    fn detects_new_video_as_video_export() {
        let now = Utc::now();
        let f = folder("video_export");
        let files = vec![file("final.mp4", "mp4", 5_000_000, 2, now)];
        let ev = detect_events(&f, &files, now);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].event_type, "video_export");
        assert_eq!(ev[0].project.as_deref(), Some("Video content business"));
    }

    #[test]
    fn ignores_files_below_minimum_size() {
        let now = Utc::now();
        let mut f = folder("video_export");
        f.min_size_bytes = 1_000_000;
        let files = vec![file("tiny.mp4", "mp4", 1000, 2, now)];
        assert!(detect_events(&f, &files, now).is_empty());
    }

    #[test]
    fn respects_extension_filter_and_debounce() {
        let now = Utc::now();
        let mut f = folder("video_export");
        f.extensions = vec!["mp4".into()];
        // wrong extension, and an in-progress (not settled) file
        let files = vec![
            file("notes.txt", "txt", 9_000_000, 30, now),
            file("rendering.mp4", "mp4", 9_000_000, 0, now), // modified just now -> within debounce
        ];
        assert!(detect_events(&f, &files, now).is_empty());
    }

    #[test]
    fn disabled_folder_yields_nothing() {
        let now = Utc::now();
        let mut f = folder("video_export");
        f.enabled = false;
        let files = vec![file("final.mp4", "mp4", 5_000_000, 2, now)];
        assert!(detect_events(&f, &files, now).is_empty());
    }

    #[test]
    fn ignores_files_predating_the_folder() {
        let now = Utc::now();
        let mut f = folder("video_export");
        f.created_at = now.to_rfc3339(); // added just now
        let files = vec![file("old.mp4", "mp4", 5_000_000, 120, now)]; // 2h old
        assert!(detect_events(&f, &files, now).is_empty());
    }
}
