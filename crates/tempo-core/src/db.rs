use rusqlite::{backup::Backup, Connection};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Shared handle to the single SQLite connection.
pub type Db = Arc<Mutex<Connection>>;

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;

-- One row per ~10s sample of the foreground desktop window.
CREATE TABLE IF NOT EXISTS activity_log (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp        TEXT    NOT NULL,           -- RFC3339 (UTC) instant of the sample
    day              TEXT    NOT NULL,           -- local calendar day, YYYY-MM-DD
    app_name         TEXT    NOT NULL,           -- process / application name
    window_title     TEXT    NOT NULL,           -- foreground window title
    executable_path  TEXT,                       -- used locally for stable app/game identity
    duration_seconds INTEGER NOT NULL,           -- length of this sample (~10s)
    is_idle          INTEGER NOT NULL DEFAULT 0  -- 0 = active, 1 = idle (no input)
);

CREATE INDEX IF NOT EXISTS idx_activity_day ON activity_log(day);
CREATE INDEX IF NOT EXISTS idx_activity_app ON activity_log(app_name);

-- User-assigned, locally-stored category per app.
CREATE TABLE IF NOT EXISTS category_rules (
    app_name   TEXT PRIMARY KEY,
    category   TEXT NOT NULL,
    ai_review  INTEGER NOT NULL DEFAULT 0,  -- always send this app to the LLM
    updated_at TEXT NOT NULL
);

-- Editable category definitions. The six built-ins are seeded as editable rows;
-- users can add more categories that still roll up into one of the three buckets.
CREATE TABLE IF NOT EXISTS category_definitions (
    id         TEXT PRIMARY KEY,
    label      TEXT NOT NULL,
    color      TEXT NOT NULL,
    bucket     TEXT NOT NULL DEFAULT 'neutral',
    blurb      TEXT NOT NULL DEFAULT '',
    built_in   INTEGER NOT NULL DEFAULT 0,
    sort_order INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);

