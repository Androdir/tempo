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
use tempo_core::models::{CheckinState, Goal, LockinPlan};
use tempo_core::{aggregate, projects, settings, streaks};

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
        let _ = projects::ensure_default_projects(&conn);
        let _ = streaks::ensure_defaults(&conn);
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
        "get_goals" => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, title, project, target_minutes, priority, completed, recurring
                     FROM goals WHERE day = ?1
                     ORDER BY CASE priority WHEN 'high' THEN 0 WHEN 'medium' THEN 1 ELSE 2 END, sort_order, id",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([&day], |r| {
                    Ok(Goal {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        project: r.get(2)?,
                        target_minutes: r.get(3)?,
                        priority: r.get(4)?,
                        completed: r.get::<_, i64>(5)? != 0,
                        recurring: r.get::<_, i64>(6)? != 0,
                    })
                })
                .map_err(|e| e.to_string())?;
            let goals: Vec<Goal> = rows.filter_map(Result::ok).collect();
            Ok(serde_json::to_value(goals).unwrap())
        }
        "get_checkins" => Ok(serde_json::to_value(checkin_state(conn, &day)).unwrap()),
        "set_checkin" => {
            const FIELDS: [&str; 7] = [
                "main_goal_completed", "videos_posted", "gym_logged", "wrestled", "studied",
                "edited_video", "analysed_content",
            ];
            let field = args.get("field").and_then(|v| v.as_str()).ok_or("missing field")?;
            if !FIELDS.contains(&field) {
                return Err(format!("unknown check-in: {field}"));
            }
            let raw = args.get("value").and_then(|v| v.as_i64()).unwrap_or(0);
            let value = if field == "videos_posted" { raw.clamp(0, 99) } else { (raw != 0) as i64 };
            conn.execute("INSERT OR IGNORE INTO daily_checkin (day) VALUES (?1)", params![day])
                .map_err(|e| e.to_string())?;
            conn.execute(&format!("UPDATE daily_checkin SET {field} = ?1 WHERE day = ?2"), params![value, day])
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "toggle_goal" => {
            let id = args.get("id").and_then(|v| v.as_i64()).ok_or("missing id")?;
            let completed = args.get("completed").and_then(|v| v.as_bool()).unwrap_or(false);
            conn.execute("UPDATE goals SET completed = ?1 WHERE id = ?2", params![completed as i64, id])
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
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

fn checkin_state(conn: &Connection, day: &str) -> CheckinState {
    conn.query_row(
        "SELECT videos_posted, gym_logged, wrestled, studied, edited_video, analysed_content
         FROM daily_checkin WHERE day = ?1",
        [day],
        |r| {
            Ok(CheckinState {
                videos_posted: r.get(0)?,
                gym_logged: r.get::<_, i64>(1)? != 0,
                wrestled: r.get::<_, i64>(2)? != 0,
                studied: r.get::<_, i64>(3)? != 0,
                edited_video: r.get::<_, i64>(4)? != 0,
                analysed_content: r.get::<_, i64>(5)? != 0,
            })
        },
    )
    .unwrap_or(CheckinState {
        videos_posted: 0,
        gym_logged: false,
        wrestled: false,
        studied: false,
        edited_video: false,
        analysed_content: false,
    })
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
        assert_eq!(c["videosPosted"], 3);
    }
}
