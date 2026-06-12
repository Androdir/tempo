//! Generic synced-event model shared by the Tempo Hub (ingest) and the desktop
//! sync client (upload). One source of truth so client and server agree on the
//! wire format and how an event maps into the domain tables.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// A single activity/output/check-in event in transit between a device and the
/// hub. `event_id` is stable per device (e.g. "activity_log:123") so re-uploads
/// dedupe on `(device_id, event_id)`.
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SyncEvent {
    pub event_id: String,
    pub event_type: String, // app_sample | browser_sample | output | checkin | goal | note
    #[serde(default)]
    pub source: Option<String>,
    pub timestamp: String,
    pub day: String,
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub duration_seconds: Option<i64>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventBatch {
    pub device_id: String,
    pub events: Vec<SyncEvent>,
}

impl SyncEvent {
    fn m_str(&self, key: &str) -> Option<String> {
        self.metadata.as_ref()?.get(key)?.as_str().map(|s| s.to_string())
    }
    fn m_i64(&self, key: &str) -> Option<i64> {
        self.metadata.as_ref()?.get(key)?.as_i64()
    }
    fn m_bool(&self, key: &str) -> bool {
        self.metadata
            .as_ref()
            .and_then(|m| m.get(key))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }
}

/// Store one event on the hub: append to the dedup ledger, and only if it is
/// genuinely new, project it into the matching domain table. Returns whether the
/// event was newly stored (false = duplicate, ignored).
pub fn ingest_event(conn: &Connection, device_id: &str, e: &SyncEvent) -> rusqlite::Result<bool> {
    let meta = e.metadata.as_ref().map(|m| m.to_string());
    let changed = conn.execute(
        "INSERT OR IGNORE INTO synced_events
           (device_id, event_id, event_type, source, timestamp, day, app_name, domain, title,
            duration_seconds, category, project, metadata_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            device_id,
            e.event_id,
            e.event_type,
            e.source,
            e.timestamp,
            e.day,
            e.app_name,
            e.domain,
            e.title,
            e.duration_seconds,
            e.category,
            e.project,
            meta,
            chrono::Utc::now().to_rfc3339(),
        ],
    )?;
    if changed == 0 {
        return Ok(false); // duplicate upload — never double-counted
    }
    apply_to_domain(conn, e)?;
    Ok(true)
}

/// Best-effort registration of a check-in definition carried in a synced event's
/// metadata, so check-ins created on one device exist on the hub before their
/// values land. Existing definitions are left untouched (the hub's edit wins).
fn register_checkin_from_event(conn: &Connection, e: &SyncEvent, id: &str) {
    if crate::models::checkin_exists(conn, id) {
        return;
    }
    let label = e
        .m_str("label")
        .filter(|l| !l.trim().is_empty())
        .unwrap_or_else(|| id.replace(['_', '-'], " "));
    let icon = e.m_str("icon").filter(|i| !i.trim().is_empty()).unwrap_or_else(|| "✅".into());
    let kind = match e.m_str("kind").as_deref() {
        Some("counter") => "counter",
        _ => "toggle",
    };
    let _ = crate::models::upsert_checkin_definition(
        conn,
        &crate::models::CheckinDefinition {
            id: id.to_string(),
            label,
            icon,
            kind: kind.to_string(),
            built_in: false,
            auto_kind: String::new(),
            auto_metric: String::new(),
            auto_threshold: 0,
        },
    );
}

/// Project a freshly-stored event into the same domain tables the desktop writes
/// to, so all existing aggregation (dashboard/timeline/score/streaks) works on
/// the hub with no changes. Multi-device just sums across rows.
fn apply_to_domain(conn: &Connection, e: &SyncEvent) -> rusqlite::Result<()> {
    match e.event_type.as_str() {
        "app_sample" => {
            conn.execute(
                "INSERT INTO activity_log (timestamp, day, app_name, window_title, duration_seconds, is_idle)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    e.timestamp,
                    e.day,
                    e.app_name.clone().unwrap_or_default(),
                    e.title.clone().unwrap_or_default(),
                    e.duration_seconds.unwrap_or(0),
                    e.m_bool("isIdle") as i64,
                ],
            )?;
        }
        "browser_sample" => {
            conn.execute(
                "INSERT INTO browser_activity
                   (timestamp, day, domain, url, page_title, duration_seconds,
                    content_capture_enabled, content_type, content_summary, detected_keywords, is_idle)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, ?9, ?10)",
                params![
                    e.timestamp,
                    e.day,
                    e.domain.clone().unwrap_or_default(),
                    e.m_str("url").unwrap_or_default(),
                    e.title.clone().unwrap_or_default(),
                    e.duration_seconds.unwrap_or(0),
                    e.m_str("contentType"),
                    e.m_str("summary"),
                    e.m_str("keywords"),
                    e.m_bool("isIdle") as i64,
                ],
            )?;
        }
        "output" => {
            // Idempotent on (file_path, modified_at) like the local watcher.
            conn.execute(
                "INSERT OR IGNORE INTO output_events
                   (timestamp, day, folder_path, file_path, file_name, extension, file_size,
                    event_type, project, modified_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    e.timestamp,
                    e.day,
                    e.m_str("folderPath").unwrap_or_default(),
                    e.m_str("filePath").unwrap_or_else(|| e.title.clone().unwrap_or_default()),
                    e.m_str("fileName").unwrap_or_else(|| e.title.clone().unwrap_or_default()),
                    e.m_str("extension"),
                    e.m_i64("fileSize").unwrap_or(0),
                    e.category.clone().unwrap_or_else(|| "other".into()),
                    e.project,
                    e.m_str("modifiedAt"),
                    e.m_str("createdAt"),
                ],
            )?;
        }
        "checkin" => {
            if let Some(field) = e.m_str("field") {
                let value = e.m_i64("value").unwrap_or(0);
                if field == "main_goal_completed" {
                    conn.execute("INSERT OR IGNORE INTO daily_checkin (day) VALUES (?1)", params![e.day])?;
                    conn.execute(
                        "UPDATE daily_checkin SET main_goal_completed = ?1 WHERE day = ?2",
                        params![(value != 0) as i64, e.day],
                    )?;
                } else if e.m_bool("cleared") {
                    // A manual override was removed on the source device → fall
                    // back to auto detection here too.
                    let _ = crate::models::clear_checkin_value(conn, &e.day, &field);
                } else {
                    register_checkin_from_event(conn, e, &field);
                    let _ = crate::models::set_checkin_value(conn, &e.day, &field, value);
                }
            }
        }
        "checkin_def" => {
            // Full check-in definition sync (create/edit/delete). The desktop is
            // the editor of record, so its copy overwrites the hub's.
            if let Some(id) = e.m_str("id") {
                if e.m_bool("deleted") {
                    let _ = crate::models::delete_checkin_definition(conn, &id);
                } else {
                    let def = crate::models::CheckinDefinition {
                        id: id.clone(),
                        label: e.m_str("label").unwrap_or_else(|| id.replace(['_', '-'], " ")),
                        icon: e.m_str("icon").unwrap_or_else(|| "✅".into()),
                        kind: match e.m_str("kind").as_deref() {
                            Some("counter") => "counter".into(),
                            _ => "toggle".into(),
                        },
                        built_in: false,
                        auto_kind: e.m_str("autoKind").unwrap_or_default(),
                        auto_metric: e.m_str("autoMetric").unwrap_or_default(),
                        auto_threshold: e.m_i64("autoThreshold").unwrap_or(0),
                    };
                    let _ = crate::models::upsert_checkin_definition(conn, &def);
                }
            }
        }
        "note" => {
            let notes = e.m_str("notes").or_else(|| e.title.clone()).unwrap_or_default();
            conn.execute("INSERT OR IGNORE INTO daily_checkin (day) VALUES (?1)", params![e.day])?;
            conn.execute("UPDATE daily_checkin SET notes = ?1 WHERE day = ?2", params![notes, e.day])?;
        }
        "focus" => {
            // A focus session, synced once it ends. The hub assigns its own row id;
            // the summary is recomputed at view time from every device's activity in
            // the window, so this just needs the session's bounds + allow/block lists.
            let started = e.m_str("startedAt").unwrap_or_else(|| e.timestamp.clone());
            let ends = e.m_str("endsAt").unwrap_or_default();
            let dur = e.m_i64("durationMinutes").unwrap_or(0);
            let allowed = e.m_str("allowed").unwrap_or_else(|| "[]".into());
            let blocked = e.m_str("blocked").unwrap_or_else(|| "[]".into());
            let status = e.m_str("status").unwrap_or_else(|| "completed".into());
            let ended = e.m_str("endedAt");
            let goal = e.m_str("goal").or_else(|| e.title.clone());
            conn.execute(
                "INSERT INTO focus_sessions
                   (goal, started_at, duration_minutes, ends_at, allowed, blocked, status, ended_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![goal, started, dur, ends, allowed, blocked, status, ended],
            )?;
        }
        "goal" => {
            if let Some(title) = e.title.clone().filter(|t| !t.trim().is_empty()) {
                let completed = e.m_bool("completed") as i64;
                let priority = e.m_str("priority").unwrap_or_else(|| "medium".into());
                let target = e.m_i64("targetMinutes");
                // Upsert by (day, title) so completion/edits from another device sync,
                // not just first-time creation.
                let existing: Option<i64> = conn
                    .query_row(
                        "SELECT id FROM goals WHERE day = ?1 AND title = ?2 LIMIT 1",
                        params![e.day, title],
                        |r| r.get(0),
                    )
                    .ok();
                if let Some(id) = existing {
                    conn.execute(
                        "UPDATE goals SET completed = ?1, priority = ?2, target_minutes = ?3 WHERE id = ?4",
                        params![completed, priority, target, id],
                    )?;
                } else {
                    let order: i64 = conn
                        .query_row(
                            "SELECT COALESCE(MAX(sort_order),0)+1 FROM goals WHERE day = ?1",
                            params![e.day],
                            |r| r.get(0),
                        )
                        .unwrap_or(0);
                    conn.execute(
                        "INSERT INTO goals (day, title, project, target_minutes, priority, completed, sort_order, recurring, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)",
                        params![e.day, title, e.project, target, priority, completed, order, e.timestamp],
                    )?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn ev(event_id: &str, etype: &str) -> SyncEvent {
        SyncEvent {
            event_id: event_id.into(),
            event_type: etype.into(),
            source: Some("desktop".into()),
            timestamp: "2026-01-01T12:00:00+00:00".into(),
            day: "2026-01-01".into(),
            app_name: Some("Code".into()),
            domain: None,
            title: Some("main.rs".into()),
            duration_seconds: Some(600),
            category: None,
            project: None,
            metadata: None,
        }
    }

    #[test]
    fn dedup_ignores_repeated_event() {
        let conn = db::test_conn();
        let e = ev("activity_log:1", "app_sample");
        assert!(ingest_event(&conn, "deviceA", &e).unwrap()); // new
        assert!(!ingest_event(&conn, "deviceA", &e).unwrap()); // duplicate
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM activity_log", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1); // never double-counted
    }

    #[test]
    fn same_event_id_distinct_devices_both_stored() {
        let conn = db::test_conn();
        let e = ev("activity_log:1", "app_sample");
        assert!(ingest_event(&conn, "deviceA", &e).unwrap());
        assert!(ingest_event(&conn, "deviceB", &e).unwrap());
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM activity_log", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2); // different devices, same local id → both real
    }

    #[test]
    fn focus_event_projects_and_summary_is_cross_device_overlap_aware() {
        let conn = db::test_conn();
        conn.execute(
            "INSERT INTO category_rules (app_name, category, updated_at) VALUES ('code','productive','t')",
            [],
        )
        .unwrap();

        // A focus session synced from the desktop once it ended (instagram blocked).
        let mut fe = ev("focus_sessions:1", "focus");
        fe.title = Some("ship".into());
        fe.metadata = Some(serde_json::json!({
            "goal": "ship",
            "startedAt": "2026-01-01T12:00:00+00:00",
            "endsAt": "2026-01-01T13:00:00+00:00",
            "durationMinutes": 60,
            "allowed": "[]",
            "blocked": "[\"instagram.com\"]",
            "status": "ended",
            "endedAt": "2026-01-01T13:00:00+00:00",
        }));
        assert!(ingest_event(&conn, "desktop", &fe).unwrap());
        assert!(!ingest_event(&conn, "desktop", &fe).unwrap()); // dedup
        let sessions: i64 =
            conn.query_row("SELECT COUNT(*) FROM focus_sessions", [], |r| r.get(0)).unwrap();
        assert_eq!(sessions, 1);
        let fid: i64 = conn.query_row("SELECT id FROM focus_sessions LIMIT 1", [], |r| r.get(0)).unwrap();

        // Desktop coded 12:10–12:40 (focused); phone scrolled instagram 12:30–12:40.
        let mut code = ev("activity_log:1", "app_sample");
        code.app_name = Some("code".into());
        code.timestamp = "2026-01-01T12:10:00+00:00".into();
        code.duration_seconds = Some(1800);
        code.metadata = Some(serde_json::json!({ "isIdle": false }));
        assert!(ingest_event(&conn, "desktop", &code).unwrap());

        let mut insta = ev("browser_activity:1", "browser_sample");
        insta.app_name = None;
        insta.domain = Some("instagram.com".into());
        insta.title = Some("reels".into());
        insta.timestamp = "2026-01-01T12:30:00+00:00".into();
        insta.duration_seconds = Some(600);
        insta.metadata = Some(serde_json::json!({ "isIdle": false }));
        assert!(ingest_event(&conn, "phone", &insta).unwrap());

        let s = crate::aggregate::focus_summary(&conn, fid).unwrap();
        // The 10 min both devices were "active" is counted once, as the distraction.
        assert_eq!(s.focused_seconds, 1200);
        assert_eq!(s.distracted_seconds, 600);
        assert_eq!(s.adherence, 66); // 1200 / 1800
        assert_eq!(s.top_distraction.as_deref(), Some("instagram.com"));
    }

    #[test]
    fn goal_event_upserts_completion() {
        let conn = db::test_conn();

        // First a goal is created (not done).
        let mut e = ev("goal:2026-01-01:Ship", "goal");
        e.title = Some("Ship".into());
        e.metadata = Some(serde_json::json!({ "completed": false, "priority": "high" }));
        assert!(ingest_event(&conn, "desktop", &e).unwrap());
        let done: i64 = conn.query_row("SELECT completed FROM goals WHERE title='Ship'", [], |r| r.get(0)).unwrap();
        assert_eq!(done, 0);

        // A later event (different id) marking it done updates the *same* row.
        let mut e2 = ev("goal:2026-01-01:Ship:done", "goal");
        e2.title = Some("Ship".into());
        e2.metadata = Some(serde_json::json!({ "completed": true, "priority": "high" }));
        assert!(ingest_event(&conn, "desktop", &e2).unwrap());
        let rows: i64 = conn.query_row("SELECT COUNT(*) FROM goals WHERE title='Ship'", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1); // upserted, not duplicated
        let done2: i64 = conn.query_row("SELECT completed FROM goals WHERE title='Ship'", [], |r| r.get(0)).unwrap();
        assert_eq!(done2, 1);
    }
}