-- One row per ~10s sample of the active browser tab (from the extension).
CREATE TABLE IF NOT EXISTS browser_activity (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp               TEXT    NOT NULL,
    day                     TEXT    NOT NULL,
    domain                  TEXT    NOT NULL,
    url                     TEXT    NOT NULL,
    page_title              TEXT    NOT NULL,
    duration_seconds        INTEGER NOT NULL,
    content_capture_enabled INTEGER NOT NULL DEFAULT 0,
    content_type            TEXT,
    raw_text_excerpt        TEXT,
    content_summary         TEXT,
    detected_keywords       TEXT,                -- JSON array of strings
    is_idle                 INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_browser_day ON browser_activity(day);
CREATE INDEX IF NOT EXISTS idx_browser_domain ON browser_activity(domain);

-- Per-domain category + capture mode (the allow/block + per-domain editor).
-- capture_mode: 'text' (readable text), 'meta' (url/title only), 'never'.
CREATE TABLE IF NOT EXISTS domain_rules (
    domain       TEXT PRIMARY KEY,
    category     TEXT,
    capture_mode TEXT NOT NULL DEFAULT 'meta',
    ai_review    INTEGER NOT NULL DEFAULT 0,  -- always send this domain to the LLM
    updated_at   TEXT NOT NULL
);

-- Simple key/value app settings (privacy toggles, ingest token, etc.).
CREATE TABLE IF NOT EXISTS app_settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Cached local-LLM classification per activity block (so the slow LLM runs in
-- the background, not on every read). Reads fall back to rule-based when absent.
CREATE TABLE IF NOT EXISTS llm_classification (
    block_key  TEXT PRIMARY KEY,
    day        TEXT NOT NULL,
    category   TEXT,
    productive INTEGER,
    confidence REAL,
    project    TEXT,
    reason     TEXT,
    model      TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_llm_day ON llm_classification(day);

-- Local error log for the LLM (failures / invalid JSON). Never leaves the device.
CREATE TABLE IF NOT EXISTS llm_errors (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL,
    context   TEXT,
    message   TEXT NOT NULL
);

-- Personal Smart Tracking Mode: screen-OCR derived activity.
-- NOTE: never stores the screenshot or the full OCR text — only a summary,
-- keywords, and the classification. Screenshots live in RAM only.
CREATE TABLE IF NOT EXISTS smart_activity (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp         TEXT    NOT NULL,
    day               TEXT    NOT NULL,
    app_name          TEXT    NOT NULL,
    window_title      TEXT    NOT NULL,
    ocr_summary       TEXT,
    detected_keywords TEXT,                  -- JSON array
    category          TEXT,
    is_idle           INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_smart_day ON smart_activity(day);

-- User-defined projects/goals used to classify activity by intent.
CREATE TABLE IF NOT EXISTS projects (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    name              TEXT    NOT NULL,
    category          TEXT    NOT NULL,
    keywords          TEXT    NOT NULL DEFAULT '[]',  -- JSON array
    apps              TEXT    NOT NULL DEFAULT '[]',  -- JSON array
    domains           TEXT    NOT NULL DEFAULT '[]',  -- JSON array
    excluded_apps     TEXT    NOT NULL DEFAULT '[]',  -- hard project exclusions
    excluded_domains  TEXT    NOT NULL DEFAULT '[]',
    excluded_keywords TEXT    NOT NULL DEFAULT '[]',
    priority          INTEGER NOT NULL DEFAULT 50,
    updated_at        TEXT    NOT NULL
);

-- Manual user corrections per activity block (highest-priority classifier).
-- category is one of the six, or 'ignore'.
CREATE TABLE IF NOT EXISTS manual_corrections (
    block_key  TEXT PRIMARY KEY,
    day        TEXT NOT NULL,
    source     TEXT,
    label      TEXT,
    title      TEXT,
    category   TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_manual_day ON manual_corrections(day);

-- Audit trail for user classification changes. This makes every correction
-- explainable and reversible without silently erasing the previous rule.
CREATE TABLE IF NOT EXISTS correction_history (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    block_key                TEXT NOT NULL,
    day                      TEXT NOT NULL,
    source                   TEXT NOT NULL,
    label                    TEXT NOT NULL,
    title                    TEXT NOT NULL,
    previous_manual_category TEXT,
    previous_rule_category   TEXT,
    new_category             TEXT NOT NULL,
    created_at               TEXT NOT NULL,
    undone_at                TEXT
);
CREATE INDEX IF NOT EXISTS idx_correction_block ON correction_history(block_key, id DESC);
-- Daily self-reported check-ins for the productivity score (events that can't be
-- inferred from activity: main goal done, videos posted, gym logged).
-- The individual check-in fields are legacy columns; new values live in
-- checkin_values keyed by user-editable checkin_definitions. main_goal_completed
-- and notes still live here.
CREATE TABLE IF NOT EXISTS daily_checkin (
    day                 TEXT PRIMARY KEY,
    main_goal_completed INTEGER NOT NULL DEFAULT 0,
    videos_posted       INTEGER NOT NULL DEFAULT 0,
    gym_logged          INTEGER NOT NULL DEFAULT 0,
    notes               TEXT,
    wrestled            INTEGER NOT NULL DEFAULT 0,
    studied             INTEGER NOT NULL DEFAULT 0,
    edited_video        INTEGER NOT NULL DEFAULT 0,
    analysed_content    INTEGER NOT NULL DEFAULT 0
);

-- Editable check-in definitions ("things the tracker can't see"). The built-ins
-- are seeded as editable rows; users can add/rename/delete their own.
-- kind: 'toggle' (done / not done) or 'counter' (0..N per day).
-- auto_kind: '' = manual only, 'target' = met after auto_threshold active minutes
-- on an app/site matching auto_metric, 'output' = met after auto_threshold
-- detected output files (auto_metric = output type or watched-folder label,
-- blank = any). A manual checkin_values row always overrides auto detection.
CREATE TABLE IF NOT EXISTS checkin_definitions (
    id             TEXT PRIMARY KEY,
    label          TEXT NOT NULL,
    icon           TEXT NOT NULL DEFAULT '✅',
    kind           TEXT NOT NULL DEFAULT 'toggle',
    built_in       INTEGER NOT NULL DEFAULT 0,
    sort_order     INTEGER NOT NULL DEFAULT 0,
    auto_kind      TEXT NOT NULL DEFAULT '',
    auto_metric    TEXT NOT NULL DEFAULT '',
    auto_threshold INTEGER NOT NULL DEFAULT 0,
    updated_at     TEXT NOT NULL
);

-- One row per (day, check-in) actually logged.
CREATE TABLE IF NOT EXISTS checkin_values (
    day        TEXT NOT NULL,
    checkin_id TEXT NOT NULL,
    value      INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (day, checkin_id)
);
CREATE INDEX IF NOT EXISTS idx_checkin_values_day ON checkin_values(day);

-- Editable daily-score rules. The built-in rule set is seeded as editable rows;
-- users can re-weight, re-threshold, delete, or add their own.
-- kind: checkin | category | target | goal | no_goal | late_start | output.
CREATE TABLE IF NOT EXISTS score_rules (
    id         TEXT PRIMARY KEY,
    label      TEXT NOT NULL,
    kind       TEXT NOT NULL,
    metric     TEXT NOT NULL DEFAULT '',
    weight     INTEGER NOT NULL DEFAULT 0,
    threshold  INTEGER,
    built_in   INTEGER NOT NULL DEFAULT 0,
    sort_order INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);

-- User-defined daily goals ("main missions today").
CREATE TABLE IF NOT EXISTS goals (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    day            TEXT    NOT NULL,
    title          TEXT    NOT NULL,
    project        TEXT,
    target_minutes INTEGER,
    priority       TEXT    NOT NULL DEFAULT 'medium', -- low | medium | high
    completed      INTEGER NOT NULL DEFAULT 0,
    sort_order     INTEGER NOT NULL DEFAULT 0,
    recurring      INTEGER NOT NULL DEFAULT 0,        -- repeats each day
    created_at     TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_goals_day ON goals(day);

-- Focus sessions ("focus mode"): a goal + duration + allowed/blocked lists.
-- Enforcement is soft (warnings); we never block apps at the OS level.
CREATE TABLE IF NOT EXISTS focus_sessions (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    goal             TEXT,
    started_at       TEXT    NOT NULL,           -- RFC3339 (UTC)
    duration_minutes INTEGER NOT NULL,
    ends_at          TEXT    NOT NULL,           -- RFC3339 (UTC)
    allowed          TEXT    NOT NULL DEFAULT '[]', -- JSON array of apps/domains
    blocked          TEXT    NOT NULL DEFAULT '[]', -- JSON array of apps/domains
    status           TEXT    NOT NULL DEFAULT 'active', -- active | completed | ended
    ended_at         TEXT
);
CREATE INDEX IF NOT EXISTS idx_focus_status ON focus_sessions(status);

-- Cached local-LLM daily review (one per day). wins/problems are JSON arrays.
CREATE TABLE IF NOT EXISTS daily_review (
    day        TEXT PRIMARY KEY,
    verdict    TEXT,
    wins       TEXT,
    problems   TEXT,
    tomorrow   TEXT,
    roast      TEXT,
    source     TEXT,
    model      TEXT,
    created_at TEXT
);

-- Folders watched for proof-of-output detection (metadata only, never contents).
CREATE TABLE IF NOT EXISTS watched_folders (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    path             TEXT    NOT NULL,
    label            TEXT    NOT NULL,
    project          TEXT,
    output_type      TEXT    NOT NULL,            -- video_export|code_change|document_created|download|study_material|other
    enabled          INTEGER NOT NULL DEFAULT 1,
    extensions       TEXT    NOT NULL DEFAULT '[]', -- JSON array, lowercased, no dot; [] = any
    min_size_bytes   INTEGER NOT NULL DEFAULT 0,
    debounce_seconds INTEGER NOT NULL DEFAULT 5,
    created_at       TEXT    NOT NULL
);

-- Detected output files (one row per file+mtime). Only metadata is stored.
CREATE TABLE IF NOT EXISTS output_events (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp         TEXT    NOT NULL,           -- detection time (RFC3339 UTC)
    day               TEXT    NOT NULL,           -- local day of the file's mtime
    folder_path       TEXT    NOT NULL,
    file_path         TEXT    NOT NULL,
    file_name         TEXT    NOT NULL,
    extension         TEXT,
    file_size         INTEGER NOT NULL DEFAULT 0,
    event_type        TEXT    NOT NULL,
    project           TEXT,
    linked_block_key  TEXT,                       -- activity block this links to (best-effort)
    linked_label      TEXT,                       -- app/domain active around the event
    created_at        TEXT,                       -- file created time (if available)
    modified_at       TEXT,                       -- file modified time
    UNIQUE(file_path, modified_at)
);
CREATE INDEX IF NOT EXISTS idx_output_day ON output_events(day);

-- Consistency streaks for meaningful behaviours.
-- days_per_week: 0 = every day (runs counted in days); 1..7 = "met on N+ days
-- per ISO week" (runs counted in weeks; an in-progress week never breaks a run).
CREATE TABLE IF NOT EXISTS streak_definitions (
    id                 TEXT PRIMARY KEY,          -- stable slug
    name               TEXT    NOT NULL,
    kind               TEXT    NOT NULL,          -- checkin|goal|category|output|block|distraction
    metric             TEXT    NOT NULL,          -- what to read (e.g. category name, checkin field)
    threshold          INTEGER NOT NULL DEFAULT 0, -- minutes / count threshold
    enabled            INTEGER NOT NULL DEFAULT 1,
    sort_order         INTEGER NOT NULL DEFAULT 0,
    best_streak        INTEGER NOT NULL DEFAULT 0,
    last_completed_day TEXT,
    days_per_week      INTEGER NOT NULL DEFAULT 0,
    updated_at         TEXT    NOT NULL
);

-- Cached "Tomorrow's Lock-In Plan", one per source day.
CREATE TABLE IF NOT EXISTS lockin_plans (
    day                TEXT PRIMARY KEY,          -- the day the plan was generated FROM
    main_mission       TEXT,
    secondary_missions TEXT,                      -- JSON array
    first_block        TEXT,
    distraction_rule   TEXT,
    focus_mode         TEXT,
    avoid_trap         TEXT,
    roast_line         TEXT,
    source             TEXT,                      -- llm|fallback|manual
    edited             INTEGER NOT NULL DEFAULT 0,
    created_at         TEXT
);

-- ============================================================ Tempo Hub sync ==
-- These tables power the optional central "Tempo Hub". On a HUB they hold the
-- master event log + paired devices; on a CLIENT they hold the outbound queue.
-- All idempotent, so local-only installs simply never use them.

-- Master event log on the hub (the dedup ledger). Each device's events are
-- unique by (device_id, event_id); re-uploads are ignored, never double-counted.
CREATE TABLE IF NOT EXISTS synced_events (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id        TEXT    NOT NULL,
    event_id         TEXT    NOT NULL,
    event_type       TEXT    NOT NULL,
    source           TEXT,
    timestamp        TEXT,
    day              TEXT,
    app_name         TEXT,
    domain           TEXT,
    title            TEXT,
    duration_seconds INTEGER,
    category         TEXT,
    project          TEXT,
    metadata_json    TEXT,
    created_at       TEXT,
    UNIQUE(device_id, event_id)
);
CREATE INDEX IF NOT EXISTS idx_synced_day ON synced_events(day);

-- Paired devices on the hub. Only a SHA-256 hash of each device token is stored.
CREATE TABLE IF NOT EXISTS devices (
    id         TEXT PRIMARY KEY,
    name       TEXT,
    platform   TEXT,
    token_hash TEXT    NOT NULL,
    revoked    INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    last_seen  TEXT
);

-- Outbound upload buffer on a CLIENT. Survives restarts → offline buffering.
CREATE TABLE IF NOT EXISTS sync_queue (
    event_id     TEXT PRIMARY KEY,
    payload_json TEXT    NOT NULL,
    status       TEXT    NOT NULL DEFAULT 'pending', -- pending | sent
    attempts     INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT
);

-- Sync failure log (client + hub).
CREATE TABLE IF NOT EXISTS sync_errors (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT,
    context   TEXT,
    message   TEXT
);
"#;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub name: String,
    pub created_at: String,
    pub bytes: u64,
    pub automatic: bool,
}

fn backup_directory(path: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "database has no parent folder".to_string())?;
    Ok(parent.join("backups"))
}

pub fn database_path(conn: &Connection) -> Result<PathBuf, String> {
    let path: String = conn
        .query_row("PRAGMA database_list", [], |row| row.get(2))
        .map_err(|e| e.to_string())?;
    if path.is_empty() {
        return Err("backups are unavailable for an in-memory database".into());
    }
    Ok(PathBuf::from(path))
}

fn backup_to(conn: &Connection, destination: &Path) -> Result<(), String> {
    let mut target = Connection::open(destination).map_err(|e| e.to_string())?;
    let backup = Backup::new(conn, &mut target).map_err(|e| e.to_string())?;
    backup
        .run_to_completion(8, Duration::from_millis(25), None)
        .map_err(|e| e.to_string())?;
    drop(backup);
    let check: String = target
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if check != "ok" {
        let _ = std::fs::remove_file(destination);
        return Err(format!("backup integrity check failed: {check}"));
    }
    Ok(())
}

pub fn create_backup(conn: &Connection, automatic: bool) -> Result<BackupInfo, String> {
    let db_path = database_path(conn)?;
    let dir = backup_directory(&db_path)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S%.6f");
    let prefix = if automatic {
        "tempo-auto"
    } else {
        "tempo-manual"
    };
    let path = dir.join(format!("{prefix}-{stamp}.db"));
    backup_to(conn, &path)?;
    rotate_automatic_backups(&dir, 7)?;
    backup_info(&path)
}

fn create_daily_startup_backup(conn: &Connection, db_path: &Path) -> Result<(), String> {
    let dir = backup_directory(db_path)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let prefix = format!("tempo-auto-{}", chrono::Local::now().format("%Y-%m-%d"));
    let exists = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().starts_with(&prefix));
    if !exists {
        let _ = create_backup(conn, true)?;
    }
    Ok(())
}

fn rotate_automatic_backups(dir: &Path, keep: usize) -> Result<(), String> {
    let mut files = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("tempo-auto-"))
        })
        .collect::<Vec<_>>();
    files.sort();
    let remove_count = files.len().saturating_sub(keep);
    for path in files.into_iter().take(remove_count) {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn backup_info(path: &Path) -> Result<BackupInfo, String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let modified = metadata.modified().map_err(|e| e.to_string())?;
    let created_at: chrono::DateTime<chrono::Local> = modified.into();
    let name = path
        .file_name()
        .ok_or_else(|| "invalid backup filename".to_string())?
        .to_string_lossy()
        .to_string();
    Ok(BackupInfo {
        automatic: name.starts_with("tempo-auto-"),
        name,
        created_at: created_at.to_rfc3339(),
        bytes: metadata.len(),
    })
}

