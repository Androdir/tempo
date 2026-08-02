//! Local LLM classification via a locally-running Ollama server.
//!
//! Design: an LLM call takes seconds, so a background worker classifies today's
//! activity *blocks* and caches the result. Reads prefer the cached LLM verdict;
//! otherwise rule-based. Any failure / invalid JSON => rule-based fallback, the
//! error is logged locally, and the app never crashes.
//!
//! Privacy: the client refuses any non-local host, so nothing can reach a cloud.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::models::{LlmError, OllamaTestResult};
use crate::{projects, semantic, settings};

const WORKER_INTERVAL: Duration = Duration::from_secs(25);
const MAX_PER_CYCLE: usize = 4;

pub struct OllamaConfig {
    pub url: String,
    pub model: String,
}

#[derive(Clone, Serialize)]
pub struct LlmClassification {
    pub category: String,
    pub productive: bool,
    pub confidence: f64,
    pub project: Option<String>,
    pub reason: String,
}

pub struct ClassifyRequest {
    pub source: String,
    pub label: String,
    pub title: String,
    pub domain: Option<String>,
    pub summary: Option<String>,
    pub activity_kind: String,
    pub rule_category: String,
    pub rule_confidence: f64,
    pub matched_project: Option<String>,
    pub matched_project_confidence: u8,
    pub goals: Vec<String>,
    pub policies: Vec<semantic::ClassificationPolicy>,
    pub duration_seconds: i64,
    pub prev: Option<String>,
    pub next: Option<String>,
}

// ------------------------------------------------------------ no-cloud guard

fn host_of(url: &str) -> Option<String> {
    let after = url.split("://").nth(1).unwrap_or(url);
    let hostport = after.split('/').next()?;
    let host = hostport
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(hostport);
    Some(host.trim().to_string())
}

/// Only loopback / `.local` / Tailscale (`.ts.net` + CGNAT) / private LAN ranges are
/// allowed — never a public host, so the client can't reach a cloud LLM.
pub fn is_private_host(url: &str) -> bool {
    let host = match host_of(url) {
        Some(h) => h.to_ascii_lowercase(),
        None => return false,
    };
    if host == "localhost" || host == "::1" || host.ends_with(".local") || host.ends_with(".ts.net")
    {
        return true;
    }
    if host.starts_with("127.") || host.starts_with("10.") || host.starts_with("192.168.") {
        return true;
    }
    // 172.16.0.0 – 172.31.255.255 (private)
    if let Some(rest) = host.strip_prefix("172.") {
        if let Some(octet) = rest.split('.').next().and_then(|s| s.parse::<u8>().ok()) {
            if (16..=31).contains(&octet) {
                return true;
            }
        }
    }
    // 100.64.0.0 – 100.127.255.255 (CGNAT — Tailscale). The rest of 100.x is public.
    if let Some(rest) = host.strip_prefix("100.") {
        if let Some(octet) = rest.split('.').next().and_then(|s| s.parse::<u8>().ok()) {
            if (64..=127).contains(&octet) {
                return true;
            }
        }
    }
    false
}

// ------------------------------------------------------------------- prompt

