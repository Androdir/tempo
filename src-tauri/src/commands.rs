use std::collections::HashMap;

use chrono::{Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use tauri::State;

use crate::aggregate::*;
use crate::classify;
use crate::db::Db;
use crate::llm;
use crate::models::*;
use crate::projects::{self, Project};
use crate::rules;
use crate::scoring;
use crate::settings;

/// Today's aggregated activity — desktop apps merged with browser websites.
#[tauri::command]
pub fn get_today_summary(db: State<'_, Db>) -> Result<TodaySummary, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    summary_for_day(&conn, &today())
}

#[tauri::command]
pub fn get_tracked_apps(db: State<'_, Db>) -> Result<Vec<TrackedApp>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT al.app_name, COALESCE(SUM(al.duration_seconds), 0) AS secs,
                    cr.category, COALESCE(cr.ai_review, 0)
             FROM activity_log al
             LEFT JOIN category_rules cr ON cr.app_name = al.app_name
             GROUP BY al.app_name ORDER BY secs DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(TrackedApp {
                app_name: r.get(0)?,
                total_seconds: r.get(1)?,
                category: r.get(2)?,
                ai_review: r.get::<_, i64>(3)? != 0,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn get_category_rules(db: State<'_, Db>) -> Result<Vec<CategoryRule>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT app_name, category FROM category_rules ORDER BY app_name")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(CategoryRule {
                app_name: r.get(0)?,
                category: r.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn get_category_definitions(db: State<'_, Db>) -> Result<Vec<CategoryDefinition>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    crate::models::list_category_definitions(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn upsert_category_definition(
    db: State<'_, Db>,
    category: CategoryDefinition,
) -> Result<(), String> {
    let id = category.id.trim().to_ascii_lowercase();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
        return Err("Category id must use lowercase letters, numbers, dashes or underscores".into());
    }
    if category.label.trim().is_empty() {
        return Err("Category label is required".into());
    }
    if !["productive", "neutral", "distracting"].contains(&category.bucket.as_str()) {
        return Err("Bucket must be productive, neutral or distracting".into());
    }
    let conn = db.lock().map_err(|e| e.to_string())?;
    crate::models::upsert_category_definition(
        &conn,
        &CategoryDefinition {
            id,
            label: category.label.trim().to_string(),
            color: category.color.trim().to_string(),
            bucket: category.bucket,
            blurb: category.blurb.trim().to_string(),
            built_in: category.built_in,
        },
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_category_definition(db: State<'_, Db>, id: String) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    crate::models::delete_category_definition(&conn, &id.trim().to_ascii_lowercase())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_category_rule(
    db: State<'_, Db>,
    app_name: String,
    category: String,
    ai_review: bool,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    if !crate::models::category_exists(&conn, &category) {
        return Err(format!("Unknown category: {category}"));
    }
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO category_rules (app_name, category, ai_review, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(app_name) DO UPDATE SET
             category = excluded.category,
             ai_review = excluded.ai_review,
             updated_at = excluded.updated_at",
        params![app_name, category, ai_review as i64, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn delete_category_rule(db: State<'_, Db>, app_name: String) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM category_rules WHERE app_name = ?1",
        params![app_name],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// --------------------------------------------------------- browser activity

#[tauri::command]
pub fn get_browser_activity(db: State<'_, Db>) -> Result<BrowserActivityView, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let day = today();
    let domain_rules = domain_category_map(&conn)?;
    let projects = load_projects(&conn)?;

    // Per-domain aggregate (active).
    let browser_rows = query_browser_rows(&conn, &day)?;
    let mut domain_secs: HashMap<String, (i64, i64)> = HashMap::new();
    for row in &browser_rows {
        let entry = domain_secs.entry(row.domain.clone()).or_insert((0, 0));
        entry.0 += row.duration;
        entry.1 += 1;
    }
    let mut per_domain: Vec<WebsiteUsage> = domain_secs
        .into_iter()
        .map(|(domain, (secs, views))| {
            let category = domain_rules.get(&domain).cloned().flatten();
            WebsiteUsage { domain, seconds: secs, category, page_views: views }
        })
        .collect();
    per_domain.sort_by(|a, b| b.seconds.cmp(&a.seconds).then_with(|| a.domain.cmp(&b.domain)));

    // Recent page rows (most recent first), classified live.
    let mut stmt = conn
        .prepare(
            "SELECT id, timestamp, domain, url, page_title, content_type,
                    content_summary, detected_keywords, duration_seconds,
                    raw_text_excerpt IS NOT NULL, is_idle
             FROM browser_activity WHERE day = ?1 ORDER BY id DESC LIMIT 100",
        )
        .map_err(|e| e.to_string())?;
    let raw_rows = stmt
        .query_map([&day], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, i64>(8)?,
                r.get::<_, bool>(9)?,
                r.get::<_, bool>(10)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut recent_pages = Vec::new();
    for row in raw_rows {
        let (id, ts, domain, url, title, ctype, summary, kw, dur, has_raw, is_idle) =
            row.map_err(|e| e.to_string())?;
        let keywords = parse_keywords(kw);
        let dcat = domain_rules.get(&domain).cloned().flatten();
        let base = classify::classify(
            &domain,
            &title,
            ctype.as_deref(),
            summary.as_deref(),
            &keywords,
            dcat.as_deref(),
        );
        let extra = browser_extra(summary.as_deref(), &keywords);
        let (category, _reason, pm) =
            projects::resolve(&projects, &domain, &title, &extra, &base.category, &base.reason);
        let project_name = pm.as_ref().map(|m| m.project_name.clone());
        let project_confidence = pm.as_ref().map(|m| m.confidence).unwrap_or(0);
        let project_signals = pm.map(|m| m.signals).unwrap_or_default();
        recent_pages.push(BrowserPage {
            id,
            timestamp: ts,
            domain,
            url,
            page_title: title,
            duration_seconds: dur,
            content_type: ctype,
            category,
            content_summary: summary,
            detected_keywords: keywords,
            has_raw,
            is_idle,
            project_name,
            project_confidence,
            project_signals,
        });
    }

    Ok(BrowserActivityView {
        date: day,
        per_domain,
        recent_pages,
    })
}

#[tauri::command]
pub fn get_activity_details(db: State<'_, Db>, id: i64) -> Result<ActivityDetail, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let domain_rules = domain_category_map(&conn)?;

    let row = conn
        .query_row(
            "SELECT id, timestamp, domain, url, page_title, duration_seconds,
                    content_type, content_summary, detected_keywords,
                    raw_text_excerpt, content_capture_enabled, is_idle
             FROM browser_activity WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, Option<String>>(9)?,
                    r.get::<_, bool>(10)?,
                    r.get::<_, bool>(11)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;

    let app_rules = load_rules(&conn)?;
    let app_ai = load_app_ai(&conn)?;
    let domain_ai = load_domain_ai(&conn)?;
    let projects = load_projects(&conn)?;
    let llm_cache = llm::load_today_cache(&conn);
    let manual = load_manual_corrections(&conn, &today())?;

    let (id, ts, domain, url, title, dur, ctype, summary, kw, raw, cap_enabled, is_idle) = row;
    let keywords = parse_keywords(kw);

    let inputs = rules::RuleInputs {
        app_rules: &app_rules,
        app_ai: &app_ai,
        domain_cat: &domain_rules,
        domain_ai: &domain_ai,
        projects: &projects,
    };
    let v = rules::classify_block(
        &inputs,
        "web",
        &domain,
        &title,
        Some(&domain),
        ctype.as_deref(),
        summary.as_deref(),
        &keywords,
    );
    let key = block_key(&today(), "web", &domain, &title);
    let rule_project = v.project.clone();
    let project_confidence = v.project_confidence;
    let project_signals = v.project_signals.clone();

    // Priority: manual correction > LLM (if rules wanted review) > rules.
    let (category, classification_reason, project_name, classifier, llm_confidence) =
        if let Some(cat) = manual.get(&key) {
            (cat.clone(), "manual correction".to_string(), rule_project, "manual".to_string(), None)
        } else if v.needs_llm {
            match llm_cache.get(&key) {
                Some(c) => {
                    let proj = c.project.clone().or(rule_project);
                    let r = if c.reason.is_empty() {
                        format!("LLM → {}", c.category)
                    } else {
                        c.reason.clone()
                    };
                    (c.category.clone(), r, proj, "llm".to_string(), Some(c.confidence))
                }
                None => (v.category.clone(), v.reason.clone(), rule_project, "rule".to_string(), None),
            }
        } else {
            (v.category.clone(), v.reason.clone(), rule_project, "rule".to_string(), None)
        };
    let category = category_or_fallback(&conn, &category);

    Ok(ActivityDetail {
        id,
        timestamp: ts,
        domain,
        url,
        page_title: title,
        duration_seconds: dur,
        content_type: ctype,
        category,
        classification_reason,
        content_summary: summary,
        detected_keywords: keywords,
        raw_text_excerpt: raw,
        content_capture_enabled: cap_enabled,
        is_idle,
        project_name,
        project_confidence,
        project_signals,
        classifier,
        llm_confidence,
        confidence: v.confidence,
        block_key: key,
    })
}

// ------------------------------------------------------------- domain rules

#[tauri::command]
pub fn get_domain_rules(db: State<'_, Db>) -> Result<Vec<DomainRule>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    settings::list_domain_rules(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_tracked_domains(db: State<'_, Db>) -> Result<Vec<TrackedDomain>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT ba.domain, COALESCE(SUM(ba.duration_seconds), 0) AS secs,
                    dr.category, COALESCE(dr.capture_mode, 'meta'), COALESCE(dr.ai_review, 0)
             FROM browser_activity ba
             LEFT JOIN domain_rules dr ON dr.domain = ba.domain
             GROUP BY ba.domain ORDER BY secs DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(TrackedDomain {
                domain: r.get(0)?,
                total_seconds: r.get(1)?,
                category: r.get(2)?,
                capture_mode: r.get(3)?,
                ai_review: r.get::<_, i64>(4)? != 0,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn set_domain_rule(
    db: State<'_, Db>,
    domain: String,
    category: Option<String>,
    capture_mode: String,
    ai_review: bool,
) -> Result<(), String> {
    let domain = domain.trim().to_ascii_lowercase();
    if domain.is_empty() {
        return Err("Domain is required".into());
    }
    if !["text", "meta", "never"].contains(&capture_mode.as_str()) {
        return Err(format!("Invalid capture mode: {capture_mode}"));
    }
    let conn = db.lock().map_err(|e| e.to_string())?;
    if let Some(c) = category.as_deref() {
        if !c.is_empty() && !crate::models::category_exists(&conn, c) {
            return Err(format!("Unknown category: {c}"));
        }
    }
    let category = category.filter(|c| !c.is_empty());
    settings::upsert_domain_rule(&conn, &domain, category.as_deref(), &capture_mode, ai_review)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_domain_rule(db: State<'_, Db>, domain: String) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    settings::delete_domain_rule(&conn, &domain.trim().to_ascii_lowercase())
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------- privacy settings

#[tauri::command]
pub fn get_privacy_settings(db: State<'_, Db>) -> Result<PrivacySettings, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let port = settings::ingest_port(&conn);
    Ok(PrivacySettings {
        capture_page_content: settings::get_bool(&conn, settings::CAPTURE_PAGE_CONTENT, false),
        store_raw_text: settings::get_bool(&conn, settings::STORE_RAW_TEXT, false),
        max_text_length: settings::get_int(&conn, settings::MAX_TEXT_LENGTH, settings::DEFAULT_MAX_TEXT_LENGTH),
        delete_raw_after_classification: settings::get_bool(&conn, settings::DELETE_RAW_AFTER, false),
        smart_tracking_enabled: settings::get_bool(&conn, settings::SMART_TRACKING_ENABLED, false),
        smart_interval_seconds: settings::get_int(&conn, settings::SMART_INTERVAL, settings::DEFAULT_SMART_INTERVAL),
        smart_ocr_available: cfg!(windows),
        ingest_port: port,
        ingest_token: settings::ingest_token(&conn),
        endpoint: format!("http://127.0.0.1:{port}"),
        retention_days: settings::get_int(&conn, settings::RETENTION_DAYS, settings::DEFAULT_RETENTION_DAYS),
        idle_threshold_seconds: settings::get_int(&conn, settings::IDLE_THRESHOLD, settings::IDLE_SECONDS),
        count_media_as_active: settings::get_bool(&conn, settings::COUNT_MEDIA_ACTIVE, true),
    })
}

#[tauri::command]
pub fn set_privacy_setting(db: State<'_, Db>, key: String, value: String) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    match key.as_str() {
        settings::CAPTURE_PAGE_CONTENT
        | settings::STORE_RAW_TEXT
        | settings::DELETE_RAW_AFTER
        | settings::SMART_TRACKING_ENABLED
        | settings::COUNT_MEDIA_ACTIVE => {
            let v = if value == "1" || value.eq_ignore_ascii_case("true") { "1" } else { "0" };
            settings::set_setting(&conn, &key, v).map_err(|e| e.to_string())
        }
        settings::MAX_TEXT_LENGTH => {
            let n = value.parse::<i64>().map_err(|_| "max length must be a number".to_string())?;
            settings::set_setting(&conn, &key, &n.clamp(500, 100_000).to_string()).map_err(|e| e.to_string())
        }
        settings::SMART_INTERVAL => {
            let n = value.parse::<i64>().map_err(|_| "interval must be a number".to_string())?;
            settings::set_setting(&conn, &key, &n.clamp(10, 3600).to_string()).map_err(|e| e.to_string())
        }
        settings::RETENTION_DAYS => {
            let n = value.parse::<i64>().map_err(|_| "retention must be a number".to_string())?;
            settings::set_setting(&conn, &key, &n.clamp(0, 3650).to_string()).map_err(|e| e.to_string())
        }
        settings::IDLE_THRESHOLD => {
            let n = value.parse::<i64>().map_err(|_| "idle threshold must be a number".to_string())?;
            settings::set_setting(&conn, &key, &n.clamp(20, 1800).to_string()).map_err(|e| e.to_string())
        }
        _ => Err(format!("Unknown setting: {key}")),
    }
}

/// Manually enforce the retention policy now; returns rows removed.
#[tauri::command]
pub fn prune_old_data(db: State<'_, Db>) -> Result<i64, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let days = settings::get_int(&conn, settings::RETENTION_DAYS, settings::DEFAULT_RETENTION_DAYS);
    crate::db::prune(&conn, days).map(|n| n as i64).map_err(|e| e.to_string())
}

/// Reset all user/application data while preserving the schema.
#[tauri::command]
pub fn reset_database(db: State<'_, Db>) -> Result<i64, String> {
    let mut conn = db.lock().map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let deletes = [
        "DELETE FROM sync_errors",
        "DELETE FROM sync_queue",
        "DELETE FROM devices",
        "DELETE FROM synced_events",
        "DELETE FROM lockin_plans",
        "DELETE FROM streak_definitions",
        "DELETE FROM output_events",
        "DELETE FROM watched_folders",
        "DELETE FROM daily_review",
        "DELETE FROM focus_sessions",
        "DELETE FROM goals",
        "DELETE FROM daily_checkin",
        "DELETE FROM manual_corrections",
        "DELETE FROM projects",
        "DELETE FROM smart_activity",
        "DELETE FROM llm_errors",
        "DELETE FROM llm_classification",
        "DELETE FROM app_settings",
        "DELETE FROM domain_rules",
        "DELETE FROM browser_activity",
        "DELETE FROM category_rules",
        "DELETE FROM category_definitions",
        "DELETE FROM activity_log",
    ];
    let mut removed = 0i64;
    for sql in deletes {
        removed += tx.execute(sql, []).map_err(|e| e.to_string())? as i64;
    }
    let _ = tx.execute("DELETE FROM sqlite_sequence", []);
    tx.commit().map_err(|e| e.to_string())?;

    settings::ensure_defaults(&conn).map_err(|e| e.to_string())?;
    crate::models::ensure_category_defaults(&conn).map_err(|e| e.to_string())?;
    Ok(removed)
}

#[tauri::command]
pub fn purge_raw_content(db: State<'_, Db>) -> Result<i64, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let n = conn
        .execute(
            "UPDATE browser_activity SET raw_text_excerpt = NULL WHERE raw_text_excerpt IS NOT NULL",
            [],
        )
        .map_err(|e| e.to_string())?;
    Ok(n as i64)
}

#[tauri::command]
pub fn delete_all_captured_content(db: State<'_, Db>) -> Result<i64, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let n1 = conn
        .execute(
            "UPDATE browser_activity
             SET raw_text_excerpt = NULL, content_summary = NULL,
                 detected_keywords = NULL, content_capture_enabled = 0",
            [],
        )
        .map_err(|e| e.to_string())?;
    // Screen-OCR rows are entirely captured content — remove them.
    let n2 = conn
        .execute("DELETE FROM smart_activity", [])
        .map_err(|e| e.to_string())?;
    Ok((n1 + n2) as i64)
}

// --------------------------------------------------------------- local LLM

#[tauri::command]
pub fn get_llm_settings(db: State<'_, Db>) -> Result<LlmSettings, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    Ok(LlmSettings {
        enabled: settings::get_bool(&conn, settings::LLM_ENABLED, false),
        url: settings::get_setting(&conn, settings::OLLAMA_URL)
            .unwrap_or_else(|| settings::DEFAULT_OLLAMA_URL.to_string()),
        model: settings::get_setting(&conn, settings::OLLAMA_MODEL)
            .unwrap_or_else(|| settings::DEFAULT_OLLAMA_MODEL.to_string()),
        last_error: llm::last_error(&conn),
    })
}

#[tauri::command]
pub fn set_llm_setting(db: State<'_, Db>, key: String, value: String) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    match key.as_str() {
        settings::LLM_ENABLED => {
            let v = if value == "1" || value.eq_ignore_ascii_case("true") { "1" } else { "0" };
            settings::set_setting(&conn, &key, v).map_err(|e| e.to_string())
        }
        settings::OLLAMA_URL => {
            let v = value.trim();
            if v.is_empty() {
                return Err("URL is required".into());
            }
            settings::set_setting(&conn, settings::OLLAMA_URL, v).map_err(|e| e.to_string())
        }
        settings::OLLAMA_MODEL => {
            let v = value.trim();
            if v.is_empty() {
                return Err("Model is required".into());
            }
            settings::set_setting(&conn, settings::OLLAMA_MODEL, v).map_err(|e| e.to_string())
        }
        _ => Err(format!("Unknown setting: {key}")),
    }
}

/// Test the Ollama connection with the given (possibly unsaved) URL + model.
#[tauri::command]
pub fn test_ollama_connection(url: String, model: String) -> OllamaTestResult {
    llm::test_connection(url.trim(), model.trim())
}

#[tauri::command]
pub fn get_llm_errors(db: State<'_, Db>) -> Result<Vec<LlmError>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    Ok(llm::recent_errors(&conn))
}

// --------------------------------------------------------- projects / goals

fn validate_project(conn: &Connection, p: &Project) -> Result<(), String> {
    if p.name.trim().is_empty() {
        return Err("Project name is required".into());
    }
    if !crate::models::category_exists(conn, &p.category) {
        return Err(format!("Unknown category: {}", p.category));
    }
    Ok(())
}

#[tauri::command]
pub fn get_projects(db: State<'_, Db>) -> Result<Vec<Project>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    load_projects(&conn)
}

#[tauri::command]
pub fn create_project(db: State<'_, Db>, project: Project) -> Result<i64, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    validate_project(&conn, &project)?;
    projects::create_project(&conn, &project).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_project(db: State<'_, Db>, project: Project) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    validate_project(&conn, &project)?;
    if project.id <= 0 {
        return Err("Missing project id".into());
    }
    projects::update_project(&conn, &project).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_project(db: State<'_, Db>, id: i64) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    projects::delete_project(&conn, id).map_err(|e| e.to_string())
}

/// Unified recent activity. Prefers the cached LLM verdict, else rule-based.
#[tauri::command]
pub fn get_recent_activity(db: State<'_, Db>) -> Result<Vec<ActivityLogEntry>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let day = today();
    let llm_cache = llm::load_today_cache(&conn);
    let manual = load_manual_corrections(&conn, &day)?;
    let mut blocks = collect_blocks(&conn)?;
    blocks.truncate(120);

    let entries = blocks
        .into_iter()
        .map(|b| {
            // Priority: manual correction > LLM (only if rules wanted review) > rules.
            let (category, reason, project_name, classifier, llm_confidence) =
                if let Some(cat) = manual.get(&b.block_key) {
                    (
                        cat.clone(),
                        "manual correction".to_string(),
                        b.rule_project.clone(),
                        "manual".to_string(),
                        None,
                    )
                } else if b.needs_llm {
                    match llm_cache.get(&b.block_key) {
                        Some(c) => {
                            let proj = c.project.clone().or_else(|| b.rule_project.clone());
                            let reason = if c.reason.is_empty() {
                                format!("LLM → {}", c.category)
                            } else {
                                c.reason.clone()
                            };
                            (c.category.clone(), reason, proj, "llm".to_string(), Some(c.confidence))
                        }
                        None => (
                            b.rule_category.clone(),
                            b.rule_reason.clone(),
                            b.rule_project.clone(),
                            "rule".to_string(),
                            None,
                        ),
                    }
                } else {
                    (
                        b.rule_category.clone(),
                        b.rule_reason.clone(),
                        b.rule_project.clone(),
                        "rule".to_string(),
                        None,
                    )
                };
            let category = category_or_fallback(&conn, &category);
            ActivityLogEntry {
                source: b.source,
                label: b.label,
                title: b.title,
                seconds: b.seconds,
                category,
                reason,
                content_type: b.content_type,
                last_seen: b.last_seen,
                detail_id: b.detail_id,
                summary: b.summary,
                project_name,
                project_confidence: b.rule_project_confidence,
                project_signals: b.rule_signals,
                classifier,
                llm_confidence,
                confidence: b.confidence,
                block_key: b.block_key,
            }
        })
        .collect();

    Ok(entries)
}

/// Manual correction of an activity block (Step 3). Saves the correction,
/// creates/updates a reusable app/domain rule, and clears any stale LLM cache.
#[tauri::command]
pub fn correct_activity(
    db: State<'_, Db>,
    block_key: String,
    source: String,
    label: String,
    title: String,
    category: String,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    if !crate::models::category_exists(&conn, &category) && category != "ignore" {
        return Err(format!("Unknown category: {category}"));
    }
    let now = Utc::now().to_rfc3339();
    let day = today();

    conn.execute(
        "INSERT INTO manual_corrections (block_key, day, source, label, title, category, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(block_key) DO UPDATE SET
             category = excluded.category, created_at = excluded.created_at",
        params![block_key, day, source, label, title, category, now],
    )
    .map_err(|e| e.to_string())?;

    // Turn the correction into a reusable rule so future blocks (and the
    // dashboard) follow it. "ignore" is per-block only — no rule.
    if crate::models::category_exists(&conn, &category) {
        if source == "web" {
            conn.execute(
                "INSERT INTO domain_rules (domain, category, capture_mode, ai_review, updated_at)
                 VALUES (?1, ?2, 'meta', 0, ?3)
                 ON CONFLICT(domain) DO UPDATE SET
                     category = excluded.category, updated_at = excluded.updated_at",
                params![label.trim().to_ascii_lowercase(), category, now],
            )
            .map_err(|e| e.to_string())?;
        } else {
            conn.execute(
                "INSERT INTO category_rules (app_name, category, ai_review, updated_at)
                 VALUES (?1, ?2, 0, ?3)
                 ON CONFLICT(app_name) DO UPDATE SET
                     category = excluded.category, updated_at = excluded.updated_at",
                params![label, category, now],
            )
            .map_err(|e| e.to_string())?;
        }
    }

    // The manual verdict wins, so drop any cached LLM verdict for this block.
    let _ = conn.execute(
        "DELETE FROM llm_classification WHERE block_key = ?1",
        params![block_key],
    );

    Ok(())
}

// --------------------------------------------------- proof-of-work timeline

/// The proof-of-work timeline for a day: raw samples merged into continuous,
/// classified blocks, with the day's highlights flagged.
#[tauri::command]
pub fn get_timeline_for_day(
    db: State<'_, Db>,
    day: String,
    max_gap_seconds: Option<i64>,
) -> Result<TimelineDay, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    timeline_for_day(&conn, &day, max_gap_seconds.unwrap_or(120).clamp(20, 3600))
}

// --------------------------------------------------- proof-of-output detection

#[tauri::command]
pub fn get_output_events(db: State<'_, Db>, day: String) -> Result<Vec<OutputEvent>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, timestamp, day, folder_path, file_path, file_name, extension, file_size,
                    event_type, project, linked_block_key, linked_label, created_at, modified_at
             FROM output_events WHERE day = ?1 ORDER BY COALESCE(modified_at, timestamp) DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([&day], row_to_output).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn get_watched_folders(db: State<'_, Db>) -> Result<Vec<WatchedFolder>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    Ok(crate::output::load_folders(&conn))
}

fn valid_output_type(t: &str) -> bool {
    matches!(
        t,
        "video_export" | "code_change" | "document_created" | "download" | "study_material" | "other"
    )
}

fn clean_exts(exts: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in exts {
        let t = e.trim().trim_start_matches('.').to_ascii_lowercase();
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

#[tauri::command]
pub fn add_watched_folder(db: State<'_, Db>, folder: WatchedFolder) -> Result<i64, String> {
    let path = folder.path.trim().to_string();
    if path.is_empty() {
        return Err("Folder path is required".into());
    }
    if !std::path::Path::new(&path).is_dir() {
        return Err("That folder doesn't exist on this machine".into());
    }
    let otype = if valid_output_type(&folder.output_type) { folder.output_type.clone() } else { "other".into() };
    let label = if folder.label.trim().is_empty() {
        std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Folder".into())
    } else {
        folder.label.trim().to_string()
    };
    let exts = serde_json::to_string(&clean_exts(&folder.extensions)).unwrap_or_else(|_| "[]".into());
    let project = folder.project.filter(|p| !p.trim().is_empty());
    let conn = db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO watched_folders
           (path, label, project, output_type, enabled, extensions, min_size_bytes, debounce_seconds, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            path,
            label,
            project,
            otype,
            folder.enabled as i64,
            exts,
            folder.min_size_bytes.max(0),
            folder.debounce_seconds.clamp(0, 3600),
            Utc::now().to_rfc3339(),
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

#[tauri::command]
pub fn update_watched_folder(db: State<'_, Db>, folder: WatchedFolder) -> Result<(), String> {
    if folder.id <= 0 {
        return Err("missing folder id".into());
    }
    let otype = if valid_output_type(&folder.output_type) { folder.output_type.clone() } else { "other".into() };
    let exts = serde_json::to_string(&clean_exts(&folder.extensions)).unwrap_or_else(|_| "[]".into());
    let project = folder.project.filter(|p| !p.trim().is_empty());
    let conn = db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE watched_folders SET label = ?1, project = ?2, output_type = ?3, enabled = ?4,
            extensions = ?5, min_size_bytes = ?6, debounce_seconds = ?7 WHERE id = ?8",
        params![
            folder.label.trim(),
            project,
            otype,
            folder.enabled as i64,
            exts,
            folder.min_size_bytes.max(0),
            folder.debounce_seconds.clamp(0, 3600),
            folder.id,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn remove_watched_folder(db: State<'_, Db>, id: i64) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM watched_folders WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn link_output_events_to_blocks(db: State<'_, Db>, day: String) -> Result<i64, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    link_outputs_for_day(&conn, &day)
}

#[tauri::command]
pub fn scan_outputs_now(db: State<'_, Db>) -> Result<i64, String> {
    Ok(crate::output::scan_once(db.inner()))
}

// ------------------------------------------------------------------- streaks

#[tauri::command]
pub fn get_streaks(db: State<'_, Db>) -> Result<Vec<Streak>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    compute_streaks(&conn)
}

#[tauri::command]
pub fn get_streak_definitions(db: State<'_, Db>) -> Result<Vec<StreakDefinition>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    Ok(crate::streaks::load_defs(&conn)
        .into_iter()
        .map(|d| StreakDefinition {
            id: d.id,
            name: d.name,
            kind: d.kind,
            threshold: d.threshold,
            enabled: d.enabled,
        })
        .collect())
}

#[tauri::command]
pub fn update_streak_definition(
    db: State<'_, Db>,
    id: String,
    enabled: Option<bool>,
    threshold: Option<i64>,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let now = Utc::now().to_rfc3339();
    if let Some(en) = enabled {
        conn.execute(
            "UPDATE streak_definitions SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
            params![en as i64, now, id],
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(th) = threshold {
        conn.execute(
            "UPDATE streak_definitions SET threshold = ?1, updated_at = ?2 WHERE id = ?3",
            params![th.clamp(0, 100_000), now, id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ----------------------------------------------------------- daily lock-in plan

#[tauri::command]
pub fn generate_lockin_plan(db: State<'_, Db>, day: String) -> Result<LockinPlan, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    Ok(generate_plan_core(&conn, &day))
}

#[tauri::command]
pub fn get_lockin_plan(db: State<'_, Db>, day: String) -> Result<Option<LockinPlan>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    Ok(load_plan(&conn, &day))
}

#[tauri::command]
pub fn save_lockin_plan(db: State<'_, Db>, day: String, plan: LockinPlan) -> Result<(), String> {
    let mut p = plan;
    p.day = day;
    p.source = "manual".to_string();
    p.edited = true;
    let conn = db.lock().map_err(|e| e.to_string())?;
    store_plan(&conn, &p)
}

#[tauri::command]
pub fn copy_lockin_plan_to_goals(db: State<'_, Db>, day: String) -> Result<i64, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    copy_plan_core(&conn, &day)
}

#[tauri::command]
pub fn get_lockin_auto(db: State<'_, Db>) -> Result<bool, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    Ok(settings::get_bool(&conn, settings::LOCKIN_AUTO_ENABLED, true))
}

#[tauri::command]
pub fn set_lockin_auto(db: State<'_, Db>, enabled: bool) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    settings::set_setting(&conn, settings::LOCKIN_AUTO_ENABLED, if enabled { "1" } else { "0" })
        .map_err(|e| e.to_string())
}

// ------------------------------------------------------------- daily score

#[tauri::command]
pub fn get_daily_score(db: State<'_, Db>) -> Result<ScoreReport, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let day = today();
    let stats = compute_stats(&conn)?;
    let checkins = effective_checkins(&conn, &day);
    let goal = top_project_name(&conn);
    let outputs = output_signals_for_day(&conn, &day);
    Ok(scoring::build_report(&conn, day, &stats, &checkins, &outputs, goal))
}

#[tauri::command]
pub fn set_checkin(db: State<'_, Db>, field: String, value: i64) -> Result<(), String> {
    const FIELDS: [&str; 7] = [
        "main_goal_completed",
        "videos_posted",
        "gym_logged",
        "wrestled",
        "studied",
        "edited_video",
        "analysed_content",
    ];
    if !FIELDS.contains(&field.as_str()) {
        return Err(format!("unknown check-in: {field}"));
    }
    let v = if field == "videos_posted" { value.clamp(0, 99) } else { (value != 0) as i64 };
    let conn = db.lock().map_err(|e| e.to_string())?;
    let day = today();
    conn.execute("INSERT OR IGNORE INTO daily_checkin (day) VALUES (?1)", params![day])
        .map_err(|e| e.to_string())?;
    // `field` is whitelisted above, so this interpolation is safe.
    let sql = format!("UPDATE daily_checkin SET {field} = ?1 WHERE day = ?2");
    conn.execute(&sql, params![v, day]).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_checkins(db: State<'_, Db>) -> Result<CheckinState, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    Ok(read_checkin_state(&conn, &today()))
}

// --------------------------------------------------------------- daily goals

fn normalize_priority(p: &str) -> String {
    match p.trim().to_ascii_lowercase().as_str() {
        "low" => "low".to_string(),
        "high" => "high".to_string(),
        _ => "medium".to_string(),
    }
}

fn row_to_goal(r: &rusqlite::Row) -> rusqlite::Result<Goal> {
    Ok(Goal {
        id: r.get(0)?,
        title: r.get(1)?,
        project: r.get(2)?,
        target_minutes: r.get(3)?,
        priority: r.get(4)?,
        completed: r.get::<_, i64>(5)? != 0,
        recurring: r.get::<_, i64>(6)? != 0,
    })
}

/// Once per day, copy recurring goals from the most recent prior day into today
/// (skipping titles already present), so daily habits reappear automatically.
fn ensure_recurring_goals(conn: &Connection, day: &str) {
    if settings::get_setting(conn, settings::RECURRING_MATERIALIZED_DAY).as_deref() == Some(day) {
        return;
    }
    let src: Option<String> = conn
        .query_row("SELECT MAX(day) FROM goals WHERE recurring = 1 AND day < ?1", [day], |r| {
            r.get::<_, Option<String>>(0)
        })
        .ok()
        .flatten();
    if let Some(src_day) = src {
        let mut templates: Vec<(String, Option<String>, Option<i64>, String)> = Vec::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT title, project, target_minutes, priority FROM goals WHERE day = ?1 AND recurring = 1",
        ) {
            if let Ok(rows) = stmt.query_map([&src_day], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, String>(3)?,
                ))
            }) {
                templates = rows.filter_map(Result::ok).collect();
            }
        }
        let mut order = next_sort_order(conn, day);
        for (title, project, target, priority) in templates {
            if goal_exists(conn, day, &title) {
                continue;
            }
            let _ = conn.execute(
                "INSERT INTO goals (day, title, project, target_minutes, priority, completed, sort_order, recurring, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, 1, ?7)",
                params![day, title, project, target, priority, order, Utc::now().to_rfc3339()],
            );
            order += 1;
        }
    }
    let _ = settings::set_setting(conn, settings::RECURRING_MATERIALIZED_DAY, day);
}

#[tauri::command]
pub fn get_goals(db: State<'_, Db>) -> Result<Vec<Goal>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let day = today();
    ensure_recurring_goals(&conn, &day);
    let mut stmt = conn
        .prepare(
            "SELECT id, title, project, target_minutes, priority, completed, recurring
             FROM goals WHERE day = ?1
             ORDER BY CASE priority WHEN 'high' THEN 0 WHEN 'medium' THEN 1 ELSE 2 END,
                      sort_order, id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([&day], row_to_goal).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for g in rows {
        out.push(g.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn add_goal(db: State<'_, Db>, goal: Goal) -> Result<i64, String> {
    let title = goal.title.trim();
    if title.is_empty() {
        return Err("Goal title is required".into());
    }
    let priority = normalize_priority(&goal.priority);
    let project = goal.project.filter(|p| !p.trim().is_empty());
    let target = goal.target_minutes.filter(|m| *m > 0);
    let conn = db.lock().map_err(|e| e.to_string())?;
    let day = today();
    let order = next_sort_order(&conn, &day);
    conn.execute(
        "INSERT INTO goals (day, title, project, target_minutes, priority, completed, sort_order, recurring, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8)",
        params![day, title, project, target, priority, order, goal.recurring as i64, Utc::now().to_rfc3339()],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

#[tauri::command]
pub fn update_goal(db: State<'_, Db>, goal: Goal) -> Result<(), String> {
    if goal.id <= 0 {
        return Err("missing goal id".into());
    }
    let title = goal.title.trim();
    if title.is_empty() {
        return Err("Goal title is required".into());
    }
    let priority = normalize_priority(&goal.priority);
    let project = goal.project.filter(|p| !p.trim().is_empty());
    let target = goal.target_minutes.filter(|m| *m > 0);
    let conn = db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE goals SET title = ?1, project = ?2, target_minutes = ?3, priority = ?4,
                          completed = ?5, recurring = ?6 WHERE id = ?7",
        params![title, project, target, priority, goal.completed as i64, goal.recurring as i64, goal.id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn toggle_goal(db: State<'_, Db>, id: i64, completed: bool) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    conn.execute("UPDATE goals SET completed = ?1 WHERE id = ?2", params![completed as i64, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn delete_goal(db: State<'_, Db>, id: i64) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM goals WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn set_goal_recurring(db: State<'_, Db>, id: i64, recurring: bool) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    conn.execute("UPDATE goals SET recurring = ?1 WHERE id = ?2", params![recurring as i64, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Copy the most recent prior day's goals into today (uncompleted), skipping
/// titles already present. Returns how many were copied.
#[tauri::command]
pub fn copy_previous_goals(db: State<'_, Db>) -> Result<i64, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let day = today();
    let src: Option<String> = conn
        .query_row("SELECT MAX(day) FROM goals WHERE day < ?1", [&day], |r| {
            r.get::<_, Option<String>>(0)
        })
        .map_err(|e| e.to_string())?;
    let Some(src_day) = src else {
        return Ok(0);
    };

    let mut templates: Vec<(String, Option<String>, Option<i64>, String, i64)> = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT title, project, target_minutes, priority, recurring FROM goals
                 WHERE day = ?1 ORDER BY sort_order, id",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([&src_day], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            templates.push(row.map_err(|e| e.to_string())?);
        }
    }

    let mut order = next_sort_order(&conn, &day);
    let mut count = 0i64;
    for (title, project, target, priority, recurring) in templates {
        if goal_exists(&conn, &day, &title) {
            continue;
        }
        conn.execute(
            "INSERT INTO goals (day, title, project, target_minutes, priority, completed, sort_order, recurring, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8)",
            params![day, title, project, target, priority, order, recurring, Utc::now().to_rfc3339()],
        )
        .map_err(|e| e.to_string())?;
        order += 1;
        count += 1;
    }
    Ok(count)
}

#[tauri::command]
pub fn set_scoring_weight(db: State<'_, Db>, id: String, weight: i64) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    scoring::set_weight(&conn, &id, weight)
}

#[tauri::command]
pub fn set_scoring_threshold(db: State<'_, Db>, id: String, threshold: i64) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    scoring::set_threshold(&conn, &id, threshold)
}

#[tauri::command]
pub fn reset_scoring_weights(db: State<'_, Db>) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    scoring::reset(&conn)
}

// ----------------------------------------------------------- review + demo

#[tauri::command]
pub fn get_daily_review(db: State<'_, Db>) -> Result<DailyAiReview, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    daily_review(&conn)
}

#[tauri::command]
pub fn generate_daily_review(db: State<'_, Db>) -> Result<DailyAiReview, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    generate_review(&conn)
}

#[tauri::command]
pub fn set_daily_note(db: State<'_, Db>, notes: String) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let day = today();
    conn.execute("INSERT OR IGNORE INTO daily_checkin (day) VALUES (?1)", params![day])
        .map_err(|e| e.to_string())?;
    conn.execute("UPDATE daily_checkin SET notes = ?1 WHERE day = ?2", params![notes, day])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ============================================================ accountability

fn clean_list_vec(items: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for it in items {
        let t = it.trim().to_string();
        if !t.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
            out.push(t);
        }
    }
    out
}

/// Mark any active session whose window has elapsed as completed.
fn complete_expired_focus(conn: &Connection) {
    let _ = conn.execute(
        "UPDATE focus_sessions SET status='completed', ended_at=ends_at
         WHERE status='active' AND ends_at <= ?1",
        params![Utc::now().to_rfc3339()],
    );
}

#[tauri::command]
pub fn start_focus_session(
    db: State<'_, Db>,
    goal: Option<String>,
    duration_minutes: i64,
    allowed: Vec<String>,
    blocked: Vec<String>,
) -> Result<FocusSession, String> {
    let dur = duration_minutes.clamp(1, 600);
    let conn = db.lock().map_err(|e| e.to_string())?;
    // Only one active session at a time — end any prior one.
    conn.execute(
        "UPDATE focus_sessions SET status='ended', ended_at=?1 WHERE status='active'",
        params![Utc::now().to_rfc3339()],
    )
    .map_err(|e| e.to_string())?;
    let now = Utc::now();
    let ends = now + Duration::minutes(dur);
    let goal = goal.filter(|g| !g.trim().is_empty());
    let allowed_json = serde_json::to_string(&clean_list_vec(allowed)).unwrap_or_else(|_| "[]".into());
    let blocked_json = serde_json::to_string(&clean_list_vec(blocked)).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "INSERT INTO focus_sessions
           (goal, started_at, duration_minutes, ends_at, allowed, blocked, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active')",
        params![goal, now.to_rfc3339(), dur, ends.to_rfc3339(), allowed_json, blocked_json],
    )
    .map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();
    conn.query_row(&format!("SELECT {FOCUS_COLS} FROM focus_sessions WHERE id = ?1"), [id], row_to_focus)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_focus_session(db: State<'_, Db>) -> Result<Option<FocusSession>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    complete_expired_focus(&conn);
    conn.query_row(
        &format!("SELECT {FOCUS_COLS} FROM focus_sessions WHERE status='active' ORDER BY id DESC LIMIT 1"),
        [],
        row_to_focus,
    )
    .optional()
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn end_focus_session(db: State<'_, Db>) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE focus_sessions SET status='ended', ended_at=?1 WHERE status='active'",
        params![Utc::now().to_rfc3339()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_focus_summary(db: State<'_, Db>, id: i64) -> Result<FocusSummary, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    focus_summary(&conn, id)
}


// ------------------------------------------------------ accountability settings

#[tauri::command]
pub fn get_accountability_settings(db: State<'_, Db>) -> Result<AccountabilitySettings, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    Ok(AccountabilitySettings {
        distraction_warn_enabled: settings::get_bool(&conn, settings::DISTRACTION_WARN_ENABLED, true),
        distraction_warn_minutes: settings::get_int(
            &conn,
            settings::DISTRACTION_WARN_MINUTES,
            settings::DEFAULT_DISTRACTION_MINUTES,
        ),
        eod_popup_enabled: settings::get_bool(&conn, settings::EOD_POPUP_ENABLED, false),
        eod_popup_time: settings::get_setting(&conn, settings::EOD_POPUP_TIME)
            .unwrap_or_else(|| settings::DEFAULT_EOD_TIME.to_string()),
    })
}

fn valid_hhmm(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    parts.len() == 2
        && parts[0].len() == 2
        && parts[1].len() == 2
        && parts[0].parse::<u32>().map(|h| h < 24).unwrap_or(false)
        && parts[1].parse::<u32>().map(|m| m < 60).unwrap_or(false)
}

#[tauri::command]
pub fn set_accountability_setting(db: State<'_, Db>, key: String, value: String) -> Result<(), String> {
    let allowed = [
        settings::DISTRACTION_WARN_ENABLED,
        settings::DISTRACTION_WARN_MINUTES,
        settings::EOD_POPUP_ENABLED,
        settings::EOD_POPUP_TIME,
    ];
    if !allowed.contains(&key.as_str()) {
        return Err(format!("unknown setting: {key}"));
    }
    let value = if key == settings::DISTRACTION_WARN_MINUTES {
        let n: i64 = value.trim().parse().map_err(|_| "minutes must be a number".to_string())?;
        n.clamp(1, 240).to_string()
    } else if key == settings::EOD_POPUP_TIME {
        if !valid_hhmm(value.trim()) {
            return Err("time must be HH:MM (24-hour)".into());
        }
        value.trim().to_string()
    } else if value == "true" || value == "1" {
        "1".to_string()
    } else {
        "0".to_string()
    };
    let conn = db.lock().map_err(|e| e.to_string())?;
    settings::set_setting(&conn, &key, &value).map_err(|e| e.to_string())
}

/// Quiet all distraction warnings for `minutes` ("Snooze" toast action).
#[tauri::command]
pub fn set_distraction_snooze(db: State<'_, Db>, minutes: i64) -> Result<(), String> {
    let until = (Utc::now() + Duration::minutes(minutes.clamp(1, 1440))).to_rfc3339();
    let conn = db.lock().map_err(|e| e.to_string())?;
    settings::set_setting(&conn, settings::DISTRACTION_SNOOZE_UNTIL, &until).map_err(|e| e.to_string())
}

/// Quiet warnings for one app/site for `minutes` ("It's intentional" action).
#[tauri::command]
pub fn set_distraction_intentional(
    db: State<'_, Db>,
    target: String,
    minutes: i64,
) -> Result<(), String> {
    let until = (Utc::now() + Duration::minutes(minutes.clamp(1, 1440))).to_rfc3339();
    let conn = db.lock().map_err(|e| e.to_string())?;
    settings::set_setting(&conn, settings::DISTRACTION_MUTE_TARGET, target.trim())
        .map_err(|e| e.to_string())?;
    settings::set_setting(&conn, settings::DISTRACTION_MUTE_UNTIL, &until).map_err(|e| e.to_string())
}

// --------------------------------------------------------------- weekly review

#[tauri::command]
pub fn get_weekly_review(db: State<'_, Db>) -> Result<WeeklyReview, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    weekly_review(&conn)
}

// ================================================================ tests
// Integration tests that run the real aggregation logic against an in-memory
// SQLite database (not the mock layer), covering day boundaries, idle filtering,
// category bucketing, goal-derived check-ins, retention, focus windowing, the
// weekly roll-up, and recurring-goal materialization.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use chrono::Local;
    use rusqlite::params;

    fn ins_app(conn: &Connection, day: &str, app: &str, title: &str, secs: i64, idle: bool) {
        conn.execute(
            "INSERT INTO activity_log (timestamp, day, app_name, window_title, duration_seconds, is_idle)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![format!("{day}T12:00:00+00:00"), day, app, title, secs, idle as i64],
        )
        .unwrap();
    }
    fn ins_app_ts(conn: &Connection, ts: &str, app: &str, secs: i64) {
        conn.execute(
            "INSERT INTO activity_log (timestamp, day, app_name, window_title, duration_seconds, is_idle)
             VALUES (?1, ?2, ?3, 'w', ?4, 0)",
            params![ts, &ts[..10], app, secs],
        )
        .unwrap();
    }
    fn ins_web(conn: &Connection, day: &str, domain: &str, title: &str, secs: i64) {
        conn.execute(
            "INSERT INTO browser_activity (timestamp, day, domain, url, page_title, duration_seconds, is_idle)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            params![format!("{day}T12:00:00+00:00"), day, domain, format!("https://{domain}"), title, secs],
        )
        .unwrap();
    }
    fn ins_web_ts(conn: &Connection, ts: &str, domain: &str, secs: i64) {
        conn.execute(
            "INSERT INTO browser_activity (timestamp, day, domain, url, page_title, duration_seconds, is_idle)
             VALUES (?1, ?2, ?3, ?4, 't', ?5, 0)",
            params![ts, &ts[..10], domain, format!("https://{domain}"), secs],
        )
        .unwrap();
    }
    fn set_app_rule(conn: &Connection, app: &str, cat: &str) {
        conn.execute(
            "INSERT INTO category_rules (app_name, category, updated_at) VALUES (?1, ?2, 't')",
            params![app, cat],
        )
        .unwrap();
    }
    fn local_day(offset: i64) -> String {
        (Local::now().date_naive() - Duration::days(offset)).format("%Y-%m-%d").to_string()
    }
    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn collect_blocks_filters_day_and_idle() {
        let conn = db::test_conn();
        ins_app(&conn, "2026-01-01", "code", "main.rs", 300, false);
        ins_app(&conn, "2026-01-01", "idleapp", "x", 999, true); // idle excluded
        ins_app(&conn, "2026-01-02", "other", "y", 100, false); // other day excluded
        let blocks = collect_blocks_for_day(&conn, "2026-01-01").unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].label, "code");
        assert_eq!(blocks[0].seconds, 300);
    }

    #[test]
    fn compute_stats_buckets_categories() {
        let conn = db::test_conn();
        crate::settings::ensure_defaults(&conn).unwrap();
        set_app_rule(&conn, "code", "productive");
        ins_app(&conn, "2026-01-01", "code", "main.rs", 600, false);
        ins_web(&conn, "2026-01-01", "instagram.com", "Feed", 300);
        ins_web(&conn, "2026-01-01", "youtube.com", "Vid", 120);
        let stats = compute_stats_for_day(&conn, "2026-01-01").unwrap();
        assert_eq!(stats.instagram_seconds, 300);
        assert_eq!(stats.youtube_seconds, 120);
        assert_eq!(stats.cat_seconds.get("productive").copied().unwrap_or(0), 600);
        assert_eq!(stats.cat_seconds.get("distraction").copied().unwrap_or(0), 300);
    }

    #[test]
    fn effective_checkins_uses_top_goal_and_wrestling() {
        let conn = db::test_conn();
        let day = "2026-02-02";
        conn.execute("INSERT INTO goals (day,title,priority,completed,sort_order,recurring,created_at) VALUES (?1,'A','high',1,0,0,'t')", params![day]).unwrap();
        conn.execute("INSERT INTO goals (day,title,priority,completed,sort_order,recurring,created_at) VALUES (?1,'B','low',0,1,0,'t')", params![day]).unwrap();
        conn.execute("INSERT INTO daily_checkin (day, wrestled, videos_posted) VALUES (?1, 1, 2)", params![day]).unwrap();
        let c = effective_checkins(&conn, day);
        assert!(c.main_goal_completed); // top-priority goal is completed
        assert!(c.gym_logged); // wrestled counts as gym
        assert_eq!(c.videos_posted, 2);
    }

    #[test]
    fn prune_removes_old_keeps_recent() {
        let conn = db::test_conn();
        ins_app(&conn, "2020-01-01", "old", "x", 100, false);
        ins_app(&conn, &local_day(0), "new", "y", 100, false);
        assert_eq!(db::prune(&conn, 30).unwrap(), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM activity_log"), 1);
        // 0 = keep forever
        ins_app(&conn, "2019-01-01", "older", "z", 100, false);
        assert_eq!(db::prune(&conn, 0).unwrap(), 0);
    }

    #[test]
    fn weekly_review_aggregates_and_ranks() {
        let conn = db::test_conn();
        crate::settings::ensure_defaults(&conn).unwrap();
        set_app_rule(&conn, "code", "productive");
        ins_app(&conn, &local_day(0), "code", "x", 3600, false); // today: productive
        ins_web(&conn, &local_day(1), "instagram.com", "feed", 1800); // yesterday: distraction
        let w = weekly_review(&conn).unwrap();
        assert_eq!(w.days.len(), 7);
        assert_eq!(w.most_common_leak.as_deref(), Some("instagram.com"));
        assert_eq!(w.most_common_leak_seconds, 1800);
        assert!(w.productive_seconds >= 3600);
        assert!(w.distraction_seconds >= 1800);
        assert!(w.best_day.is_some() && w.worst_day.is_some());
    }

    #[test]
    fn focus_summary_windows_and_classifies() {
        let conn = db::test_conn();
        set_app_rule(&conn, "code", "productive");
        conn.execute(
            "INSERT INTO focus_sessions (id, goal, started_at, duration_minutes, ends_at, allowed, blocked, status, ended_at)
             VALUES (1, 'g', '2026-01-01T12:00:00+00:00', 60, '2026-01-01T13:00:00+00:00', '[]', '[\"instagram.com\"]', 'ended', '2026-01-01T13:00:00+00:00')",
            [],
        ).unwrap();
        ins_app_ts(&conn, "2026-01-01T12:10:00+00:00", "code", 1800); // [12:10–12:40] focused
        ins_web_ts(&conn, "2026-01-01T12:30:00+00:00", "instagram.com", 600); // [12:30–12:40] blocked
        ins_app_ts(&conn, "2026-01-01T14:00:00+00:00", "code", 5940); // outside window -> excluded
        let s = focus_summary(&conn, 1).unwrap();
        // Overlap-aware: the shared 10 min [12:30–12:40] is credited to the
        // distraction (it wins over focus), so focused = 20 min, not 30.
        assert_eq!(s.focused_seconds, 1200);
        assert_eq!(s.distracted_seconds, 600);
        assert_eq!(s.top_distraction.as_deref(), Some("instagram.com"));
    }

    #[test]
    fn recurring_goals_materialize_once() {
        let conn = db::test_conn();
        conn.execute("INSERT INTO goals (day,title,priority,completed,sort_order,recurring,created_at) VALUES ('2026-01-01','Gym','high',1,0,1,'t')", []).unwrap();
        ensure_recurring_goals(&conn, "2026-01-02");
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM goals WHERE day='2026-01-02' AND title='Gym' AND completed=0 AND recurring=1"),
            1
        );
        ensure_recurring_goals(&conn, "2026-01-02"); // idempotent — no duplicate
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM goals WHERE day='2026-01-02'"), 1);
    }

    #[test]
    fn timeline_merges_runs_and_splits_on_gap() {
        let conn = db::test_conn();
        set_app_rule(&conn, "code", "productive");
        ins_app_ts(&conn, "2026-01-01T12:00:00+00:00", "code", 10);
        ins_app_ts(&conn, "2026-01-01T12:00:10+00:00", "code", 10);
        ins_app_ts(&conn, "2026-01-01T12:00:20+00:00", "code", 10);
        ins_app_ts(&conn, "2026-01-01T12:30:00+00:00", "code", 10); // 30-min gap -> split
        let tl = timeline_for_day(&conn, "2026-01-01", 120).unwrap();
        assert_eq!(tl.blocks.len(), 2);
        assert_eq!(tl.blocks[0].sample_count, 3);
        assert_eq!(tl.blocks[0].duration_seconds, 30);
        assert_eq!(tl.blocks[0].category, "productive");
        assert!(tl.blocks[0].longest_productive); // 30s > 10s
        assert!(tl.blocks[0].first_productive);
        assert_eq!(tl.productive_seconds, 40);
    }

    #[test]
    fn timeline_splits_on_category_change() {
        let conn = db::test_conn();
        set_app_rule(&conn, "code", "productive");
        set_app_rule(&conn, "game", "distraction");
        ins_app_ts(&conn, "2026-01-01T12:00:00+00:00", "code", 10);
        ins_app_ts(&conn, "2026-01-01T12:00:10+00:00", "game", 10); // category change -> split
        let tl = timeline_for_day(&conn, "2026-01-01", 120).unwrap();
        assert_eq!(tl.blocks.len(), 2);
        assert!(tl.blocks[1].biggest_distraction);
    }

    #[test]
    fn output_links_to_nearby_activity_block() {
        let conn = db::test_conn();
        ins_app_ts(&conn, "2026-03-01T11:58:00+00:00", "Code", 10);
        ins_app_ts(&conn, "2026-03-01T12:00:00+00:00", "Code", 10);
        ins_app_ts(&conn, "2026-03-01T12:02:00+00:00", "Code", 10);
        conn.execute(
            "INSERT INTO output_events (timestamp, day, folder_path, file_path, file_name, extension, file_size, event_type, modified_at)
             VALUES ('2026-03-01T12:05:00+00:00','2026-03-01','/exports','/exports/a.mp4','a.mp4','mp4',5000000,'video_export','2026-03-01T12:01:00+00:00')",
            [],
        ).unwrap();
        assert_eq!(link_outputs_for_day(&conn, "2026-03-01").unwrap(), 1);
        let label: String = conn
            .query_row("SELECT linked_label FROM output_events WHERE day='2026-03-01'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(label, "Code");
    }

    #[test]
    fn lockin_generates_and_caches() {
        let conn = db::test_conn();
        let p = generate_plan_core(&conn, "2026-04-01");
        assert_eq!(p.source, "fallback"); // LLM disabled by default
        assert!(!p.main_mission.is_empty());
        let cached = load_plan(&conn, "2026-04-01").unwrap();
        assert_eq!(cached.main_mission, p.main_mission);
    }

    #[test]
    fn lockin_copy_creates_next_day_goals() {
        let conn = db::test_conn();
        let plan = LockinPlan {
            day: "2026-04-01".into(),
            main_mission: "Ship the editor".into(),
            secondary_missions: vec!["Gym".into()],
            first_block: String::new(),
            distraction_rule: String::new(),
            focus_mode: String::new(),
            avoid_trap: String::new(),
            roast_line: String::new(),
            source: "manual".into(),
            edited: true,
        };
        store_plan(&conn, &plan).unwrap();
        assert_eq!(copy_plan_core(&conn, "2026-04-01").unwrap(), 2);
        let c: i64 = conn
            .query_row("SELECT COUNT(*) FROM goals WHERE day='2026-04-02'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(c, 2);
        let pri: String = conn
            .query_row("SELECT priority FROM goals WHERE day='2026-04-02' AND title='Ship the editor'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(pri, "high");
    }

    #[test]
    fn lockin_edited_plan_persists() {
        let conn = db::test_conn();
        let mut plan = generate_plan_core(&conn, "2026-04-03");
        plan.main_mission = "Custom mission".into();
        plan.source = "manual".into();
        plan.edited = true;
        store_plan(&conn, &plan).unwrap();
        let got = load_plan(&conn, "2026-04-03").unwrap();
        assert_eq!(got.main_mission, "Custom mission");
        assert!(got.edited);
        assert_eq!(got.source, "manual");
    }

    #[test]
    fn streaks_compute_consecutive_checkins() {
        let conn = db::test_conn();
        crate::streaks::ensure_defaults(&conn).unwrap();
        for off in [0i64, 1, 2] {
            conn.execute(
                "INSERT INTO daily_checkin (day, studied) VALUES (?1, 1)",
                params![local_day(off)],
            )
            .unwrap();
        }
        let streaks = compute_streaks(&conn).unwrap();
        let studied = streaks.iter().find(|s| s.id == "studied").unwrap();
        assert_eq!(studied.current, 3);
        assert!(studied.best >= 3);
        assert_eq!(studied.calendar.len(), STREAK_WINDOW_DAYS as usize);
    }

    #[test]
    fn streaks_missed_day_breaks_run() {
        let conn = db::test_conn();
        crate::streaks::ensure_defaults(&conn).unwrap();
        // studied today and 2 days ago, but NOT yesterday → current run is just today.
        for off in [0i64, 2] {
            conn.execute(
                "INSERT INTO daily_checkin (day, studied) VALUES (?1, 1)",
                params![local_day(off)],
            )
            .unwrap();
        }
        let streaks = compute_streaks(&conn).unwrap();
        let studied = streaks.iter().find(|s| s.id == "studied").unwrap();
        assert_eq!(studied.current, 1);
    }

    #[test]
    fn streaks_excludes_disabled() {
        let conn = db::test_conn();
        crate::streaks::ensure_defaults(&conn).unwrap();
        let before = compute_streaks(&conn).unwrap().len();
        conn.execute("UPDATE streak_definitions SET enabled = 0 WHERE id = 'gym'", []).unwrap();
        let after = compute_streaks(&conn).unwrap();
        assert_eq!(after.len(), before - 1);
        assert!(after.iter().all(|s| s.id != "gym"));
    }

    #[test]
    fn output_signals_count_by_type() {
        let conn = db::test_conn();
        let mk = |fp: &str, et: &str| {
            conn.execute(
                "INSERT INTO output_events (timestamp, day, folder_path, file_path, file_name, event_type, modified_at)
                 VALUES ('t','2026-03-02','/f', ?1, 'f', ?2, 't')",
                params![fp, et],
            )
            .unwrap();
        };
        mk("/f/a.mp4", "video_export");
        mk("/f/b.mp4", "video_export");
        mk("/f/c.rs", "code_change");
        let sig = output_signals_for_day(&conn, "2026-03-02");
        assert_eq!(sig.video_exports, 2);
        assert_eq!(sig.code_changes, 1);
        assert_eq!(sig.study_materials, 0);
    }

    #[test]
    fn timeline_dedupes_browser_over_desktop() {
        let conn = db::test_conn();
        crate::settings::ensure_defaults(&conn).unwrap();
        ins_app_ts(&conn, "2026-01-01T09:00:00+00:00", "Google Chrome", 10);
        ins_app_ts(&conn, "2026-01-01T09:00:10+00:00", "Google Chrome", 10);
        ins_web_ts(&conn, "2026-01-01T09:00:00+00:00", "youtube.com", 10);
        ins_web_ts(&conn, "2026-01-01T09:00:10+00:00", "youtube.com", 10);
        let tl = timeline_for_day(&conn, "2026-01-01", 120).unwrap();
        // The desktop Chrome run is dropped in favour of the richer browser run.
        assert_eq!(tl.blocks.len(), 1);
        assert_eq!(tl.blocks[0].source, "browser");
        assert_eq!(tl.blocks[0].label, "youtube.com");
    }
}