pub fn list_backups(conn: &Connection) -> Result<Vec<BackupInfo>, String> {
    let dir = backup_directory(&database_path(conn)?)?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut backups = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "db"))
        .filter_map(|path| backup_info(&path).ok())
        .collect::<Vec<_>>();
    backups.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(backups)
}

pub fn restore_backup(conn: &mut Connection, name: &str) -> Result<(), String> {
    if Path::new(name).file_name().and_then(|v| v.to_str()) != Some(name)
        || !name.starts_with("tempo-")
        || !name.ends_with(".db")
    {
        return Err("invalid backup name".into());
    }
    let db_path = database_path(conn)?;
    let source_path = backup_directory(&db_path)?.join(name);
    if !source_path.is_file() {
        return Err("backup was not found".into());
    }
    // Always preserve the current database before replacing it.
    let _ = create_backup(conn, false)?;
    let source = Connection::open(&source_path).map_err(|e| e.to_string())?;
    let check: String = source
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if check != "ok" {
        return Err(format!("selected backup failed integrity check: {check}"));
    }
    let backup = Backup::new(&source, conn).map_err(|e| e.to_string())?;
    backup
        .run_to_completion(8, Duration::from_millis(25), None)
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn integrity_check(conn: &Connection) -> Result<String, String> {
    conn.query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|e| e.to_string())
}

