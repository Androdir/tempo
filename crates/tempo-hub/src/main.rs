//! Tempo Hub — the headless central server. Receives events from devices into a
//! single shared SQLite database, serves the same dashboard over a REST API, and
//! reuses all of `tempo-core`'s aggregation. No Tauri/GUI deps → builds on a Pi.

mod auth;

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};
use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server};

use tempo_core::db::{self, Db};
use tempo_core::events::{self, EventBatch};
use tempo_core::models::{self, Goal, LockinPlan};
use tempo_core::{aggregate, projects, scoring, settings};

struct Config {
    pairing_secret: String,
    static_dir: String,
    allowed_origins: Vec<String>,
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn main() {
    let port: u16 = env_or("TEMPO_PORT", "7700").parse().unwrap_or(7700);
    let bind = env_or("TEMPO_BIND", "127.0.0.1");
    let db_path = env_or("TEMPO_DB", "tempo-hub.db");
    let static_dir = env_or("TEMPO_STATIC_DIR", "web");
    let allowed_origins: Vec<String> = env_or("TEMPO_ALLOWED_ORIGINS", "")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let pairing_secret = std::env::var("TEMPO_PAIRING_SECRET").unwrap_or_default();

    if pairing_secret.trim().is_empty() {
        eprintln!("[tempo-hub] FATAL: set TEMPO_PAIRING_SECRET to a strong secret before starting.");
        std::process::exit(1);
    }
    if bind == "0.0.0.0" {
        eprintln!("[tempo-hub] ⚠️  WARNING: binding to 0.0.0.0 exposes the hub on ALL network interfaces.");
        eprintln!("[tempo-hub]    Keep it on a private LAN/VPN (Tailscale recommended). Never expose to the internet.");
    }

    let database = db::init(Path::new(&db_path)).expect("[tempo-hub] could not open database");
    {
        let conn = database.lock().expect("db lock");
        let _ = settings::ensure_defaults(&conn);
        let _ = models::ensure_category_defaults(&conn);
        let _ = models::ensure_checkin_defaults(&conn);
        let _ = scoring::ensure_rule_defaults(&conn);
        apply_llm_env(&conn);
    }

    let cfg = Config { pairing_secret, static_dir, allowed_origins };
    let addr = format!("{bind}:{port}");
    let server = match Server::http(&addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[tempo-hub] cannot bind {addr}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("[tempo-hub] listening on http://{addr}  (dashboard from ./{})", cfg.static_dir);

    let limiter = Mutex::new(RateLimiter::default());
    for mut request in server.incoming_requests() {
        let resp = build_response(&database, &cfg, &limiter, &mut request);
        let _ = request.respond(resp);
    }
}

/// Optionally point the hub at an Ollama instance (e.g. your desktop's) via env,
/// so the hub-served Daily Review / Lock-In Plan can be LLM-written instead of the
/// deterministic fallback. Off unless set. The no-cloud guard still applies —
/// loopback / private-LAN / Tailscale URLs are accepted; public/cloud is refused.
fn apply_llm_env(conn: &Connection) {
    if let Ok(url) = std::env::var("TEMPO_OLLAMA_URL") {
        let url = url.trim();
        if !url.is_empty() {
            if tempo_core::llm::is_private_host(url) {
                let _ = settings::set_setting(conn, settings::OLLAMA_URL, url);
            } else {
                eprintln!(
                    "[tempo-hub] ⚠️  ignoring TEMPO_OLLAMA_URL '{url}': not a private address \
                     (cloud LLMs are blocked; use a LAN or Tailscale IP, e.g. http://192.168.x.y:11434)."
                );
            }
        }
    }
    if let Ok(model) = std::env::var("TEMPO_OLLAMA_MODEL") {
        let model = model.trim();
        if !model.is_empty() {
            let _ = settings::set_setting(conn, settings::OLLAMA_MODEL, model);
        }
    }
    if let Ok(v) = std::env::var("TEMPO_LLM_ENABLED") {
        let on = matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on");
        let _ = settings::set_setting(conn, settings::LLM_ENABLED, if on { "1" } else { "0" });
        if on {
            let url = settings::get_setting(conn, settings::OLLAMA_URL)
                .unwrap_or_else(|| settings::DEFAULT_OLLAMA_URL.to_string());
            let model = settings::get_setting(conn, settings::OLLAMA_MODEL)
                .unwrap_or_else(|| settings::DEFAULT_OLLAMA_MODEL.to_string());
            eprintln!(
                "[tempo-hub] LLM enabled → Ollama at {url} (model {model}); \
                 falls back to deterministic if unreachable."
            );
        }
    }
}

// --------------------------------------------------------------- HTTP plumbing

fn build_response(
    db: &Db,
    cfg: &Config,
    limiter: &Mutex<RateLimiter>,
    req: &mut Request,
) -> Response<Cursor<Vec<u8>>> {
    let method = req.method().clone();
    let path = req.url().split('?').next().unwrap_or("").to_string();
    let origin = header(req, "origin");
    let cors = cors_headers(cfg, origin.as_deref());

    if method == Method::Options {
        return with_headers(Response::from_data(Vec::new()).with_status_code(204), &cors);
    }

    if path.starts_with("/api/") {
        let (status, body) = api_route(db, cfg, limiter, &method, &path, req);
        let mut headers = cors;
        headers.push(content_type("application/json"));
        return with_headers(Response::from_data(body.into_bytes()).with_status_code(status), &headers);
    }

    let (status, bytes, ctype) = serve_static(cfg, &path);
    let mut headers = cors;
    headers.push(content_type(ctype));
    with_headers(Response::from_data(bytes).with_status_code(status), &headers)
}

fn api_route(
    db: &Db,
    cfg: &Config,
    limiter: &Mutex<RateLimiter>,
    method: &Method,
    path: &str,
    req: &mut Request,
) -> (u16, String) {
    // Unauthenticated liveness.
    if method == &Method::Get && path == "/api/health" {
        return (200, json!({"ok": true, "app": "tempo-hub"}).to_string());
    }

    match (method, path) {
        // ---- pairing (admin secret + rate limit) ----
        (Method::Post, "/api/pair") => {
            let ip = req.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default();
            if !limiter.lock().map(|mut l| l.allow(&ip, 10, Duration::from_secs(60))).unwrap_or(false) {
                return (429, err("too many pairing attempts"));
            }
            let body = read_body(req);
            let v: Value = match serde_json::from_str(&body) {
                Ok(v) => v,
                Err(e) => return (400, err(&format!("bad json: {e}"))),
            };
            let secret = v.get("pairingSecret").and_then(|x| x.as_str()).unwrap_or("");
            if !auth::secret_ok(secret, &cfg.pairing_secret) {
                return (401, err("invalid pairing secret"));
            }
            let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("device");
            let platform = v.get("platform").and_then(|x| x.as_str()).unwrap_or("unknown");
            let Ok(conn) = db.lock() else { return (500, err("db lock")) };
            match auth::pair_device(&conn, name, platform) {
                Ok((id, token)) => (200, json!({"ok": true, "deviceId": id, "token": token}).to_string()),
                Err(e) => (500, err(&e)),
            }
        }

        // ---- event ingestion (device token) ----
        (Method::Post, "/api/events") => {
            let Some(token) = bearer(req) else { return (401, err("missing device token")) };
            let Ok(conn) = db.lock() else { return (500, err("db lock")) };
            let Some(device_id) = auth::device_for_token(&conn, &token) else {
                return (401, err("unknown or revoked device"));
            };
            let body = read_body(req);
            let batch: EventBatch = match serde_json::from_str(&body) {
                Ok(b) => b,
                Err(e) => return (400, err(&format!("bad batch: {e}"))),
            };
            let mut stored = 0i64;
            let mut duplicates = 0i64;
            for e in &batch.events {
                match events::ingest_event(&conn, &device_id, e) {
                    Ok(true) => stored += 1,
                    Ok(false) => duplicates += 1,
                    Err(err) => {
                        let _ = conn.execute(
                            "INSERT INTO sync_errors (timestamp, context, message) VALUES (?1, 'ingest', ?2)",
                            params![chrono::Utc::now().to_rfc3339(), err.to_string()],
                        );
                    }
                }
            }
            (200, json!({"ok": true, "stored": stored, "duplicates": duplicates}).to_string())
        }

        // ---- web dashboard transport (pairing secret as web token) ----
        (Method::Post, "/api/invoke") => {
            let Some(token) = bearer(req) else { return (401, err("missing token")) };
            if !auth::secret_ok(&token, &cfg.pairing_secret) {
                return (401, err("unauthorized"));
            }
            let body = read_body(req);
            let v: Value = match serde_json::from_str(&body) {
                Ok(v) => v,
                Err(e) => return (400, err(&format!("bad json: {e}"))),
            };
            let cmd = v.get("cmd").and_then(|x| x.as_str()).unwrap_or("");
            let args = v.get("args").cloned().unwrap_or(Value::Null);
            let Ok(conn) = db.lock() else { return (500, err("db lock")) };
            match dispatch(&conn, cmd, &args) {
                Ok(val) => (200, val.to_string()),
                Err(e) => (400, err(&e)),
            }
        }

        // ---- admin: devices list / revoke (pairing secret) ----
        (Method::Get, "/api/devices") => {
            let Some(token) = bearer(req) else { return (401, err("unauthorized")) };
            if !auth::secret_ok(&token, &cfg.pairing_secret) {
                return (401, err("unauthorized"));
            }
            let Ok(conn) = db.lock() else { return (500, err("db lock")) };
            (200, list_devices(&conn))
        }
        (Method::Post, "/api/devices/revoke") => {
            let Some(token) = bearer(req) else { return (401, err("unauthorized")) };
            if !auth::secret_ok(&token, &cfg.pairing_secret) {
                return (401, err("unauthorized"));
            }
            let body = read_body(req);
            let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("");
            let Ok(conn) = db.lock() else { return (500, err("db lock")) };
            match auth::revoke_device(&conn, id) {
                Ok(_) => (200, json!({"ok": true}).to_string()),
                Err(e) => (500, err(&e)),
            }
        }

        _ => (404, err("not found")),
    }
}

fn validate_category_definition(category: models::CategoryDefinition) -> Result<models::CategoryDefinition, String> {
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
    Ok(models::CategoryDefinition {
        id,
        label: category.label.trim().to_string(),
        color: category.color.trim().to_string(),
        bucket: category.bucket,
        blurb: category.blurb.trim().to_string(),
        built_in: category.built_in,
    })
}

fn validate_project(conn: &Connection, p: &projects::Project) -> Result<(), String> {
    if p.name.trim().is_empty() {
        return Err("Project name is required".into());
    }
    if !models::category_exists(conn, &p.category) {
        return Err(format!("Unknown category: {}", p.category));
    }
    Ok(())
}

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

fn ensure_recurring_goals(conn: &Connection, day: &str) {
    if settings::get_setting(conn, settings::RECURRING_MATERIALIZED_DAY).as_deref() == Some(day) {
        return;
    }

    let src: Option<String> = conn
        .query_row("SELECT MAX(day) FROM goals WHERE day < ?1 AND recurring = 1", [day], |r| {
            r.get::<_, Option<String>>(0)
        })
        .ok()
        .flatten();

    if let Some(src_day) = src {
        let mut templates: Vec<(String, Option<String>, Option<i64>, String)> = Vec::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT title, project, target_minutes, priority FROM goals
             WHERE day = ?1 AND recurring = 1 ORDER BY sort_order, id",
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
        let mut order = aggregate::next_sort_order(conn, day);
        for (title, project, target, priority) in templates {
            if aggregate::goal_exists(conn, day, &title) {
                continue;
            }
            let _ = conn.execute(
                "INSERT INTO goals (day, title, project, target_minutes, priority, completed, sort_order, recurring, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, 1, ?7)",
                params![day, title, project, target, priority, order, chrono::Utc::now().to_rfc3339()],
            );
            order += 1;
        }
    }
    let _ = settings::set_setting(conn, settings::RECURRING_MATERIALIZED_DAY, day);
}

fn list_goals(conn: &Connection, day: &str) -> Result<Vec<Goal>, String> {
    ensure_recurring_goals(conn, day);
    let mut stmt = conn
        .prepare(
            "SELECT id, title, project, target_minutes, priority, completed, recurring
             FROM goals WHERE day = ?1
             ORDER BY CASE priority WHEN 'high' THEN 0 WHEN 'medium' THEN 1 ELSE 2 END, sort_order, id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([day], row_to_goal).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

fn add_goal_for_day(conn: &Connection, day: &str, goal: Goal) -> Result<i64, String> {
    let title = goal.title.trim();
    if title.is_empty() {
        return Err("Goal title is required".into());
    }
    let priority = normalize_priority(&goal.priority);
    let project = goal.project.filter(|p| !p.trim().is_empty());
    let target = goal.target_minutes.filter(|m| *m > 0);
    let order = aggregate::next_sort_order(conn, day);
    conn.execute(
        "INSERT INTO goals (day, title, project, target_minutes, priority, completed, sort_order, recurring, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8)",
        params![day, title, project, target, priority, order, goal.recurring as i64, chrono::Utc::now().to_rfc3339()],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

fn update_goal_row(conn: &Connection, goal: Goal) -> Result<(), String> {
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
    conn.execute(
        "UPDATE goals SET title = ?1, project = ?2, target_minutes = ?3, priority = ?4,
                          completed = ?5, recurring = ?6 WHERE id = ?7",
        params![title, project, target, priority, goal.completed as i64, goal.recurring as i64, goal.id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn copy_previous_goals(conn: &Connection, day: &str) -> Result<i64, String> {
    let src: Option<String> = conn
        .query_row("SELECT MAX(day) FROM goals WHERE day < ?1", [day], |r| {
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

    let mut order = aggregate::next_sort_order(conn, day);
    let mut count = 0i64;
    for (title, project, target, priority, recurring) in templates {
        if aggregate::goal_exists(conn, day, &title) {
            continue;
        }
        conn.execute(
            "INSERT INTO goals (day, title, project, target_minutes, priority, completed, sort_order, recurring, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8)",
            params![day, title, project, target, priority, order, recurring, chrono::Utc::now().to_rfc3339()],
        )
        .map_err(|e| e.to_string())?;
        order += 1;
        count += 1;
    }
    Ok(count)
}

/// Dispatch a frontend command to the shared aggregation. Covers the full
/// read dashboard (Today, Timeline, Daily Score, Streaks, Weekly Review, Output
/// Events, Daily Review, Lock-In Plan, Focus summary) plus goals/check-ins/notes/
/// plan writes — all reusing the exact same `tempo_core::aggregate` cores the
/// desktop uses. The Focus summary is computed across every synced device.
fn dispatch(conn: &Connection, cmd: &str, args: &Value) -> Result<Value, String> {
    let day = args.get("day").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(aggregate::today);
    match cmd {
        "get_today_summary" => Ok(serde_json::to_value(aggregate::summary_for_day(conn, &day)?).unwrap()),
        "get_timeline_for_day" => {
            let gap = args.get("maxGapSeconds").and_then(|v| v.as_i64()).unwrap_or(120).clamp(20, 3600);
            Ok(serde_json::to_value(aggregate::timeline_for_day(conn, &day, gap)?).unwrap())
        }
        "get_daily_score" => Ok(serde_json::to_value(aggregate::score_report_for_day(conn, &day)?).unwrap()),
        "get_streaks" => Ok(serde_json::to_value(aggregate::compute_streaks(conn)?).unwrap()),
        "get_weekly_review" => Ok(serde_json::to_value(aggregate::weekly_review(conn)?).unwrap()),
        "get_output_events" => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, timestamp, day, folder_path, file_path, file_name, extension, file_size,
                            event_type, project, linked_block_key, linked_label, created_at, modified_at
                     FROM output_events WHERE day = ?1 ORDER BY COALESCE(modified_at, timestamp) DESC",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt.query_map([&day], aggregate::row_to_output).map_err(|e| e.to_string())?;
            let out: Vec<_> = rows.filter_map(Result::ok).collect();
            Ok(serde_json::to_value(out).unwrap())
        }
        "get_category_definitions" => {
            Ok(serde_json::to_value(models::list_category_definitions(conn).map_err(|e| e.to_string())?).unwrap())
        }
        "upsert_category_definition" => {
            let category: models::CategoryDefinition =
                serde_json::from_value(args.get("category").cloned().ok_or("missing category")?)
                    .map_err(|e| e.to_string())?;
            let category = validate_category_definition(category)?;
            models::upsert_category_definition(conn, &category).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "delete_category_definition" => {
            let id = args.get("id").and_then(|v| v.as_str()).ok_or("missing id")?;
            models::delete_category_definition(conn, &id.trim().to_ascii_lowercase()).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "get_projects" => Ok(serde_json::to_value(projects::list_projects(conn).map_err(|e| e.to_string())?).unwrap()),
        "create_project" => {
            let project: projects::Project =
                serde_json::from_value(args.get("project").cloned().ok_or("missing project")?)
                    .map_err(|e| e.to_string())?;
            validate_project(conn, &project)?;
            Ok(serde_json::to_value(projects::create_project(conn, &project).map_err(|e| e.to_string())?).unwrap())
        }
        "update_project" => {
            let project: projects::Project =
                serde_json::from_value(args.get("project").cloned().ok_or("missing project")?)
                    .map_err(|e| e.to_string())?;
            validate_project(conn, &project)?;
            if project.id <= 0 {
                return Err("Missing project id".into());
            }
            projects::update_project(conn, &project).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "delete_project" => {
            let id = args.get("id").and_then(|v| v.as_i64()).ok_or("missing id")?;
            projects::delete_project(conn, id).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "get_goals" => {
            Ok(serde_json::to_value(list_goals(conn, &day)?).unwrap())
        }
        "add_goal" => {
            let goal: Goal =
                serde_json::from_value(args.get("goal").cloned().ok_or("missing goal")?)
                    .map_err(|e| e.to_string())?;
            Ok(serde_json::to_value(add_goal_for_day(conn, &day, goal)?).unwrap())
        }
        "update_goal" => {
            let goal: Goal =
                serde_json::from_value(args.get("goal").cloned().ok_or("missing goal")?)
                    .map_err(|e| e.to_string())?;
            update_goal_row(conn, goal)?;
            Ok(Value::Null)
        }
        "get_checkins" => Ok(serde_json::to_value(models::checkin_values_for_day(conn, &day)).unwrap()),
        "set_checkin" => {
            let field = args.get("field").and_then(|v| v.as_str()).ok_or("missing field")?;
            let raw = args.get("value").and_then(|v| v.as_i64()).unwrap_or(0);
            if field == "main_goal_completed" {
                conn.execute("INSERT OR IGNORE INTO daily_checkin (day) VALUES (?1)", params![day])
                    .map_err(|e| e.to_string())?;
                conn.execute(
                    "UPDATE daily_checkin SET main_goal_completed = ?1 WHERE day = ?2",
                    params![(raw != 0) as i64, day],
                )
                .map_err(|e| e.to_string())?;
            } else {
                models::set_checkin_value(conn, &day, field, raw)?;
            }
            Ok(Value::Null)
        }
        "clear_checkin" => {
            let field = args.get("field").and_then(|v| v.as_str()).ok_or("missing field")?;
            models::clear_checkin_value(conn, &day, field)?;
            Ok(Value::Null)
        }
        "get_checkin_definitions" => {
            Ok(serde_json::to_value(models::list_checkin_definitions(conn).map_err(|e| e.to_string())?).unwrap())
        }
        "upsert_checkin_definition" => {
            let checkin: models::CheckinDefinition =
                serde_json::from_value(args.get("checkin").cloned().ok_or("missing checkin")?)
                    .map_err(|e| e.to_string())?;
            models::upsert_checkin_definition(conn, &checkin)?;
            Ok(Value::Null)
        }
        "delete_checkin_definition" => {
            let id = args.get("id").and_then(|v| v.as_str()).ok_or("missing id")?;
            models::delete_checkin_definition(conn, id)?;
            Ok(Value::Null)
        }
        "toggle_goal" => {
            let id = args.get("id").and_then(|v| v.as_i64()).ok_or("missing id")?;
            let completed = args.get("completed").and_then(|v| v.as_bool()).unwrap_or(false);
            conn.execute("UPDATE goals SET completed = ?1 WHERE id = ?2", params![completed as i64, id])
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "delete_goal" => {
            let id = args.get("id").and_then(|v| v.as_i64()).ok_or("missing id")?;
            conn.execute("DELETE FROM goals WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "set_goal_recurring" => {
            let id = args.get("id").and_then(|v| v.as_i64()).ok_or("missing id")?;
            let recurring = args.get("recurring").and_then(|v| v.as_bool()).unwrap_or(false);
            conn.execute("UPDATE goals SET recurring = ?1 WHERE id = ?2", params![recurring as i64, id])
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "copy_previous_goals" => {
            Ok(serde_json::to_value(copy_previous_goals(conn, &day)?).unwrap())
        }
        "set_scoring_weight" => {
            let id = args.get("id").and_then(|v| v.as_str()).ok_or("missing id")?;
            let weight = args.get("weight").and_then(|v| v.as_i64()).ok_or("missing weight")?;
            scoring::set_weight(conn, id, weight)?;
            Ok(Value::Null)
        }
        "set_scoring_threshold" => {
            let id = args.get("id").and_then(|v| v.as_str()).ok_or("missing id")?;
            let threshold = args.get("threshold").and_then(|v| v.as_i64()).ok_or("missing threshold")?;
            scoring::set_threshold(conn, id, threshold)?;
            Ok(Value::Null)
        }
        "reset_scoring_weights" => {
            scoring::reset(conn)?;
            Ok(Value::Null)
        }
        "get_score_rules" => Ok(serde_json::to_value(scoring::list_rules(conn)).unwrap()),
        "upsert_score_rule" => {
            let rule: scoring::ScoreRule =
                serde_json::from_value(args.get("rule").cloned().ok_or("missing rule")?)
                    .map_err(|e| e.to_string())?;
            scoring::upsert_rule(conn, &rule)?;
            Ok(Value::Null)
        }
        "delete_score_rule" => {
            let id = args.get("id").and_then(|v| v.as_str()).ok_or("missing id")?;
            scoring::delete_rule(conn, id)?;
            Ok(Value::Null)
        }
        "get_streak_definitions" => {
            let defs: Vec<models::StreakDefinition> = tempo_core::streaks::load_defs(conn)
                .into_iter()
                .map(|d| models::StreakDefinition {
                    id: d.id,
                    name: d.name,
                    kind: d.kind,
                    metric: d.metric,
                    threshold: d.threshold,
                    enabled: d.enabled,
                    days_per_week: d.days_per_week,
                })
                .collect();
            Ok(serde_json::to_value(defs).unwrap())
        }
        "update_streak_definition" => {
            let id = args.get("id").and_then(|v| v.as_str()).ok_or("missing id")?;
            let now = chrono::Utc::now().to_rfc3339();
            if let Some(en) = args.get("enabled").and_then(|v| v.as_bool()) {
                conn.execute(
                    "UPDATE streak_definitions SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
                    params![en as i64, now, id],
                )
                .map_err(|e| e.to_string())?;
            }
            if let Some(th) = args.get("threshold").and_then(|v| v.as_i64()) {
                conn.execute(
                    "UPDATE streak_definitions SET threshold = ?1, updated_at = ?2 WHERE id = ?3",
                    params![th.clamp(0, 100_000), now, id],
                )
                .map_err(|e| e.to_string())?;
            }
            if let Some(name) = args.get("name").and_then(|v| v.as_str()).filter(|n| !n.trim().is_empty()) {
                conn.execute(
                    "UPDATE streak_definitions SET name = ?1, updated_at = ?2 WHERE id = ?3",
                    params![name.trim(), now, id],
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(Value::Null)
        }
        "add_streak_definition" => {
            let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let kind = args.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let metric = args.get("metric").and_then(|v| v.as_str()).unwrap_or("");
            let threshold = args.get("threshold").and_then(|v| v.as_i64()).unwrap_or(0);
            let days_per_week = args.get("daysPerWeek").and_then(|v| v.as_i64()).unwrap_or(0);
            tempo_core::streaks::add_definition(conn, id, name, kind, metric, threshold, days_per_week)?;
            Ok(Value::Null)
        }
        "delete_streak_definition" => {
            let id = args.get("id").and_then(|v| v.as_str()).ok_or("missing id")?;
            tempo_core::streaks::delete_definition(conn, id)?;
            Ok(Value::Null)
        }
        "seed_default_streaks" => {
            Ok(serde_json::to_value(tempo_core::streaks::seed_suggested(conn).map_err(|e| e.to_string())?)
                .unwrap())
        }
        "set_daily_note" => {
            let notes = args.get("notes").and_then(|v| v.as_str()).unwrap_or("");
            conn.execute("INSERT OR IGNORE INTO daily_checkin (day) VALUES (?1)", params![day])
                .map_err(|e| e.to_string())?;
            conn.execute("UPDATE daily_checkin SET notes = ?1 WHERE day = ?2", params![notes, day])
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        // --- daily review (today-scoped, mirrors the desktop wrappers) ---
        "get_daily_review" => Ok(serde_json::to_value(aggregate::daily_review(conn)?).unwrap()),
        "generate_daily_review" => Ok(serde_json::to_value(aggregate::generate_review(conn)?).unwrap()),
        // --- daily lock-in plan ---
        "get_lockin_plan" => Ok(serde_json::to_value(aggregate::load_plan(conn, &day)).unwrap()),
        "generate_lockin_plan" => {
            Ok(serde_json::to_value(aggregate::generate_plan_core(conn, &day)).unwrap())
        }
        "save_lockin_plan" => {
            let mut p: LockinPlan =
                serde_json::from_value(args.get("plan").cloned().ok_or("missing plan")?)
                    .map_err(|e| e.to_string())?;
            p.day = day;
            p.source = "manual".to_string();
            p.edited = true;
            aggregate::store_plan(conn, &p)?;
            Ok(Value::Null)
        }
        "copy_lockin_plan_to_goals" => {
            Ok(serde_json::to_value(aggregate::copy_plan_core(conn, &day)?).unwrap())
        }
        // --- focus sessions (cross-device adherence; sessions sync once they end) ---
        "get_focus_session" => {
            Ok(serde_json::to_value(aggregate::latest_focus_session_today(conn)).unwrap())
        }
        "get_focus_summary" => {
            let id = args.get("id").and_then(|v| v.as_i64()).ok_or("missing id")?;
            Ok(serde_json::to_value(aggregate::focus_summary(conn, id)?).unwrap())
        }
        // --- which device contributed what today (desktop vs phone) ---
        "get_device_breakdown" => device_breakdown(conn, &day),
        other => Err(format!("command not available on the hub yet: {other}")),
    }
}

/// Per-device active time today (desktop vs phone …), straight from the synced-event
/// ledger. Idle samples are excluded via the event metadata, so an always-on but
/// unused desktop never inflates its share. Returns each device's total plus its
/// single biggest app/site.
fn device_breakdown(conn: &Connection, day: &str) -> Result<Value, String> {
    let mut stmt = conn
        .prepare(
            "SELECT s.device_id,
                    COALESCE(d.name, s.device_id) AS name,
                    COALESCE(d.platform, '') AS platform,
                    COALESCE(s.app_name, s.domain, '?') AS label,
                    SUM(s.duration_seconds) AS secs
             FROM synced_events s
             LEFT JOIN devices d ON d.id = s.device_id
             WHERE s.day = ?1
               AND s.event_type IN ('app_sample', 'browser_sample')
               AND COALESCE(json_extract(s.metadata_json, '$.isIdle'), 0) = 0
               AND COALESCE(s.duration_seconds, 0) > 0
             GROUP BY s.device_id, label",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([day], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    struct Dev {
        name: String,
        platform: String,
        active: i64,
        top_label: String,
        top_secs: i64,
    }
    let mut devs: HashMap<String, Dev> = HashMap::new();
    for row in rows {
        let (id, name, platform, label, secs) = row.map_err(|e| e.to_string())?;
        let d = devs.entry(id).or_insert(Dev {
            name,
            platform,
            active: 0,
            top_label: String::new(),
            top_secs: 0,
        });
        d.active += secs;
        if secs > d.top_secs {
            d.top_secs = secs;
            d.top_label = label;
        }
    }
    let mut list: Vec<Value> = devs
        .into_iter()
        .map(|(id, d)| {
            json!({
                "deviceId": id,
                "name": d.name,
                "platform": d.platform,
                "activeSeconds": d.active,
                "topLabel": d.top_label,
                "topLabelSeconds": d.top_secs,
            })
        })
        .collect();
    list.sort_by(|a, b| {
        b["activeSeconds"].as_i64().unwrap_or(0).cmp(&a["activeSeconds"].as_i64().unwrap_or(0))
    });
    Ok(Value::Array(list))
}

fn list_devices(conn: &Connection) -> String {
    let mut stmt = match conn
        .prepare("SELECT id, name, platform, revoked, created_at, last_seen FROM devices ORDER BY created_at")
    {
        Ok(s) => s,
        Err(e) => return err(&e.to_string()),
    };
    let rows = stmt.query_map([], |r| {
        Ok(json!({
            "id": r.get::<_, String>(0)?,
            "name": r.get::<_, Option<String>>(1)?,
            "platform": r.get::<_, Option<String>>(2)?,
            "revoked": r.get::<_, i64>(3)? != 0,
            "createdAt": r.get::<_, Option<String>>(4)?,
            "lastSeen": r.get::<_, Option<String>>(5)?,
        }))
    });
    match rows {
        Ok(rs) => json!({"devices": rs.filter_map(Result::ok).collect::<Vec<_>>()}).to_string(),
        Err(e) => err(&e.to_string()),
    }
}

// ---- static SPA serving ----

fn serve_static(cfg: &Config, path: &str) -> (u16, Vec<u8>, &'static str) {
    if path.contains("..") {
        return (400, b"bad path".to_vec(), "text/plain");
    }
    let rel = if path == "/" { "index.html" } else { path.trim_start_matches('/') };
    let full: PathBuf = Path::new(&cfg.static_dir).join(rel);
    if full.is_file() {
        if let Ok(bytes) = std::fs::read(&full) {
            return (200, bytes, ctype_for(rel));
        }
    }
    // SPA fallback → index.html
    let index = Path::new(&cfg.static_dir).join("index.html");
    match std::fs::read(&index) {
        Ok(bytes) => (200, bytes, "text/html; charset=utf-8"),
        Err(_) => (404, b"Dashboard not built. Build the React app and set TEMPO_STATIC_DIR.".to_vec(), "text/plain"),
    }
}

fn ctype_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

// ---- header / cors helpers ----

fn header(req: &Request, name: &str) -> Option<String> {
    req.headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str().to_string())
}

fn bearer(req: &Request) -> Option<String> {
    let v = header(req, "authorization")?;
    v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")).map(|s| s.trim().to_string())
}

fn read_body(req: &mut Request) -> String {
    let mut s = String::new();
    let _ = req.as_reader().read_to_string(&mut s);
    s
}

fn content_type(ct: &str) -> Header {
    Header::from_bytes(&b"Content-Type"[..], ct.as_bytes()).expect("valid header")
}

/// CORS: same-origin needs none; for cross-origin clients we only reflect an
/// Origin that's in the allowlist (empty allowlist = LAN-only, reflect any).
fn cors_headers(cfg: &Config, origin: Option<&str>) -> Vec<Header> {
    let mut out = Vec::new();
    let allow = match origin {
        Some(o) if cfg.allowed_origins.is_empty() || cfg.allowed_origins.iter().any(|a| a == o) => Some(o),
        _ => None,
    };
    if let Some(o) = allow {
        out.push(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], o.as_bytes()).unwrap());
        out.push(Header::from_bytes(&b"Vary"[..], &b"Origin"[..]).unwrap());
        out.push(
            Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"authorization, content-type"[..]).unwrap(),
        );
        out.push(
            Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"GET, POST, OPTIONS"[..]).unwrap(),
        );
    }
    out
}

fn with_headers(mut resp: Response<Cursor<Vec<u8>>>, headers: &[Header]) -> Response<Cursor<Vec<u8>>> {
    for h in headers {
        resp.add_header(h.clone());
    }
    resp
}

fn err(msg: &str) -> String {
    json!({"ok": false, "error": msg}).to_string()
}

#[derive(Default)]
struct RateLimiter {
    hits: HashMap<String, (u32, Instant)>,
}
impl RateLimiter {
    fn allow(&mut self, ip: &str, max: u32, window: Duration) -> bool {
        let now = Instant::now();
        let e = self.hits.entry(ip.to_string()).or_insert((0, now));
        if now.duration_since(e.1) > window {
            *e = (0, now);
        }
        e.0 += 1;
        e.0 <= max
    }
}

// ===================================================================== tests
#[cfg(test)]
mod tests {
    use super::*;
    use tempo_core::events::SyncEvent;

    fn app_event(eid: &str, app: &str, secs: i64) -> SyncEvent {
        SyncEvent {
            event_id: eid.into(),
            event_type: "app_sample".into(),
            source: Some("desktop".into()),
            timestamp: "2026-01-01T12:00:00+00:00".into(),
            day: "2026-01-01".into(),
            app_name: Some(app.into()),
            domain: None,
            title: Some("work".into()),
            duration_seconds: Some(secs),
            category: None,
            project: None,
            metadata: None,
        }
    }

    #[test]
    fn pairing_token_lifecycle_and_revoke() {
        let conn = db::test_conn();
        let (id, token) = auth::pair_device(&conn, "Desktop", "windows").unwrap();
        assert_eq!(auth::device_for_token(&conn, &token).as_deref(), Some(id.as_str()));
        assert!(auth::device_for_token(&conn, "not-a-real-token").is_none()); // invalid token
        auth::revoke_device(&conn, &id).unwrap();
        assert!(auth::device_for_token(&conn, &token).is_none()); // revoked device rejected
    }

    #[test]
    fn pairing_secret_check() {
        assert!(auth::secret_ok("hunter2", "hunter2"));
        assert!(!auth::secret_ok("wrong", "hunter2"));
        assert!(!auth::secret_ok("", "")); // empty secret never matches
    }

    #[test]
    fn dedup_and_multi_device_aggregation() {
        let conn = db::test_conn();
        // Same event_id from one device twice → counted once.
        assert!(events::ingest_event(&conn, "A", &app_event("activity_log:1", "Code", 600)).unwrap());
        assert!(!events::ingest_event(&conn, "A", &app_event("activity_log:1", "Code", 600)).unwrap());
        // Different device, same local id → genuinely separate, both counted.
        assert!(events::ingest_event(&conn, "B", &app_event("activity_log:1", "Code", 300)).unwrap());

        let s = aggregate::summary_for_day(&conn, "2026-01-01").unwrap();
        assert_eq!(s.total_active_seconds, 900); // 600 (A) + 300 (B), A's re-upload ignored
    }

    #[test]
    fn dispatch_round_trips_checkins_and_summary() {
        let conn = db::test_conn();
        events::ingest_event(&conn, "A", &app_event("activity_log:1", "Code", 600)).unwrap();
        let v = dispatch(&conn, "get_today_summary", &json!({"day": "2026-01-01"})).unwrap();
        assert_eq!(v["totalActiveSeconds"], 600);

        dispatch(&conn, "set_checkin", &json!({"field": "videos_posted", "value": 3})).unwrap();
        let c = dispatch(&conn, "get_checkins", &json!({})).unwrap();
        let videos = c
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == "videos_posted")
            .expect("videos_posted check-in exists");
        assert_eq!(videos["value"], 3);
    }

    #[test]
    fn dispatch_manages_checkin_definitions() {
        let conn = db::test_conn();
        dispatch(&conn, "upsert_checkin_definition", &json!({"checkin": {
            "id": "growth_research", "label": "Growth research", "icon": "📈",
            "kind": "toggle", "builtIn": false
        }})).unwrap();
        dispatch(&conn, "set_checkin", &json!({"field": "growth_research", "value": 1})).unwrap();
        let c = dispatch(&conn, "get_checkins", &json!({})).unwrap();
        assert!(c.as_array().unwrap().iter().any(|x| x["id"] == "growth_research" && x["value"] == 1));
        dispatch(&conn, "delete_checkin_definition", &json!({"id": "growth_research"})).unwrap();
        let c = dispatch(&conn, "get_checkins", &json!({})).unwrap();
        assert!(c.as_array().unwrap().iter().all(|x| x["id"] != "growth_research"));
    }

    #[test]
    fn checkin_def_event_syncs_full_definition_and_deletion() {
        let conn = db::test_conn();
        // A definition event carries the auto-detection config end-to-end.
        let mut e = app_event("checkin_def:read_bible:1", "x", 0);
        e.event_type = "checkin_def".into();
        e.app_name = None;
        e.duration_seconds = None;
        e.metadata = Some(serde_json::json!({
            "id": "read_bible", "label": "Read Bible", "icon": "📖", "kind": "toggle",
            "autoKind": "target", "autoMetric": "bible", "autoThreshold": 30
        }));
        assert!(events::ingest_event(&conn, "desktop", &e).unwrap());
        let defs = dispatch(&conn, "get_checkin_definitions", &json!({})).unwrap();
        let d = defs.as_array().unwrap().iter().find(|d| d["id"] == "read_bible").expect("synced");
        assert_eq!(d["autoKind"], "target");
        assert_eq!(d["autoThreshold"], 30);

        // The hub now auto-detects from its own (multi-device) activity.
        conn.execute(
            "INSERT INTO activity_log (timestamp, day, app_name, window_title, duration_seconds, is_idle)
             VALUES ('2026-01-01T10:00:00+00:00', '2026-01-01', 'Bible App', 'x', 2400, 0)",
            [],
        )
        .unwrap();
        let c = dispatch(&conn, "get_checkins", &json!({"day": "2026-01-01"})).unwrap();
        let bible = c.as_array().unwrap().iter().find(|x| x["id"] == "read_bible").unwrap();
        assert_eq!(bible["value"], 1); // 40 min ≥ 30 min threshold
        assert_eq!(bible["detected"], 40);

        // A cleared-override event deletes any manual row.
        dispatch(&conn, "set_checkin", &json!({"day": "2026-01-01", "field": "read_bible", "value": 0}))
            .unwrap();
        let mut clear = app_event("checkin:2026-01-01:read_bible:clear:2", "x", 0);
        clear.event_type = "checkin".into();
        clear.app_name = None;
        clear.duration_seconds = None;
        clear.day = "2026-01-01".into();
        clear.metadata = Some(serde_json::json!({ "field": "read_bible", "cleared": true }));
        assert!(events::ingest_event(&conn, "desktop", &clear).unwrap());
        let c = dispatch(&conn, "get_checkins", &json!({"day": "2026-01-01"})).unwrap();
        let bible = c.as_array().unwrap().iter().find(|x| x["id"] == "read_bible").unwrap();
        assert_eq!(bible["value"], 1); // back to auto
        assert_eq!(bible["overridden"], false);

        // A deletion event removes the definition.
        let mut del = app_event("checkin_def:read_bible:del:3", "x", 0);
        del.event_type = "checkin_def".into();
        del.app_name = None;
        del.duration_seconds = None;
        del.metadata = Some(serde_json::json!({ "id": "read_bible", "deleted": true }));
        assert!(events::ingest_event(&conn, "desktop", &del).unwrap());
        let defs = dispatch(&conn, "get_checkin_definitions", &json!({})).unwrap();
        assert!(defs.as_array().unwrap().iter().all(|d| d["id"] != "read_bible"));
    }

    #[test]
    fn checkin_event_auto_registers_definition() {
        let conn = db::test_conn();
        let mut e = app_event("checkin:2026-01-01:drilling:1", "x", 0);
        e.event_type = "checkin".into();
        e.app_name = None;
        e.duration_seconds = None;
        e.metadata = Some(serde_json::json!({
            "field": "drilling", "value": 1, "label": "Drilling session", "icon": "🤼", "kind": "toggle"
        }));
        assert!(events::ingest_event(&conn, "desktop", &e).unwrap());
        let c = dispatch(&conn, "get_checkins", &json!({"day": "2026-01-01"})).unwrap();
        let drill = c.as_array().unwrap().iter().find(|x| x["id"] == "drilling").expect("registered");
        assert_eq!(drill["value"], 1);
        assert_eq!(drill["label"], "Drilling session");
    }

    #[test]
    fn dispatch_supports_web_dashboard_writes() {
        let conn = db::test_conn();

        let pid = dispatch(&conn, "create_project", &json!({"project": {
            "id": 0, "name": "Content", "category": "business",
            "keywords": ["hooks"], "apps": ["Premiere Pro"], "domains": ["youtube.com"], "priority": 80
        }})).unwrap().as_i64().unwrap();
        let projects = dispatch(&conn, "get_projects", &json!({})).unwrap();
        assert!(projects.as_array().unwrap().iter().any(|p| p["id"] == pid));
        dispatch(&conn, "update_project", &json!({"project": {
            "id": pid, "name": "Content Ops", "category": "business",
            "keywords": ["retention"], "apps": ["Premiere Pro"], "domains": ["youtube.com"], "priority": 90
        }})).unwrap();

        let gid = dispatch(&conn, "add_goal", &json!({"day": "2026-01-02", "goal": {
            "title": "Ship video", "project": "Content Ops", "targetMinutes": 45,
            "priority": "high", "recurring": true
        }})).unwrap().as_i64().unwrap();
        dispatch(&conn, "update_goal", &json!({"goal": {
            "id": gid, "title": "Ship edited video", "project": "Content Ops",
            "targetMinutes": 30, "priority": "medium", "completed": true, "recurring": false
        }})).unwrap();
        dispatch(&conn, "set_goal_recurring", &json!({"id": gid, "recurring": true})).unwrap();
        dispatch(&conn, "toggle_goal", &json!({"id": gid, "completed": false})).unwrap();
        let goals = dispatch(&conn, "get_goals", &json!({"day": "2026-01-02"})).unwrap();
        assert_eq!(goals[0]["title"], "Ship edited video");
        assert_eq!(goals[0]["completed"], false);
        assert_eq!(goals[0]["recurring"], true);

        let copied = dispatch(&conn, "copy_previous_goals", &json!({"day": "2026-01-03"})).unwrap();
        assert_eq!(copied, 1);
        dispatch(&conn, "delete_goal", &json!({"id": gid})).unwrap();
        let goals_after_delete = dispatch(&conn, "get_goals", &json!({"day": "2026-01-02"})).unwrap();
        assert!(goals_after_delete.as_array().unwrap().is_empty());

        dispatch(&conn, "set_scoring_weight", &json!({"id": "main_goal", "weight": 42})).unwrap();
        dispatch(&conn, "set_scoring_threshold", &json!({"id": "business_min", "threshold": 120})).unwrap();
        let rules = dispatch(&conn, "get_score_rules", &json!({})).unwrap();
        let rules = rules.as_array().unwrap();
        assert!(rules.iter().any(|r| r["id"] == "main_goal" && r["weight"] == 42));
        assert!(rules.iter().any(|r| r["id"] == "business_min" && r["threshold"] == 120));

        // Custom rule lifecycle: add against a custom check-in, then delete.
        dispatch(&conn, "upsert_checkin_definition", &json!({"checkin": {
            "id": "drilling", "label": "Drilling", "icon": "🤼", "kind": "toggle", "builtIn": false
        }})).unwrap();
        dispatch(&conn, "upsert_score_rule", &json!({"rule": {
            "id": "drilling_rule", "label": "Drilled today", "kind": "checkin",
            "metric": "drilling", "weight": 10, "threshold": 1, "builtIn": false
        }})).unwrap();
        let rules = dispatch(&conn, "get_score_rules", &json!({})).unwrap();
        assert!(rules.as_array().unwrap().iter().any(|r| r["id"] == "drilling_rule"));
        dispatch(&conn, "delete_score_rule", &json!({"id": "drilling_rule"})).unwrap();
        dispatch(&conn, "reset_scoring_weights", &json!({})).unwrap();
        let rules = dispatch(&conn, "get_score_rules", &json!({})).unwrap();
        assert!(rules.as_array().unwrap().iter().any(|r| r["id"] == "main_goal" && r["weight"] == 30));

        // Streak definition lifecycle over the hub transport.
        dispatch(&conn, "add_streak_definition", &json!({
            "id": "drilling_streak", "name": "Drilling", "kind": "checkin",
            "metric": "drilling", "threshold": 0
        })).unwrap();
        let defs = dispatch(&conn, "get_streak_definitions", &json!({})).unwrap();
        assert!(defs.as_array().unwrap().iter().any(|d| d["id"] == "drilling_streak"));
        dispatch(&conn, "delete_streak_definition", &json!({"id": "drilling_streak"})).unwrap();
        let defs = dispatch(&conn, "get_streak_definitions", &json!({})).unwrap();
        assert!(defs.as_array().unwrap().iter().all(|d| d["id"] != "drilling_streak"));

        dispatch(&conn, "delete_project", &json!({"id": pid})).unwrap();
        let projects = dispatch(&conn, "get_projects", &json!({})).unwrap();
        assert!(projects.as_array().unwrap().is_empty());
    }
}
