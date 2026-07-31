//! Desktop sync client. When `app_mode = hub`, a background worker turns local
//! activity rows into generic events, buffers them in `sync_queue` (so it
//! survives the hub being offline), and uploads batches to the Tempo Hub —
//! retrying on the next tick. Local-only mode does nothing here.

use std::time::Duration;

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::db::Db;
use crate::settings;
use tempo_core::events::{EventBatch, SyncEvent};

const TICK: Duration = Duration::from_secs(30);
const BATCH: usize = 500;

const APP_MODE: &str = "app_mode";
const HUB_URL: &str = "hub_url";
const HUB_TOKEN: &str = "hub_device_token";
const HUB_DEVICE_ID: &str = "hub_device_id";
const SYNC_LAST_AT: &str = "sync_last_at";
const SYNC_CONNECTED: &str = "sync_connected";
const CLASSIFICATION_SNAP_KEY: &str = "sync_snap_classification_bundle";

fn normalize_hub_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("Hub URL is required".into());
    }
    let candidate = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let mut parsed =
        url::Url::parse(&candidate).map_err(|_| "Enter a valid Hub URL".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Hub URL must start with http:// or https://".into());
    }
    if parsed.host_str().is_none() {
        return Err("Hub URL must include a host name or IP address".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("Do not put a username or password in the Hub URL".into());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("Hub URL cannot include a query string or fragment".into());
    }
    let path = parsed.path().trim_end_matches('/');
    if !path.is_empty() {
        return Err("Hub URL should not include /api or another path".into());
    }
    parsed.set_path("");
    Ok(parsed.to_string().trim_end_matches('/').to_string())
}

fn wm_key(table: &str) -> String {
    format!("sync_wm_{table}")
}

/// `Some((url, token, device_id))` when hub sync is configured, else `None`.
fn sync_target(conn: &Connection) -> Option<(String, String, String)> {
    if settings::get_setting(conn, APP_MODE).as_deref() != Some("hub") {
        return None;
    }
    let url = settings::get_setting(conn, HUB_URL).unwrap_or_default();
    let token = settings::get_setting(conn, HUB_TOKEN).unwrap_or_default();
    if url.trim().is_empty() || token.trim().is_empty() {
        return None;
    }
    let device = settings::get_setting(conn, HUB_DEVICE_ID).unwrap_or_default();
    Some((url.trim_end_matches('/').to_string(), token, device))
}

// ------------------------------------------------------ build events from rows

fn enqueue(conn: &Connection, e: &SyncEvent) {
    if let Ok(payload) = serde_json::to_string(e) {
        let _ = conn.execute(
            "INSERT OR IGNORE INTO sync_queue (event_id, payload_json, status, attempts, created_at)
             VALUES (?1, ?2, 'pending', 0, ?3)",
            params![e.event_id, payload, chrono::Utc::now().to_rfc3339()],
        );
    }
}

/// Parse a `"<day>|field=value,…"` snapshot back to a map, or empty if absent or
/// from a different day (so each new day starts from a clean baseline).
fn parse_snap_map(raw: &Option<String>, day: &str) -> std::collections::HashMap<String, i64> {
    let mut out = std::collections::HashMap::new();
    if let Some(s) = raw {
        if let Some((d, rest)) = s.split_once('|') {
            if d == day && !rest.is_empty() {
                for pair in rest.split(',') {
                    if let Some((k, v)) = pair.split_once('=') {
                        if let Ok(v) = v.parse::<i64>() {
                            out.insert(k.to_string(), v);
                        }
                    }
                }
            }
        }
    }
    out
}

/// One check-in change in transit. The definition (label/icon/kind) rides along
/// so a check-in created on this device auto-registers on the hub.
fn checkin_event(
    day: &str,
    field: &str,
    value: i64,
    def: Option<&tempo_core::models::CheckinDefinition>,
    now_ms: i64,
) -> SyncEvent {
    let metadata = match def {
        Some(d) => serde_json::json!({
            "field": field, "value": value,
            "label": d.label, "icon": d.icon, "kind": d.kind,
        }),
        None => serde_json::json!({ "field": field, "value": value }),
    };
    SyncEvent {
        event_id: format!("checkin:{day}:{field}:{now_ms}"),
        event_type: "checkin".into(),
        source: Some("desktop".into()),
        timestamp: chrono::Utc::now().to_rfc3339(),
        day: day.to_string(),
        app_name: None,
        domain: None,
        title: None,
        duration_seconds: None,
        category: None,
        project: None,
        metadata: Some(metadata),
    }
}

/// A manual override was removed on the desktop → the hub deletes its row too,
/// falling back to auto detection there.
fn checkin_cleared_event(day: &str, field: &str, now_ms: i64) -> SyncEvent {
    SyncEvent {
        event_id: format!("checkin:{day}:{field}:clear:{now_ms}"),
        event_type: "checkin".into(),
        source: Some("desktop".into()),
        timestamp: chrono::Utc::now().to_rfc3339(),
        day: day.to_string(),
        app_name: None,
        domain: None,
        title: None,
        duration_seconds: None,
        category: None,
        project: None,
        metadata: Some(serde_json::json!({ "field": field, "cleared": true })),
    }
}