pub fn init(path: &Path) -> Result<Db, String> {
    let existed = path.exists()
        && std::fs::metadata(path)
            .map(|m| m.len() > 0)
            .unwrap_or(false);
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    if existed {
        create_daily_startup_backup(&conn, path)?;
    }
    conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    migrate(&conn);
    Ok(Arc::new(Mutex::new(conn)))
}

/// Additive migrations for databases created by earlier versions. Each ADD
/// COLUMN errors harmlessly ("duplicate column") on up-to-date schemas.
fn migrate(conn: &Connection) {
    let _ = conn.execute(
        "ALTER TABLE category_rules ADD COLUMN ai_review INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE domain_rules ADD COLUMN ai_review INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute("ALTER TABLE daily_checkin ADD COLUMN notes TEXT", []);
    for col in ["wrestled", "studied", "edited_video", "analysed_content"] {
        let _ = conn.execute(
            &format!("ALTER TABLE daily_checkin ADD COLUMN {col} INTEGER NOT NULL DEFAULT 0"),
            [],
        );
    }
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS category_definitions (
            id         TEXT PRIMARY KEY,
            label      TEXT NOT NULL,
            color      TEXT NOT NULL,
            bucket     TEXT NOT NULL DEFAULT 'neutral',
            blurb      TEXT NOT NULL DEFAULT '',
            built_in   INTEGER NOT NULL DEFAULT 0,
            sort_order INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL
        )",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE projects ADD COLUMN excluded_apps TEXT NOT NULL DEFAULT '[]'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE projects ADD COLUMN excluded_domains TEXT NOT NULL DEFAULT '[]'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE projects ADD COLUMN excluded_keywords TEXT NOT NULL DEFAULT '[]'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE goals ADD COLUMN recurring INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute("ALTER TABLE goals ADD COLUMN target_count INTEGER", []);
    let _ = conn.execute("ALTER TABLE goals ADD COLUMN target_unit TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE activity_log ADD COLUMN executable_path TEXT",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_activity_day_ts ON activity_log(day, timestamp)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_browser_day_ts ON browser_activity(day, timestamp)",
        [],
    );
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS checkin_definitions (
            id         TEXT PRIMARY KEY,
            label      TEXT NOT NULL,
            icon       TEXT NOT NULL DEFAULT '✅',
            kind       TEXT NOT NULL DEFAULT 'toggle',
            built_in   INTEGER NOT NULL DEFAULT 0,
            sort_order INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL
        )",
        [],
    );
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS checkin_values (
            day        TEXT NOT NULL,
            checkin_id TEXT NOT NULL,
            value      INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (day, checkin_id)
        )",
        [],
    );
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS score_rules (
            id         TEXT PRIMARY KEY,
            label      TEXT NOT NULL,
            kind       TEXT NOT NULL,
            metric     TEXT NOT NULL DEFAULT '',
            weight     INTEGER NOT NULL DEFAULT 0,
            threshold  INTEGER,
            built_in   INTEGER NOT NULL DEFAULT 0,
            sort_order INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL
        )",
        [],
    );
    // Auto-detection source on check-ins; weekly cadence on streaks.
    let _ = conn.execute(
        "ALTER TABLE checkin_definitions ADD COLUMN auto_kind TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE checkin_definitions ADD COLUMN auto_metric TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE checkin_definitions ADD COLUMN auto_threshold INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE streak_definitions ADD COLUMN days_per_week INTEGER NOT NULL DEFAULT 0",
        [],
    );
    migrate_legacy_checkins(conn);
    invalidate_unsafe_llm_cache(conn);
}