pub fn build_prompt(req: &ClassifyRequest) -> String {
    let none = "none".to_string();
    let mut s = String::new();
    s.push_str("You classify a user's computer activity into exactly one category.\n");
    s.push_str(
        "Return ONLY a JSON object with keys: category, productive, confidence, project, reason.\n",
    );
    s.push_str(
        "category must be one of: productive, study, business, neutral, distraction, recovery.\n",
    );
    s.push_str("productive: boolean. confidence: 0.0-1.0. project: one of the user's projects or null. reason: <= 18 words.\n");
    s.push_str("Evidence rules:\n");
    s.push_str("- Never choose a project from goals or neighboring activity. Use only the matched project below; if it is none, project must be null.\n");
    s.push_str("- A goal is not evidence that the current app belongs to that goal.\n");
    s.push_str("- Tempo itself is neutral tracking/admin time and never study or project work.\n");
    s.push_str("- If evidence is weak, choose neutral and keep confidence at or below 0.55.\n\n");

    if !req.policies.is_empty() {
        s.push_str("User classification policies (apply when their terms match the activity):\n");
        for policy in req.policies.iter().filter(|policy| policy.enabled) {
            s.push_str(&format!(
                "- {} => {} when matching: {}\n",
                policy.name,
                policy.category,
                policy.terms.join(", ")
            ));
        }
        s.push('\n');
    }
    if !req.goals.is_empty() {
        s.push_str("User's projects / daily goals:\n");
        for g in &req.goals {
            s.push_str(&format!("- {g}\n"));
        }
        s.push('\n');
    }

    s.push_str("Activity:\n");
    s.push_str(&format!("- source: {}\n", req.source));
    s.push_str(&format!("- app/site: {}\n", req.label));
    s.push_str(&format!("- title: {}\n", req.title));
    if let Some(d) = &req.domain {
        s.push_str(&format!("- domain: {d}\n"));
    }
    if let Some(sum) = &req.summary {
        if !sum.trim().is_empty() {
            s.push_str(&format!("- text summary: {}\n", truncate(sum, 400)));
        }
    }
    s.push_str(&format!(
        "- detected activity type: {}\n",
        req.activity_kind
    ));
    s.push_str(&format!(
        "- rule category: {} ({:.0}% confidence)\n",
        req.rule_category,
        req.rule_confidence * 100.0
    ));
    s.push_str(&format!(
        "- matched project (rule-based): {} ({}% confidence)\n",
        req.matched_project.as_ref().unwrap_or(&none),
        req.matched_project_confidence,
    ));
    s.push_str(&format!("- duration: {}s\n", req.duration_seconds));
    s.push_str(&format!(
        "- previous activity: {}\n",
        req.prev.as_ref().unwrap_or(&none)
    ));
    s.push_str(&format!(
        "- next activity: {}\n",
        req.next.as_ref().unwrap_or(&none)
    ));
    s.push_str("\nJSON:");
    s
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

// ----------------------------------------------------- strict JSON handling

#[derive(Deserialize)]
struct Raw {
    category: Option<String>,
    productive: Option<bool>,
    confidence: Option<f64>,
    project: Option<serde_json::Value>,
    reason: Option<String>,
}

/// Parse + validate the model's JSON into a clean result, or Err.
pub fn parse_and_validate(s: &str) -> Result<LlmClassification, String> {
    let raw: Raw = serde_json::from_str(s.trim()).map_err(|e| format!("invalid JSON: {e}"))?;

    let category = raw.category.unwrap_or_default().trim().to_ascii_lowercase();
    if !crate::models::is_valid_category(&category) {
        return Err(format!("invalid category: {category:?}"));
    }

    let confidence = raw.confidence.unwrap_or(0.5).clamp(0.0, 1.0);
    let productive = raw
        .productive
        .unwrap_or_else(|| crate::models::bucket_for(&category) == "productive");
    let project = match raw.project {
        Some(serde_json::Value::String(p)) => {
            let t = p.trim();
            if t.is_empty() || t.eq_ignore_ascii_case("null") || t.eq_ignore_ascii_case("none") {
                None
            } else {
                Some(t.to_string())
            }
        }
        _ => None,
    };
    let reason: String = raw
        .reason
        .unwrap_or_default()
        .trim()
        .chars()
        .take(200)
        .collect();

    Ok(LlmClassification {
        category,
        productive,
        confidence,
        project,
        reason,
    })
}

fn normalized_app(label: &str) -> String {
    label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Enforce evidence boundaries after model output. LLM confidence is not proof:
/// project attribution must agree with the deterministic matcher, and sparse
/// desktop context cannot turn Tempo into unrelated productive work.
pub fn apply_evidence_guardrails(
    req: &ClassifyRequest,
    mut c: LlmClassification,
) -> LlmClassification {
    let model_project = c.project.take();
    c.project = match (&req.matched_project, model_project) {
        (Some(expected), Some(actual))
            if req.matched_project_confidence >= projects::OVERRIDE_THRESHOLD
                && expected.eq_ignore_ascii_case(actual.trim()) =>
        {
            Some(expected.clone())
        }
        _ => None,
    };

    let app = normalized_app(&req.label);
    let has_visible_evidence = req.summary.as_deref().is_some_and(|s| !s.trim().is_empty());
    let is_tempo = matches!(
        app.as_str(),
        "tempo" | "tempoexe" | "productivitytracker" | "productivitytrackerexe"
    );

    if is_tempo {
        c.category = "neutral".to_string();
        c.confidence = 0.98;
        c.project = None;
        c.reason = "Tempo is tracking/admin time".to_string();
    } else if c.project.is_none() && req.rule_confidence < 0.75 {
        let evidence = format!(
            "{} {} {} {} {}",
            req.label,
            req.title,
            req.domain.as_deref().unwrap_or_default(),
            req.summary.as_deref().unwrap_or_default(),
            c.reason,
        );
        if let Some((policy, term)) =
            semantic::matched(&req.policies, &evidence, &req.activity_kind)
        {
            c.category = policy.category.clone();
            c.confidence = c.confidence.max(0.88);
            c.reason = format!("Policy: {} (matched ‘{}’)", policy.name, term);
        } else if !has_visible_evidence && req.rule_category == "uncategorized" {
            c.confidence = c.confidence.min(0.70);
        }
    } else if !has_visible_evidence && c.project.is_none() && req.rule_category == "uncategorized" {
        // App + title alone can be useful, but cannot justify near-certainty.
        c.confidence = c.confidence.min(0.70);
    }

    c.productive = crate::models::bucket_for(&c.category) == "productive";
    c
}

// --------------------------------------------------------------- ollama calls

fn classify_with_keep_alive(
    cfg: &OllamaConfig,
    req: &ClassifyRequest,
    keep_alive: &str,
) -> Result<LlmClassification, String> {
    if !is_private_host(&cfg.url) {
        return Err(format!("refusing non-local Ollama URL: {}", cfg.url));
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout_read(Duration::from_secs(45))
        .build();

    let body = serde_json::json!({
        "model": cfg.model,
        "prompt": build_prompt(req),
        "stream": false,
        "format": "json",
        "options": { "temperature": 0.1 },
        "keep_alive": keep_alive
    });

    let resp: serde_json::Value = agent
        .post(&format!("{}/api/generate", cfg.url.trim_end_matches('/')))
        .send_json(body)
        .map_err(|e| format!("ollama request failed: {e}"))?
        .into_json()
        .map_err(|e| format!("ollama response not JSON: {e}"))?;

    let response_str = resp
        .get("response")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "ollama response missing 'response' field".to_string())?;

    parse_and_validate(response_str)
}

/// Generic strict-JSON generation via Ollama. Returns the model's raw JSON
/// string (used by the daily review). Refuses non-local hosts.
pub fn generate_json(cfg: &OllamaConfig, prompt: &str, temperature: f64) -> Result<String, String> {
    if !is_private_host(&cfg.url) {
        return Err(format!("refusing non-local Ollama URL: {}", cfg.url));
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout_read(Duration::from_secs(60))
        .build();

    let body = serde_json::json!({
        "model": cfg.model,
        "prompt": prompt,
        "stream": false,
        "format": "json",
        "options": { "temperature": temperature },
        "keep_alive": 0
    });

    let resp: serde_json::Value = agent
        .post(&format!("{}/api/generate", cfg.url.trim_end_matches('/')))
        .send_json(body)
        .map_err(|e| format!("ollama request failed: {e}"))?
        .into_json()
        .map_err(|e| format!("ollama response not JSON: {e}"))?;

    resp.get("response")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "ollama response missing 'response' field".to_string())
}
/// Release the model after Tempo's background batch so it does not reserve GPU
/// memory while games or other graphics-heavy applications are running.
fn unload_model(cfg: &OllamaConfig) -> Result<(), String> {
    if !is_private_host(&cfg.url) {
        return Err(format!("refusing non-local Ollama URL: {}", cfg.url));
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout_read(Duration::from_secs(15))
        .build();
    agent
        .post(&format!("{}/api/generate", cfg.url.trim_end_matches('/')))
        .send_json(serde_json::json!({
            "model": cfg.model,
            "stream": false,
            "keep_alive": 0
        }))
        .map_err(|error| format!("ollama unload failed: {error}"))?;
    Ok(())
}

pub fn test_connection(url: &str, model: &str) -> OllamaTestResult {
    if !is_private_host(url) {
        return OllamaTestResult {
            ok: false,
            message: format!("Blocked: '{url}' is not a local address. Use localhost or a LAN IP."),
            models: vec![],
            model_available: false,
        };
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout_read(Duration::from_secs(8))
        .build();

    match agent
        .get(&format!("{}/api/tags", url.trim_end_matches('/')))
        .call()
    {
        Ok(resp) => match resp.into_json::<serde_json::Value>() {
            Ok(v) => {
                let models: Vec<String> = v
                    .get("models")
                    .and_then(|m| m.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| {
                                m.get("name").and_then(|n| n.as_str()).map(String::from)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let model_available = models
                    .iter()
                    .any(|m| m == model || m.split(':').next() == Some(model));
                let message = if models.is_empty() {
                    "Connected, but no models are installed. Try: ollama pull llama3.1:8b"
                        .to_string()
                } else if model_available {
                    format!("Connected ✓  {} model(s) available.", models.len())
                } else {
                    format!("Connected, but '{model}' is not installed. Try: ollama pull {model}")
                };
                OllamaTestResult {
                    ok: true,
                    message,
                    models,
                    model_available,
                }
            }
            Err(e) => OllamaTestResult {
                ok: false,
                message: format!("Unexpected response from Ollama: {e}"),
                models: vec![],
                model_available: false,
            },
        },
        Err(e) => OllamaTestResult {
            ok: false,
            message: format!("Could not reach Ollama at {url}. Is it running? ({e})"),
            models: vec![],
            model_available: false,
        },
    }
}

// --------------------------------------------------------------- DB helpers

pub fn load_today_cache(conn: &Connection) -> HashMap<String, LlmClassification> {
    let day = chrono::Local::now().format("%Y-%m-%d").to_string();
    load_cache_for_day(conn, &day)
}

/// Same as `load_today_cache` but for an arbitrary local day (used by the
/// weekly review, which aggregates the past 7 days).
pub fn load_cache_for_day(conn: &Connection, day: &str) -> HashMap<String, LlmClassification> {
    let mut map = HashMap::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT block_key, category, productive, confidence, project, reason
         FROM llm_classification WHERE day = ?1",
    ) else {
        return map;
    };
    if let Ok(rows) = stmt.query_map([day], |r| {
        Ok((
            r.get::<_, String>(0)?,
            LlmClassification {
                category: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                productive: r.get::<_, Option<i64>>(2)?.unwrap_or(0) != 0,
                confidence: r.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
                project: r.get::<_, Option<String>>(4)?,
                reason: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
            },
        ))
    }) {
        for (k, v) in rows.flatten() {
            if crate::models::is_valid_category(&v.category) {
                map.insert(k, v);
            }
        }
    }
    map
}

fn store_classification(db: &Db, key: &str, day: &str, c: &LlmClassification, model: &str) {
    if let Ok(conn) = db.lock() {
        let _ = conn.execute(
            "INSERT INTO llm_classification
               (block_key, day, category, productive, confidence, project, reason, model, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(block_key) DO UPDATE SET
               category=excluded.category, productive=excluded.productive,
               confidence=excluded.confidence, project=excluded.project,
               reason=excluded.reason, model=excluded.model, created_at=excluded.created_at",
            params![
                key, day, c.category, c.productive as i64, c.confidence,
                c.project, c.reason, model, chrono::Utc::now().to_rfc3339()
            ],
        );
    }
}

pub fn log_error(db: &Db, context: Option<&str>, message: &str) {
    eprintln!("[tempo][llm] {}: {}", context.unwrap_or("-"), message);
    if let Ok(conn) = db.lock() {
        let _ = conn.execute(
            "INSERT INTO llm_errors (timestamp, context, message) VALUES (?1, ?2, ?3)",
            params![chrono::Utc::now().to_rfc3339(), context, message],
        );
        // Keep only the most recent 200 errors.
        let _ = conn.execute(
            "DELETE FROM llm_errors WHERE id NOT IN
               (SELECT id FROM llm_errors ORDER BY id DESC LIMIT 200)",
            [],
        );
    }
}

pub fn recent_errors(conn: &Connection) -> Vec<LlmError> {
    let mut out = Vec::new();
    if let Ok(mut stmt) =
        conn.prepare("SELECT timestamp, context, message FROM llm_errors ORDER BY id DESC LIMIT 20")
    {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok(LlmError {
                timestamp: r.get(0)?,
                context: r.get(1)?,
                message: r.get(2)?,
            })
        }) {
            for e in rows.flatten() {
                out.push(e);
            }
        }
    }
    out
}

pub fn last_error(conn: &Connection) -> Option<String> {
    conn.query_row(
        "SELECT message FROM llm_errors ORDER BY id DESC LIMIT 1",
        [],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

// ----------------------------------------------------------- background worker

struct WorkerConfig {
    enabled: bool,
    url: String,
    model: String,
}

fn read_config(db: &Db) -> Option<WorkerConfig> {
    let conn = db.lock().ok()?;
    Some(WorkerConfig {
        enabled: settings::get_bool(&conn, settings::LLM_ENABLED, false),
        url: settings::get_setting(&conn, settings::OLLAMA_URL)
            .unwrap_or_else(|| settings::DEFAULT_OLLAMA_URL.to_string()),
        model: settings::get_setting(&conn, settings::OLLAMA_MODEL)
            .unwrap_or_else(|| settings::DEFAULT_OLLAMA_MODEL.to_string()),
    })
}

fn load_today_keys(conn: &Connection, day: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    if let Ok(mut stmt) = conn.prepare("SELECT block_key FROM llm_classification WHERE day = ?1") {
        if let Ok(rows) = stmt.query_map([day], |r| r.get::<_, String>(0)) {
            for k in rows.flatten() {
                set.insert(k);
            }
        }
    }
    set
}

fn goal_list(conn: &Connection) -> Vec<String> {
    projects::list_projects(conn)
        .unwrap_or_default()
        .into_iter()
        .take(8)
        .map(|p| format!("{} ({})", p.name, p.category))
        .collect()
}

fn neighbor(b: &crate::aggregate::Block) -> String {
    let t = if b.title.is_empty() {
        b.label.clone()
    } else {
        b.title.clone()
    };
    format!("{} — {}", b.label, t)
}

fn load_manual_keys(conn: &Connection, day: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    if let Ok(mut stmt) = conn.prepare("SELECT block_key FROM manual_corrections WHERE day = ?1") {
        if let Ok(rows) = stmt.query_map([day], |r| r.get::<_, String>(0)) {
            for k in rows.flatten() {
                set.insert(k);
            }
        }
    }
    set
}

/// Treat a recently observed game as active even during a short alt-tab. This
/// prevents the classifier from starting GPU-heavy work while the game is
/// likely still running behind another foreground window.
fn recent_game_active(db: &Db) -> bool {
    let Ok(conn) = db.lock() else {
        return false;
    };
    let Ok(mut stmt) = conn.prepare(
        "SELECT timestamp, app_name, window_title, executable_path
         FROM activity_log
         WHERE is_idle = 0
         ORDER BY id DESC
         LIMIT 8",
    ) else {
        return false;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    }) else {
        return false;
    };
    let now = chrono::Utc::now();
    let active = rows
        .flatten()
        .any(|(timestamp, app, title, executable_path)| {
            let recent = chrono::DateTime::parse_from_rfc3339(&timestamp)
                .map(|seen| {
                    let age = now.signed_duration_since(seen.with_timezone(&chrono::Utc));
                    age >= chrono::Duration::seconds(-5) && age <= chrono::Duration::seconds(75)
                })
                .unwrap_or(false);
            recent
                && semantic::detect_activity_kind(
                    "desktop",
                    &app,
                    &title,
                    executable_path.as_deref(),
                    None,
                    None,
                    &[],
                ) == "game"
        });
    active
}

fn gather_work(db: &Db, max: usize) -> Result<Vec<(String, String, ClassifyRequest)>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let day = chrono::Local::now().format("%Y-%m-%d").to_string();
    let blocks = crate::aggregate::collect_blocks(&conn)?;
    let cached = load_today_keys(&conn, &day);
    let manual = load_manual_keys(&conn, &day);
    let goals = goal_list(&conn);
    let policies = semantic::load(&conn);

    let mut out = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        if out.len() >= max {
            break;
        }
        // Step 2: only consult the LLM for blocks the rules flagged for review,
        // and never for blocks the user already corrected.
        if !b.needs_llm {
            continue;
        }
        let key = b.block_key.clone();
        if cached.contains(&key) || manual.contains(&key) {
            continue;
        }
        let next = if i > 0 {
            Some(neighbor(&blocks[i - 1]))
        } else {
            None
        };
        let prev = blocks.get(i + 1).map(neighbor);
        out.push((
            key,
            day.clone(),
            ClassifyRequest {
                source: b.source.clone(),
                label: b.label.clone(),
                title: b.title.clone(),
                domain: b.domain.clone(),
                summary: b.summary.clone(),
                activity_kind: b.activity_kind.clone(),
                rule_category: b.rule_category.clone(),
                rule_confidence: b.confidence,
                matched_project: b
                    .rule_project
                    .clone()
                    .filter(|_| b.rule_project_confidence >= projects::OVERRIDE_THRESHOLD),
                matched_project_confidence: b.rule_project_confidence,
                goals: goals.clone(),
                policies: policies.clone(),
                duration_seconds: b.seconds,
                prev,
                next,
            },
        ));
    }
    Ok(out)
}

pub fn start(db: Db) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(WORKER_INTERVAL);

            let cfg = match read_config(&db) {
                Some(c) if c.enabled => c,
                _ => continue,
            };
            if !is_private_host(&cfg.url) {
                log_error(
                    &db,
                    None,
                    &format!("Refusing non-local Ollama URL: {}", cfg.url),
                );
                continue;
            }
            if recent_game_active(&db) {
                continue;
            }

            let work = match gather_work(&db, MAX_PER_CYCLE) {
                Ok(w) => w,
                Err(e) => {
                    log_error(&db, None, &e);
                    continue;
                }
            };
            let used_model = !work.is_empty();

            let oc = OllamaConfig {
                url: cfg.url.clone(),
                model: cfg.model.clone(),
            };
            for (key, day, req) in work {
                // On any error we simply don't cache => reads use rule-based.
                match classify_with_keep_alive(&oc, &req, "2m") {
                    Ok(c) => {
                        let guarded = apply_evidence_guardrails(&req, c);
                        store_classification(&db, &key, &day, &guarded, &cfg.model);
                    }
                    Err(e) => log_error(&db, Some(&key), &e),
                }
            }
            if used_model {
                let _ = unload_model(&oc);
            }
        }
    });
}