/// Diff the full check-in definition set against the last-synced snapshot and
/// enqueue one `checkin_def` event per created/edited definition (full payload)
/// or per deletion (`deleted: true`). Returns how many events were enqueued.
fn sync_checkin_definitions(
    conn: &Connection,
    defs: &[tempo_core::models::CheckinDefinition],
    day: &str,
    now_ms: i64,
) -> i64 {
    use std::hash::{Hash, Hasher};
    let fingerprint = |d: &tempo_core::models::CheckinDefinition| -> i64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (
            &d.label,
            &d.icon,
            &d.kind,
            &d.auto_kind,
            &d.auto_metric,
            d.auto_threshold,
        )
            .hash(&mut h);
        h.finish() as i64
    };
    const SNAP_KEY: &str = "sync_snap_checkin_defs";
    // The snapshot isn't day-scoped — reuse the parser with a fixed prefix.
    let raw = settings::get_setting(conn, SNAP_KEY);
    let old = parse_snap_map(&raw, "defs");
    let mut n = 0i64;
    let mut cur: Vec<(String, i64)> = Vec::new();
    for d in defs {
        let fp = fingerprint(d);
        cur.push((d.id.clone(), fp));
        if old.get(&d.id).copied() != Some(fp) {
            enqueue(
                conn,
                &SyncEvent {
                    event_id: format!("checkin_def:{}:{now_ms}", d.id),
                    event_type: "checkin_def".into(),
                    source: Some("desktop".into()),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    day: day.to_string(),
                    app_name: None,
                    domain: None,
                    title: None,
                    duration_seconds: None,
                    category: None,
                    project: None,
                    metadata: Some(serde_json::json!({
                        "id": d.id, "label": d.label, "icon": d.icon, "kind": d.kind,
                        "autoKind": d.auto_kind, "autoMetric": d.auto_metric,
                        "autoThreshold": d.auto_threshold,
                    })),
                },
            );
            n += 1;
        }
    }
    for id in old.keys() {
        if !defs.iter().any(|d| &d.id == id) {
            enqueue(
                conn,
                &SyncEvent {
                    event_id: format!("checkin_def:{id}:del:{now_ms}"),
                    event_type: "checkin_def".into(),
                    source: Some("desktop".into()),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    day: day.to_string(),
                    app_name: None,
                    domain: None,
                    title: None,
                    duration_seconds: None,
                    category: None,
                    project: None,
                    metadata: Some(serde_json::json!({ "id": id, "deleted": true })),
                },
            );
            n += 1;
        }
    }
    if n > 0 {
        let snap = format!(
            "defs|{}",
            cur.iter()
                .map(|(f, v)| format!("{f}={v}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        let _ = settings::set_setting(conn, SNAP_KEY, &snap);
    }
    n
}

fn classification_bundle(conn: &Connection) -> serde_json::Value {
    let mut app_rules = Vec::new();
    if let Ok(mut stmt) =
        conn.prepare("SELECT app_name, category, ai_review FROM category_rules ORDER BY app_name")
    {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok(serde_json::json!({
                "appName": r.get::<_, String>(0)?, "category": r.get::<_, String>(1)?,
                "aiReview": r.get::<_, i64>(2)? != 0,
            }))
        }) {
            app_rules = rows.filter_map(Result::ok).collect();
        }
    }
    let mut domain_rules = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT domain, category, capture_mode, ai_review FROM domain_rules ORDER BY domain",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok(serde_json::json!({
                "domain": r.get::<_, String>(0)?, "category": r.get::<_, Option<String>>(1)?,
                "captureMode": r.get::<_, String>(2)?, "aiReview": r.get::<_, i64>(3)? != 0,
            }))
        }) {
            domain_rules = rows.filter_map(Result::ok).collect();
        }
    }
    serde_json::json!({
        "version": 2, "appRules": app_rules, "domainRules": domain_rules,
        "categories": crate::models::list_category_definitions(conn).unwrap_or_default(),
        "projects": crate::projects::list_projects(conn).unwrap_or_default(),
        "policies": crate::semantic::load(conn),
    })
}

fn has_pending_classification_bundle(conn: &Connection, snapshot: &str) -> bool {
    let Ok(mut stmt) = conn.prepare(
        "SELECT payload_json FROM sync_queue
         WHERE status = 'pending' AND event_id LIKE 'classification_bundle:%'",
    ) else {
        return false;
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) else {
        return false;
    };
    let found = rows.filter_map(Result::ok).any(|payload| {
        serde_json::from_str::<SyncEvent>(&payload)
            .ok()
            .and_then(|event| event.metadata)
            .map(|metadata| metadata.to_string() == snapshot)
            .unwrap_or(false)
    });
    found
}