/// Cached LLM rows are derived data. Refresh today's pre-guardrail verdicts once
/// and remove model-authored project labels; historical categories, raw activity,
/// and manual fixes are kept.
fn invalidate_unsafe_llm_cache(conn: &Connection) {
    const FLAG: &str = "llm_evidence_guardrails_v1";
    let done = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [FLAG],
            |r| r.get::<_, String>(0),
        )
        .map(|v| v == "1")
        .unwrap_or(false);
    if done {
        return;
    }
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let _ = conn.execute("DELETE FROM llm_classification WHERE day = ?1", [&today]);
    // Project labels are now always resolved from deterministic app/domain/keyword
    // evidence at read time, so old model-authored values are unnecessary.
    let _ = conn.execute("UPDATE llm_classification SET project = NULL", []);
    let _ = conn.execute(
        "INSERT INTO app_settings (key, value) VALUES (?1, '1')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [FLAG],
    );
}

/// One-time copy of the legacy fixed daily_checkin columns into the flexible
/// checkin_values table, so history (streak runs, weekly totals) survives the
/// upgrade. Guarded by a settings flag; INSERT OR IGNORE keeps it idempotent.
fn migrate_legacy_checkins(conn: &Connection) {
    const FLAG: &str = "checkin_values_migrated";
    let done = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [FLAG],
            |r| r.get::<_, String>(0),
        )
        .map(|v| v == "1")
        .unwrap_or(false);
    if done {
        return;
    }
    let now = chrono::Utc::now().to_rfc3339();
    for col in [
        "videos_posted",
        "gym_logged",
        "wrestled",
        "studied",
        "edited_video",
        "analysed_content",
    ] {
        let _ = conn.execute(
            &format!(
                "INSERT OR IGNORE INTO checkin_values (day, checkin_id, value, updated_at)
                 SELECT day, '{col}', {col}, ?1 FROM daily_checkin WHERE {col} != 0"
            ),
            [&now],
        );
    }
    let _ = conn.execute(
        "INSERT INTO app_settings (key, value) VALUES (?1, '1')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [FLAG],
    );
}

