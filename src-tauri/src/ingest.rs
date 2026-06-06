//! Turns an incoming extension payload into a stored `browser_activity` row,
//! re-applying capture/retention policy defensively (never trust the client).

use rusqlite::params;

use crate::db::Db;
use crate::models::IngestPayload;
use crate::settings;

pub fn handle_ingest(db: &Db, p: IngestPayload) -> Result<i64, String> {
    let domain = p.domain.trim().to_ascii_lowercase();
    if domain.is_empty() {
        return Err("missing domain".into());
    }

    let conn = db.lock().map_err(|e| e.to_string())?;

    let capture_master = settings::get_bool(&conn, settings::CAPTURE_PAGE_CONTENT, false);
    let store_raw = settings::get_bool(&conn, settings::STORE_RAW_TEXT, false);
    let delete_raw_after = settings::get_bool(&conn, settings::DELETE_RAW_AFTER, false);
    let max_len = settings::get_int(&conn, settings::MAX_TEXT_LENGTH, settings::DEFAULT_MAX_TEXT_LENGTH)
        .clamp(0, 200_000) as usize;

    let mode = settings::effective_capture_mode(&conn, &domain);
    let blocked = mode == "never";
    // Master toggle is the final gate: with capture OFF, never store content,
    // even if a client sends it.
    let allow_text = capture_master && !blocked && mode == "text";

    // For blocked/sensitive domains, strip the URL down to its origin so we
    // never persist a path/query that could carry sensitive tokens.
    let url = if blocked { origin_of(&p.url) } else { p.url.clone() };

    // Content retention: summary/keywords kept only for text mode; raw kept only
    // if the user explicitly opted in AND isn't auto-deleting after classify.
    let mut summary = if allow_text { p.content_summary.clone() } else { None };
    let mut keywords: Vec<String> = if allow_text {
        p.detected_keywords.clone().unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut raw = if allow_text && store_raw && !delete_raw_after {
        p.raw_text_excerpt.clone()
    } else {
        None
    };

    if let Some(r) = raw.as_mut() {
        truncate_chars(r, max_len);
    }
    if let Some(s) = summary.as_mut() {
        truncate_chars(s, 1200);
    }
    keywords.truncate(24);

    let content_capture_enabled = summary.is_some() || raw.is_some();
    let keywords_json = serde_json::to_string(&keywords).unwrap_or_else(|_| "[]".to_string());

    let ts = p
        .timestamp
        .clone()
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let day = local_day(&ts);
    let duration = p.duration_seconds.unwrap_or(settings::SAMPLE_SECONDS).clamp(0, 3600);
    let is_idle = p.is_idle.unwrap_or(false) as i64;

    conn.execute(
        "INSERT INTO browser_activity
            (timestamp, day, domain, url, page_title, duration_seconds,
             content_capture_enabled, content_type, raw_text_excerpt,
             content_summary, detected_keywords, is_idle)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            ts,
            day,
            domain,
            url,
            p.page_title,
            duration,
            content_capture_enabled as i64,
            p.content_type,
            raw,
            summary,
            keywords_json,
            is_idle,
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(conn.last_insert_rowid())
}

fn origin_of(url: &str) -> String {
    if let Some(scheme_end) = url.find("://") {
        let after = &url[scheme_end + 3..];
        let host_end = after.find('/').unwrap_or(after.len());
        return format!("{}{}", &url[..scheme_end + 3], &after[..host_end]);
    }
    url.to_string()
}

fn truncate_chars(s: &mut String, max: usize) {
    if s.chars().count() > max {
        *s = s.chars().take(max).collect();
    }
}

fn local_day(ts: &str) -> String {
    use chrono::{DateTime, Local};
    match DateTime::parse_from_rfc3339(ts) {
        Ok(dt) => dt.with_timezone(&Local).format("%Y-%m-%d").to_string(),
        Err(_) => Local::now().format("%Y-%m-%d").to_string(),
    }
}