#[cfg(test)]
mod game_guard_tests {
    use super::*;
    fn activity_db(timestamp: String, executable_path: &str) -> Db {
        let conn = crate::db::test_conn();
        conn.execute(
            "INSERT INTO activity_log
               (timestamp, day, app_name, window_title, executable_path, duration_seconds, is_idle)
             VALUES (?1, '2026-08-02', 'Dead by Daylight', 'Dead by Daylight', ?2, 10, 0)",
            params![timestamp, executable_path],
        )
        .unwrap();
        std::sync::Arc::new(std::sync::Mutex::new(conn))
    }

    #[test]
    fn recent_game_activity_pauses_background_ai() {
        let db = activity_db(
            chrono::Utc::now().to_rfc3339(),
            r"C:\Games\Steam\steamapps\common\Dead by Daylight\DeadByDaylight.exe",
        );
        assert!(recent_game_active(&db));
    }

    #[test]
    fn stale_game_activity_does_not_pause_background_ai() {
        let db = activity_db(
            (chrono::Utc::now() - chrono::Duration::minutes(2)).to_rfc3339(),
            r"C:\Games\Steam\steamapps\common\Dead by Daylight\DeadByDaylight.exe",
        );
        assert!(!recent_game_active(&db));
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_json() {
        let c = parse_and_validate(
            r#"{"category":"study","productive":true,"confidence":0.92,"project":"Exam studying","reason":"algorithms practice"}"#,
        )
        .unwrap();
        assert_eq!(c.category, "study");
        assert!(c.productive);
        assert!((c.confidence - 0.92).abs() < 1e-9);
        assert_eq!(c.project.as_deref(), Some("Exam studying"));
    }

    fn request(
        label: &str,
        summary: Option<&str>,
        matched_project: Option<&str>,
    ) -> ClassifyRequest {
        ClassifyRequest {
            source: "app".into(),
            label: label.into(),
            title: label.into(),
            domain: None,
            summary: summary.map(str::to_string),
            activity_kind: semantic::detect_activity_kind(
                "app",
                label,
                label,
                None,
                None,
                summary,
                &[],
            ),
            rule_category: "uncategorized".into(),
            rule_confidence: 0.3,
            matched_project: matched_project.map(str::to_string),
            matched_project_confidence: if matched_project.is_some() { 85 } else { 0 },
            goals: vec!["Bible Reading (study)".into()],
            policies: semantic::defaults(),
            duration_seconds: 20,
            prev: None,
            next: None,
        }
    }

    #[test]
    fn rejects_llm_project_without_rule_evidence() {
        let parsed = parse_and_validate(
            r#"{"category":"study","productive":true,"confidence":0.95,"project":"Bible Reading","reason":"goal context"}"#,
        )
        .unwrap();
        let guarded = apply_evidence_guardrails(&request("Some App", None, None), parsed);
        assert!(guarded.project.is_none());
        assert_eq!(guarded.confidence, 0.70);
    }

    #[test]
    fn preserves_only_the_project_the_rules_matched() {
        let parsed = parse_and_validate(
            r#"{"category":"business","confidence":0.88,"project":"Content Creation","reason":"editing"}"#,
        )
        .unwrap();
        let guarded = apply_evidence_guardrails(
            &request("DaVinci Resolve", None, Some("Content Creation")),
            parsed,
        );
        assert_eq!(guarded.project.as_deref(), Some("Content Creation"));
    }

    #[test]
    fn tempo_cannot_become_study_or_inherit_a_goal() {
        let parsed = parse_and_validate(
            r#"{"category":"study","confidence":0.95,"project":"Bible Reading","reason":"goal context"}"#,
        )
        .unwrap();
        let guarded = apply_evidence_guardrails(&request("Tempo", None, None), parsed);
        assert_eq!(guarded.category, "neutral");
        assert!(guarded.project.is_none());
        assert!(!guarded.productive);
    }

    #[test]
    fn telegram_desktop_without_content_stays_distracting() {
        let parsed = parse_and_validate(
            r#"{"category":"productive","confidence":0.95,"project":null,"reason":"chat"}"#,
        )
        .unwrap();
        let guarded = apply_evidence_guardrails(&request("Telegram Desktop", None, None), parsed);
        assert_eq!(guarded.category, "distraction");
        assert!(!guarded.productive);
    }

    #[test]
    fn rejects_invalid_category() {
        assert!(parse_and_validate(r#"{"category":"gaming","confidence":0.5}"#).is_err());
    }

    #[test]
    fn rejects_non_json() {
        assert!(parse_and_validate("the answer is study").is_err());
    }

    #[test]
    fn normalizes_null_project_and_clamps_confidence() {
        let c = parse_and_validate(
            r#"{"category":"NEUTRAL","confidence":5,"project":"null","reason":"x"}"#,
        )
        .unwrap();
        assert_eq!(c.category, "neutral");
        assert_eq!(c.confidence, 1.0);
        assert!(c.project.is_none());
    }

    #[test]
    fn blocks_cloud_hosts() {
        assert!(is_private_host("http://localhost:11434"));
        assert!(is_private_host("http://127.0.0.1:11434"));
        assert!(is_private_host("http://192.168.1.50:11434"));
        assert!(is_private_host("http://10.0.0.5:11434"));
        assert!(is_private_host("http://172.16.4.4:11434"));
        assert!(!is_private_host("https://api.openai.com"));
        assert!(!is_private_host("http://8.8.8.8:11434"));
        assert!(!is_private_host("http://172.40.0.1:11434"));
        // Tailscale: CGNAT range + MagicDNS names are allowed; public 100.x is not.
        assert!(is_private_host("http://100.64.0.1:11434"));
        assert!(is_private_host("http://100.127.255.254:11434"));
        assert!(is_private_host("http://nas.tail9f2c.ts.net:11434"));
        assert!(!is_private_host("http://100.63.0.1:11434")); // just below CGNAT = public
        assert!(!is_private_host("http://100.200.0.1:11434")); // above CGNAT = public
    }

    #[test]
    fn recognized_gameplay_reason_is_forced_to_distraction() {
        let parsed = parse_and_validate(
            r#"{"category":"neutral","confidence":0.55,"project":null,"reason":"Short gameplay session without clear work-related context."}"#,
        ).unwrap();
        let guarded = apply_evidence_guardrails(&request("Unknown Game", None, None), parsed);
        assert_eq!(guarded.category, "distraction");
        assert!(guarded.reason.contains("Recognized gameplay"));
    }
}