/// Delete raw activity older than `days` local days (0 = keep forever). Returns
/// the number of rows removed. The small per-day tables (check-ins, goals,
/// reviews, focus sessions, rules) are kept — only the high-volume sample tables
/// and their derived per-block rows are pruned.
pub fn prune(conn: &Connection, days: i64) -> rusqlite::Result<usize> {
    if days <= 0 {
        return Ok(0);
    }
    let cutoff = (chrono::Local::now().date_naive() - chrono::Duration::days(days))
        .format("%Y-%m-%d")
        .to_string();
    let mut removed = 0;
    for table in [
        "activity_log",
        "browser_activity",
        "smart_activity",
        "llm_classification",
        "manual_corrections",
        "correction_history",
    ] {
        removed += conn.execute(&format!("DELETE FROM {table} WHERE day < ?1"), [&cutoff])?;
    }
    Ok(removed)
}

/// In-memory database with the full schema applied. Used by tests in this crate
/// and in the desktop/hub crates (where this crate's `#[cfg(test)]` is inactive,
/// so the helper must always be compiled).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_backup_can_restore_previous_database_state() {
        let dir = std::env::temp_dir().join(format!(
            "tempo-backup-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("productivity.db");
        let db = init(&path).unwrap();
        let mut conn = db.lock().unwrap();
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES ('proof', 'before')",
            [],
        )
        .unwrap();
        let backup = create_backup(&conn, false).unwrap();
        conn.execute(
            "UPDATE app_settings SET value = 'after' WHERE key = 'proof'",
            [],
        )
        .unwrap();
        restore_backup(&mut conn, &backup.name).unwrap();
        let value: String = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = 'proof'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "before");
        assert_eq!(integrity_check(&conn).unwrap(), "ok");
        drop(conn);
        drop(db);
        assert!(dir.starts_with(std::env::temp_dir()));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
#[doc(hidden)]
pub fn test_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    migrate(&conn);
    conn
}