/// Keep classification configuration identical on the desktop and Hub.
/// The snapshot is acknowledged only after a successful HTTP upload.
fn sync_classification_bundle(conn: &Connection, day: &str, now_ms: i64) -> i64 {
    let bundle = classification_bundle(conn);
    let snapshot = bundle.to_string();
    if settings::get_setting(conn, CLASSIFICATION_SNAP_KEY).as_deref() == Some(snapshot.as_str())
        || has_pending_classification_bundle(conn, &snapshot)
    {
        return 0;
    }
    enqueue(
        conn,
        &SyncEvent {
            event_id: format!("classification_bundle:{now_ms}"),
            event_type: "classification_bundle".into(),
            source: Some("desktop".into()),
            timestamp: chrono::Utc::now().to_rfc3339(),
            day: day.to_string(),
            app_name: None,
            domain: None,
            title: None,
            duration_seconds: None,
            category: None,
            project: None,
            metadata: Some(bundle),
        },
    );
    1
}
fn watermark(conn: &Connection, table: &str) -> i64 {
    settings::get_setting(conn, &wm_key(table))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
fn set_watermark(conn: &Connection, table: &str, id: i64) {
    let _ = settings::set_setting(conn, &wm_key(table), &id.to_string());
}

/// Scan the append-only activity tables past their watermark and enqueue the new
/// rows as generic events. Returns how many were newly enqueued.
pub fn scan_and_enqueue(conn: &Connection) -> i64 {
    let mut n = 0i64;

    // Desktop window samples.
    {
        let wm = watermark(conn, "activity_log");
        let mut max_id = wm;
        if let Ok(mut stmt) = conn.prepare(
            "SELECT id, timestamp, day, app_name, window_title, executable_path, duration_seconds, is_idle
             FROM activity_log WHERE id > ?1 ORDER BY id LIMIT 2000",
        ) {
            let rows = stmt.query_map([wm], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, i64>(7)?,
                ))
            });
            if let Ok(rows) = rows {
                for (id, ts, day, app, title, executable_path, dur, idle) in rows.flatten() {
                    enqueue(conn, &SyncEvent {
                        event_id: format!("activity_log:{id}"),
                        event_type: "app_sample".into(),
                        source: Some("desktop".into()),
                        timestamp: ts,
                        day,
                        app_name: Some(app),
                        domain: None,
                        title: Some(title),
                        duration_seconds: Some(dur),
                        category: None,
                        project: None,
                        metadata: Some(serde_json::json!({ "isIdle": idle != 0, "executablePath": executable_path })),
                    });
                    max_id = max_id.max(id);
                    n += 1;
                }
            }
        }
        if max_id > wm {
            set_watermark(conn, "activity_log", max_id);
        }
    }

    // Browser tab samples.
    {
        let wm = watermark(conn, "browser_activity");
        let mut max_id = wm;
        if let Ok(mut stmt) = conn.prepare(
            "SELECT id, timestamp, day, domain, url, page_title, duration_seconds, is_idle,
                    content_type, content_summary, detected_keywords
             FROM browser_activity WHERE id > ?1 ORDER BY id LIMIT 2000",
        ) {
            let rows = stmt.query_map([wm], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, i64>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, Option<String>>(9)?,
                    r.get::<_, Option<String>>(10)?,
                ))
            });
            if let Ok(rows) = rows {
                for (id, ts, day, domain, url, title, dur, idle, ctype, summary, kw) in
                    rows.flatten()
                {
                    enqueue(
                        conn,
                        &SyncEvent {
                            event_id: format!("browser_activity:{id}"),
                            event_type: "browser_sample".into(),
                            source: Some("browser".into()),
                            timestamp: ts,
                            day,
                            app_name: None,
                            domain: Some(domain),
                            title: Some(title),
                            duration_seconds: Some(dur),
                            category: None,
                            project: None,
                            metadata: Some(serde_json::json!({
                                "isIdle": idle != 0, "url": url,
                                "contentType": ctype, "summary": summary, "keywords": kw,
                            })),
                        },
                    );
                    max_id = max_id.max(id);
                    n += 1;
                }
            }
        }
        if max_id > wm {
            set_watermark(conn, "browser_activity", max_id);
        }
    }

    // Detected outputs.
    {
        let wm = watermark(conn, "output_events");
        let mut max_id = wm;
        if let Ok(mut stmt) = conn.prepare(
            "SELECT id, timestamp, day, folder_path, file_path, file_name, extension, file_size,
                    event_type, project, modified_at, created_at
             FROM output_events WHERE id > ?1 ORDER BY id LIMIT 2000",
        ) {
            let rows = stmt.query_map([wm], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, i64>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, Option<String>>(9)?,
                    r.get::<_, Option<String>>(10)?,
                    r.get::<_, Option<String>>(11)?,
                ))
            });
            if let Ok(rows) = rows {
                for (
                    id,
                    ts,
                    day,
                    folder,
                    fpath,
                    fname,
                    ext,
                    size,
                    etype,
                    project,
                    modified,
                    created,
                ) in rows.flatten()
                {
                    enqueue(
                        conn,
                        &SyncEvent {
                            event_id: format!("output_events:{id}"),
                            event_type: "output".into(),
                            source: Some("desktop".into()),
                            timestamp: ts,
                            day,
                            app_name: None,
                            domain: None,
                            title: Some(fname.clone()),
                            duration_seconds: None,
                            category: Some(etype),
                            project,
                            metadata: Some(serde_json::json!({
                                "folderPath": folder, "filePath": fpath, "fileName": fname,
                                "extension": ext, "fileSize": size, "modifiedAt": modified, "createdAt": created,
                            })),
                        },
                    );
                    max_id = max_id.max(id);
                    n += 1;
                }
            }
        }
        if max_id > wm {
            set_watermark(conn, "output_events", max_id);
        }
    }

    // Focus sessions — only terminal ones (status != 'active'), so each uploads
    // exactly once in its final form. The hub recomputes adherence across devices.
    {
        let wm = watermark(conn, "focus_sessions");
        let mut max_id = wm;
        if let Ok(mut stmt) = conn.prepare(
            "SELECT id, goal, started_at, duration_minutes, ends_at, allowed, blocked, status, ended_at
             FROM focus_sessions WHERE id > ?1 AND status != 'active' ORDER BY id LIMIT 2000",
        ) {
            let rows = stmt.query_map([wm], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, Option<String>>(8)?,
                ))
            });
            if let Ok(rows) = rows {
                for (id, goal, started, dur, ends, allowed, blocked, status, ended) in rows.flatten() {
                    let day = started.get(0..10).unwrap_or("").to_string();
                    enqueue(conn, &SyncEvent {
                        event_id: format!("focus_sessions:{id}"),
                        event_type: "focus".into(),
                        source: Some("desktop".into()),
                        timestamp: started.clone(),
                        day,
                        app_name: None,
                        domain: None,
                        title: goal.clone(),
                        duration_seconds: None,
                        category: None,
                        project: None,
                        metadata: Some(serde_json::json!({
                            "goal": goal, "startedAt": started, "durationMinutes": dur,
                            "endsAt": ends, "allowed": allowed, "blocked": blocked,
                            "status": status, "endedAt": ended,
                        })),
                    });
                    max_id = max_id.max(id);
                    n += 1;
                }
            }
        }
        if max_id > wm {
            set_watermark(conn, "focus_sessions", max_id);
        }
    }

    // Mutable "today" state: check-ins, the daily note, and goals. These change in
    // place (no new rows), so we snapshot the current state and enqueue — with a
    // fresh, monotonic event id — only the parts that changed since we last sent.
    // The hub projects each as an idempotent upsert, so re-sends never corrupt.
    {
        let day = chrono::Local::now().format("%Y-%m-%d").to_string();
        let now_ms = chrono::Utc::now().timestamp_millis();

        // --- check-in definitions (label/kind/auto config), synced on change ---
        // The hub needs the definition itself — auto check-ins may never produce a
        // value event here, and the hub recomputes auto detection over *all*
        // devices' activity from the definition.
        let defs = tempo_core::models::list_checkin_definitions(conn).unwrap_or_default();
        n += sync_checkin_definitions(conn, &defs, &day, now_ms);

        // --- check-in values (main_goal_completed + manually-logged rows only) ---
        // Auto-detected values are intentionally NOT synced: the hub derives them
        // itself, so only manual logs/overrides travel.
        let mg: i64 = conn
            .query_row(
                "SELECT main_goal_completed FROM daily_checkin WHERE day = ?1",
                [&day],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let mut manual: Vec<(String, i64)> = vec![("main_goal_completed".to_string(), mg)];
        if let Ok(mut stmt) = conn.prepare(
            "SELECT checkin_id, value FROM checkin_values WHERE day = ?1 ORDER BY checkin_id",
        ) {
            if let Ok(rows) = stmt.query_map([&day], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            }) {
                manual.extend(rows.flatten());
            }
        }
        let old = parse_snap_map(&settings::get_setting(conn, "sync_snap_checkin"), &day);
        let mut changed = false;
        for (field, value) in &manual {
            // An absent snapshot entry means "never sent today": 0 is the implied
            // baseline for main_goal, while a fresh manual row syncs even at 0
            // (it may be an override forcing an auto check-in off).
            let baseline = if field == "main_goal_completed" {
                0
            } else {
                i64::MIN
            };
            if old.get(field).copied().unwrap_or(baseline) != *value {
                let def = defs.iter().find(|d| &d.id == field);
                enqueue(conn, &checkin_event(&day, field, *value, def, now_ms));
                n += 1;
                changed = true;
            }
        }
        // A manual row that disappeared = override cleared → tell the hub to clear too.
        for field in old.keys() {
            if field != "main_goal_completed" && !manual.iter().any(|(f, _)| f == field) {
                enqueue(conn, &checkin_cleared_event(&day, field, now_ms));
                n += 1;
                changed = true;
            }
        }
        if changed {
            let snap = format!(
                "{day}|{}",
                manual
                    .iter()
                    .map(|(f, v)| format!("{f}={v}"))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            let _ = settings::set_setting(conn, "sync_snap_checkin", &snap);
        }

        // --- daily note ---
        let note: String = conn
            .query_row(
                "SELECT notes FROM daily_checkin WHERE day = ?1",
                [&day],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
            .unwrap_or_default();
        let note_snap = format!("{day}|{note}");
        let prev_note = settings::get_setting(conn, "sync_snap_note");
        if prev_note.as_deref() != Some(note_snap.as_str()) {
            // Send when there's a note, or when clearing a note we previously sent today.
            let cleared_today = prev_note
                .map(|s| s.starts_with(&format!("{day}|")))
                .unwrap_or(false);
            if !note.is_empty() || cleared_today {
                enqueue(
                    conn,
                    &SyncEvent {
                        event_id: format!("note:{day}:{now_ms}"),
                        event_type: "note".into(),
                        source: Some("desktop".into()),
                        timestamp: chrono::Utc::now().to_rfc3339(),
                        day: day.clone(),
                        app_name: None,
                        domain: None,
                        title: None,
                        duration_seconds: None,
                        category: None,
                        project: None,
                        metadata: Some(serde_json::json!({ "notes": note })),
                    },
                );
                n += 1;
            }
            let _ = settings::set_setting(conn, "sync_snap_note", &note_snap);
        }

        // --- goals for today (re-send all of today's goals on any change) ---
        let mut goals: Vec<(
            String,
            Option<String>,
            Option<i64>,
            Option<i64>,
            Option<String>,
            String,
            i64,
        )> = Vec::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT title, project, target_minutes, target_count, target_unit, priority, completed FROM goals
             WHERE day = ?1 ORDER BY sort_order, id",
        ) {
            if let Ok(rows) = stmt.query_map([&day], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
            }) {
                goals = rows.filter_map(Result::ok).collect();
            }
        }
        let goal_snap = format!(
            "{day}|{}",
            goals
                .iter()
                .map(|g| {
                    format!(
                        "{}={}={}={}={}={}",
                        g.0,
                        g.6,
                        g.5,
                        g.2.unwrap_or(-1),
                        g.3.unwrap_or(-1),
                        g.4.as_deref().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join(";;"),
        );
        if settings::get_setting(conn, "sync_snap_goals").as_deref() != Some(goal_snap.as_str()) {
            for (title, project, target, target_count, target_unit, priority, completed) in &goals {
                enqueue(
                    conn,
                    &SyncEvent {
                        event_id: format!("goal:{day}:{title}:{now_ms}"),
                        event_type: "goal".into(),
                        source: Some("desktop".into()),
                        timestamp: chrono::Utc::now().to_rfc3339(),
                        day: day.clone(),
                        app_name: None,
                        domain: None,
                        title: Some(title.clone()),
                        duration_seconds: None,
                        category: None,
                        project: project.clone(),
                        metadata: Some(serde_json::json!({
                            "completed": *completed != 0,
                            "priority": priority,
                            "targetMinutes": target,
                            "targetCount": target_count,
                            "targetUnit": target_unit,
                        })),
                    },
                );
                n += 1;
            }
            let _ = settings::set_setting(conn, "sync_snap_goals", &goal_snap);
        }
    }

    let config_day = chrono::Local::now().format("%Y-%m-%d").to_string();
    n += sync_classification_bundle(conn, &config_day, chrono::Utc::now().timestamp_millis());
    n
}

// --------------------------------------------------------------- queue draining

pub fn mark_sent(conn: &Connection, ids: &[String]) {
    for id in ids {
        let _ = conn.execute(
            "UPDATE sync_queue SET status = 'sent' WHERE event_id = ?1",
            params![id],
        );
    }
}

pub fn record_failure(conn: &Connection, ids: &[String], msg: &str) {
    let _ = conn.execute(
        "INSERT INTO sync_errors (timestamp, context, message) VALUES (?1, 'upload', ?2)",
        params![chrono::Utc::now().to_rfc3339(), msg],
    );
    for id in ids {
        let _ = conn.execute(
            "UPDATE sync_queue SET attempts = attempts + 1 WHERE event_id = ?1",
            params![id],
        );
    }
}

fn pending(conn: &Connection) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT event_id, payload_json FROM sync_queue
         WHERE status = 'pending'
         ORDER BY CASE WHEN event_id LIKE 'classification_bundle:%' THEN 0 ELSE 1 END, rowid
         LIMIT ?1",
    ) {
        if let Ok(rows) = stmt.query_map([BATCH as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        }) {
            out = rows.filter_map(Result::ok).collect();
        }
    }
    out
}

fn acknowledge_classification_bundles(conn: &Connection, items: &[(String, String)]) {
    for (_, payload) in items {
        let Ok(event) = serde_json::from_str::<SyncEvent>(payload) else {
            continue;
        };
        if event.event_type != "classification_bundle" {
            continue;
        }
        if let Some(bundle) = event.metadata {
            let _ = settings::set_setting(conn, CLASSIFICATION_SNAP_KEY, &bundle.to_string());
        }
    }
}

fn validate_configuration_ack(body: &serde_json::Value) -> Result<(), String> {
    let version = body
        .get("classificationVersion")
        .and_then(|value| value.as_i64())
        .unwrap_or(0);
    if version < 2 {
        return Err(
            "Tempo Hub is too old for project sync. Update and rebuild the Hub, then try again."
                .into(),
        );
    }
    Ok(())
}

/// Upload one batch. Returns Ok(count_sent) or Err on network/server failure.
/// The HTTP call happens WITHOUT the DB lock held.
fn drain_once(db: &Db, url: &str, token: &str, device_id: &str) -> Result<i64, String> {
    let items = {
        let conn = db.lock().map_err(|e| e.to_string())?;
        pending(&conn)
    };
    if items.is_empty() {
        return Ok(0);
    }
    let events: Vec<SyncEvent> = items
        .iter()
        .filter_map(|(_, p)| serde_json::from_str(p).ok())
        .collect();
    let requires_configuration_ack = events
        .iter()
        .any(|event| event.event_type == "classification_bundle");
    let batch = EventBatch {
        device_id: device_id.to_string(),
        events,
    };
    let body = serde_json::to_value(&batch).map_err(|e| e.to_string())?;

    let result = ureq::post(&format!("{url}/api/events"))
        .timeout(Duration::from_secs(15))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(body);

    let delivery = match result {
        Ok(response) if requires_configuration_ack => response
            .into_json::<serde_json::Value>()
            .map_err(|error| format!("invalid Hub acknowledgement: {error}"))
            .and_then(|body| validate_configuration_ack(&body)),
        Ok(_) => Ok(()),
        Err(error) => Err(error.to_string()),
    };

    let ids: Vec<String> = items.iter().map(|(id, _)| id.clone()).collect();
    let conn = db.lock().map_err(|e| e.to_string())?;
    match delivery {
        Ok(_) => {
            acknowledge_classification_bundles(&conn, &items);
            mark_sent(&conn, &ids);
            Ok(ids.len() as i64)
        }
        Err(message) => {
            record_failure(&conn, &ids, &message);
            Err(message)
        }
    }
}

// ------------------------------------------------------------------- worker

pub fn start(db: Db, app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(TICK);

        let target = {
            let Ok(conn) = db.lock() else { continue };
            sync_target(&conn)
        };
        let Some((url, token, device_id)) = target else {
            continue;
        }; // local mode → idle

        {
            let Ok(conn) = db.lock() else { continue };
            scan_and_enqueue(&conn);
        }

        let outcome = drain_once(&db, &url, &token, &device_id);

        {
            let Ok(conn) = db.lock() else { continue };
            let connected = outcome.is_ok();
            let _ = settings::set_setting(&conn, SYNC_CONNECTED, if connected { "1" } else { "0" });
            if connected {
                let _ =
                    settings::set_setting(&conn, SYNC_LAST_AT, &chrono::Utc::now().to_rfc3339());
            }
        }
        let _ = app.emit("sync-status", ());
    });
}

// ------------------------------------------------------------------- commands

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub mode: String,
    pub connected: bool,
    pub last_sync: Option<String>,
    pub queued: i64,
    pub hub_url: String,
    pub paired: bool,
}

#[tauri::command]
pub fn get_sync_status(db: State<'_, Db>) -> Result<SyncStatus, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let queued: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sync_queue WHERE status = 'pending'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Ok(SyncStatus {
        mode: settings::get_setting(&conn, APP_MODE).unwrap_or_else(|| "local".into()),
        connected: settings::get_bool(&conn, SYNC_CONNECTED, false),
        last_sync: settings::get_setting(&conn, SYNC_LAST_AT),
        queued,
        hub_url: settings::get_setting(&conn, HUB_URL).unwrap_or_default(),
        paired: !settings::get_setting(&conn, HUB_TOKEN)
            .unwrap_or_default()
            .is_empty(),
    })
}

#[tauri::command]
pub fn set_app_mode(db: State<'_, Db>, mode: String) -> Result<(), String> {
    let mode = if mode == "hub" { "hub" } else { "local" };
    let conn = db.lock().map_err(|e| e.to_string())?;
    settings::set_setting(&conn, APP_MODE, mode).map_err(|e| e.to_string())
}

/// Pair this device with a hub: exchanges the pairing secret for a device token.
#[tauri::command]
pub fn pair_with_hub(
    db: State<'_, Db>,
    hub_url: String,
    pairing_secret: String,
) -> Result<(), String> {
    let url = normalize_hub_url(&hub_url)?;
    let resp = ureq::post(&format!("{url}/api/pair"))
        .timeout(Duration::from_secs(15))
        .send_json(serde_json::json!({
            "pairingSecret": pairing_secret,
            "name": hostname(),
            "platform": std::env::consts::OS,
        }))
        .map_err(|e| format!("pairing failed: {e}"))?;
    let body: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
    let token = body
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or("hub did not return a token")?;
    let device_id = body.get("deviceId").and_then(|v| v.as_str()).unwrap_or("");

    let conn = db.lock().map_err(|e| e.to_string())?;
    settings::set_setting(&conn, HUB_URL, &url).map_err(|e| e.to_string())?;
    settings::set_setting(&conn, HUB_TOKEN, token).map_err(|e| e.to_string())?;
    settings::set_setting(&conn, HUB_DEVICE_ID, device_id).map_err(|e| e.to_string())?;
    settings::set_setting(&conn, APP_MODE, "hub").map_err(|e| e.to_string())?;
    settings::set_setting(&conn, CLASSIFICATION_SNAP_KEY, "").map_err(|e| e.to_string())?;
    Ok(())
}

fn reset_sync_state(conn: &Connection, include_history: bool) {
    if include_history {
        for table in [
            "activity_log",
            "browser_activity",
            "output_events",
            "focus_sessions",
        ] {
            set_watermark(conn, table, 0);
        }
    }
    for key in [
        "sync_snap_checkin",
        "sync_snap_checkin_defs",
        "sync_snap_note",
        "sync_snap_goals",
        CLASSIFICATION_SNAP_KEY,
    ] {
        let _ = settings::set_setting(conn, key, "");
    }
}

/// Re-send all local history to the hub once (resets the upload watermarks).
#[tauri::command]
pub fn import_history_to_hub(db: State<'_, Db>) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    reset_sync_state(&conn, true);
    Ok(())
}

/// Force projects, categories and matching rules to the Hub immediately.
#[tauri::command]
pub fn sync_configuration_now(db: State<'_, Db>) -> Result<i64, String> {
    let (url, token, device_id) = {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let target =
            sync_target(&conn).ok_or("Connect and pair this desktop with Tempo Hub first.")?;
        reset_sync_state(&conn, false);
        let day = chrono::Local::now().format("%Y-%m-%d").to_string();
        sync_classification_bundle(&conn, &day, chrono::Utc::now().timestamp_millis());
        target
    };

    let sent = drain_once(db.inner(), &url, &token, &device_id)?;
    let conn = db.lock().map_err(|e| e.to_string())?;
    settings::set_setting(&conn, SYNC_CONNECTED, "1").map_err(|e| e.to_string())?;
    settings::set_setting(&conn, SYNC_LAST_AT, &chrono::Utc::now().to_rfc3339())
        .map_err(|e| e.to_string())?;
    Ok(sent)
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "desktop".into())
}

// ===================================================================== tests
#[cfg(test)]
mod tests {
    use super::*;
    use tempo_core::db;

    fn seed_activity(conn: &Connection, n: i64) {
        for i in 1..=n {
            conn.execute(
                "INSERT INTO activity_log (timestamp, day, app_name, window_title, duration_seconds, is_idle)
                 VALUES ('2026-01-01T12:00:00+00:00', '2026-01-01', 'Code', 'x', 600, 0)",
                [],
            )
            .unwrap();
            let _ = i;
        }
    }

    #[test]
    fn hub_urls_are_normalized_and_limited_to_http() {
        assert_eq!(
            normalize_hub_url("tempo-hub.example.ts.net").unwrap(),
            "https://tempo-hub.example.ts.net"
        );
        assert_eq!(
            normalize_hub_url("http://100.90.80.70:7700/").unwrap(),
            "http://100.90.80.70:7700"
        );
        assert!(normalize_hub_url("ftp://tempo-hub").is_err());
        assert!(normalize_hub_url("https://user:secret@tempo-hub.example.ts.net").is_err());
        assert!(normalize_hub_url("https://tempo-hub.example.ts.net?token=nope").is_err());
        assert!(normalize_hub_url("https://tempo-hub.example.ts.net/api/health").is_err());
    }
    #[test]
    fn configuration_ack_requires_current_hub() {
        assert!(
            validate_configuration_ack(&serde_json::json!({"classificationVersion": 2})).is_ok()
        );
        assert!(
            validate_configuration_ack(&serde_json::json!({"classificationVersion": 1})).is_err()
        );
        assert!(validate_configuration_ack(&serde_json::json!({})).is_err());
    }

    #[test]
    fn local_mode_has_no_sync_target() {
        let conn = db::test_conn();
        // default: no app_mode → None
        assert!(sync_target(&conn).is_none());
        settings::set_setting(&conn, APP_MODE, "local").unwrap();
        assert!(sync_target(&conn).is_none());
        // hub mode but unconfigured → still None
        settings::set_setting(&conn, APP_MODE, "hub").unwrap();
        assert!(sync_target(&conn).is_none());
        // fully configured → Some
        settings::set_setting(&conn, HUB_URL, "http://pi:7700").unwrap();
        settings::set_setting(&conn, HUB_TOKEN, "tok").unwrap();
        assert!(sync_target(&conn).is_some());
    }

    /// First-ever scan registers the seeded check-in definitions as events;
    /// later scans are quiet until something changes.
    fn prime(conn: &Connection) {
        assert!(scan_and_enqueue(conn) > 0); // the seeded check-in definitions
        assert_eq!(scan_and_enqueue(conn), 0);
    }

    #[test]
    fn classification_bundle_is_acknowledged_only_after_delivery() {
        let conn = db::test_conn();
        let day = "2026-07-30";

        assert_eq!(sync_classification_bundle(&conn, day, 1), 1);
        assert!(settings::get_setting(&conn, CLASSIFICATION_SNAP_KEY).is_none());
        assert_eq!(sync_classification_bundle(&conn, day, 2), 0);

        let items = pending(&conn);
        assert_eq!(items.len(), 1);
        acknowledge_classification_bundles(&conn, &items);
        mark_sent(&conn, &[items[0].0.clone()]);

        let expected = classification_bundle(&conn).to_string();
        assert_eq!(
            settings::get_setting(&conn, CLASSIFICATION_SNAP_KEY).as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(sync_classification_bundle(&conn, day, 3), 0);

        reset_sync_state(&conn, false);
        assert_eq!(sync_classification_bundle(&conn, day, 4), 1);
    }

    #[test]
    fn scan_enqueues_then_advances_watermark() {
        let conn = db::test_conn();
        prime(&conn);
        seed_activity(&conn, 3);
        assert_eq!(scan_and_enqueue(&conn), 3);
        let q: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_queue WHERE status='pending' AND event_id LIKE 'activity_log:%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(q, 3);
        // second scan: watermark advanced → nothing new
        assert_eq!(scan_and_enqueue(&conn), 0);
    }

    #[test]
    fn offline_keeps_queue_then_success_drains() {
        let conn = db::test_conn();
        prime(&conn);
        conn.execute("DELETE FROM sync_queue", []).unwrap(); // drop the def events
        seed_activity(&conn, 2);
        scan_and_enqueue(&conn);
        let ids: Vec<String> = pending(&conn).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids.len(), 3); // two samples plus the unacknowledged config bundle

        // Simulate a failed upload: queue stays pending, error logged.
        record_failure(&conn, &ids, "connection refused");
        let pend: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_queue WHERE status='pending'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pend, 3);
        let errs: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_errors", [], |r| r.get(0))
            .unwrap();
        assert_eq!(errs, 1);
        let attempts: i64 = conn
            .query_row(
                "SELECT attempts FROM sync_queue WHERE event_id='activity_log:1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(attempts, 1);

        // Now the hub comes back: a successful drain marks them sent.
        mark_sent(&conn, &ids);
        let pend2: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_queue WHERE status='pending'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pend2, 0);
    }

    #[test]
    fn focus_scan_skips_active_then_uploads_terminal_once() {
        let conn = db::test_conn();
        prime(&conn);
        // An active session must NOT be uploaded (it would freeze on the hub as active).
        conn.execute(
            "INSERT INTO focus_sessions (id, goal, started_at, duration_minutes, ends_at, allowed, blocked, status)
             VALUES (1, 'g', '2026-01-01T12:00:00+00:00', 50, '2026-01-01T12:50:00+00:00', '[]', '[]', 'active')",
            [],
        )
        .unwrap();
        assert_eq!(scan_and_enqueue(&conn), 0);

        // Once it ends it becomes terminal and uploads exactly once.
        conn.execute(
            "UPDATE focus_sessions SET status='ended', ended_at='2026-01-01T12:50:00+00:00' WHERE id=1",
            [],
        )
        .unwrap();
        assert_eq!(scan_and_enqueue(&conn), 1);
        let q: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_queue WHERE event_id='focus_sessions:1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(q, 1);
        // Watermark advanced → no re-upload.
        assert_eq!(scan_and_enqueue(&conn), 0);
    }

    #[test]
    fn checkins_note_goals_sync_on_change_only() {
        let conn = db::test_conn();
        let day = chrono::Local::now().format("%Y-%m-%d").to_string();

        // First scan registers the seeded definitions, then everything is quiet.
        prime(&conn);

        // Log a check-in (+ note) and a goal on the desktop.
        tempo_core::models::set_checkin_value(&conn, &day, "gym_logged", 1).unwrap();
        conn.execute(
            "INSERT INTO daily_checkin (day, notes) VALUES (?1, 'leg day')",
            params![day],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO goals (day, title, priority, completed, sort_order, recurring, created_at)
             VALUES (?1, 'Ship v2', 'high', 0, 1, 0, 't')",
            params![day],
        )
        .unwrap();

        assert!(scan_and_enqueue(&conn) >= 3); // gym check-in + note + goal
        let count = |like: &str| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM sync_queue WHERE event_id LIKE ?1",
                params![like],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(count("checkin:%:gym_logged:%"), 1);
        assert_eq!(count("note:%"), 1);
        assert_eq!(count("goal:%"), 1);

        // Unchanged → nothing new.
        assert_eq!(scan_and_enqueue(&conn), 0);

        // Completing the goal re-sends it (so completion reaches the hub).
        conn.execute(
            "UPDATE goals SET completed = 1 WHERE day = ?1 AND title = 'Ship v2'",
            params![day],
        )
        .unwrap();
        assert!(scan_and_enqueue(&conn) >= 1);
    }

    #[test]
    fn checkin_definitions_and_cleared_overrides_sync() {
        let conn = db::test_conn();
        let day = chrono::Local::now().format("%Y-%m-%d").to_string();
        prime(&conn);

        // Creating an auto check-in syncs its full definition (no value event needed).
        tempo_core::models::upsert_checkin_definition(
            &conn,
            &tempo_core::models::CheckinDefinition {
                id: "read_bible".into(),
                label: "Read Bible".into(),
                icon: "📖".into(),
                kind: "toggle".into(),
                built_in: false,
                auto_kind: "target".into(),
                auto_metric: "bible".into(),
                auto_threshold: 30,
            },
        )
        .unwrap();
        assert_eq!(scan_and_enqueue(&conn), 1);
        let payload: String = conn
            .query_row(
                "SELECT payload_json FROM sync_queue WHERE event_id LIKE 'checkin_def:read_bible:%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(payload.contains("\"autoKind\":\"target\""));
        assert!(payload.contains("\"autoThreshold\":30"));

        // Manually overriding it syncs a value event; clearing the override syncs a clear.
        tempo_core::models::set_checkin_value(&conn, &day, "read_bible", 0).unwrap();
        assert_eq!(scan_and_enqueue(&conn), 1);
        tempo_core::models::clear_checkin_value(&conn, &day, "read_bible").unwrap();
        assert_eq!(scan_and_enqueue(&conn), 1);
        let cleared: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_queue WHERE event_id LIKE 'checkin:%:read_bible:clear:%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cleared, 1);

        // Deleting the definition syncs a deletion marker.
        tempo_core::models::delete_checkin_definition(&conn, "read_bible").unwrap();
        assert_eq!(scan_and_enqueue(&conn), 1);
        let del: String = conn
            .query_row(
                "SELECT payload_json FROM sync_queue WHERE event_id LIKE 'checkin_def:read_bible:del:%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(del.contains("\"deleted\":true"));
    }
}
