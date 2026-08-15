//! App settings (key/value), per-domain rules, and the built-in capture policy.

use rusqlite::{params, Connection};

use crate::models::{ConfigDto, DomainRule};

// Setting keys.
pub const CAPTURE_PAGE_CONTENT: &str = "capture_page_content";
pub const STORE_RAW_TEXT: &str = "store_raw_text";
pub const MAX_TEXT_LENGTH: &str = "max_text_length";
pub const DELETE_RAW_AFTER: &str = "delete_raw_after_classification";
pub const INGEST_TOKEN: &str = "ingest_token";
pub const INGEST_PORT: &str = "ingest_port";
pub const SMART_TRACKING_ENABLED: &str = "smart_tracking_enabled";
pub const SMART_INTERVAL: &str = "smart_interval_seconds";
pub const IDLE_THRESHOLD: &str = "idle_threshold_seconds";
pub const COUNT_MEDIA_ACTIVE: &str = "count_media_as_active";
pub const LLM_ENABLED: &str = "llm_enabled";
pub const LLM_PROVIDER: &str = "llm_provider";
pub const OLLAMA_URL: &str = "ollama_url";
pub const OLLAMA_MODEL: &str = "ollama_model";
pub const OPENAI_CLASSIFICATION_MODEL: &str = "openai_classification_model";
pub const OPENAI_REVIEW_MODEL: &str = "openai_review_model";
pub const OPENAI_INCLUDE_CONTENT: &str = "openai_include_content";
/// One-time release initialization; the OS remains the source of truth afterwards.
pub const LAUNCH_AT_LOGIN_INITIALIZED: &str = "launch_at_login_initialized";
pub const TRACKING_PAUSED_UNTIL: &str = "tracking_paused_until";
pub const TITLE_EXCLUDED_APPS: &str = "title_excluded_apps";

// Accountability.
pub const DISTRACTION_WARN_ENABLED: &str = "distraction_warn_enabled";
pub const DISTRACTION_WARN_MINUTES: &str = "distraction_warn_minutes";
pub const EOD_POPUP_ENABLED: &str = "eod_popup_enabled";
pub const EOD_POPUP_TIME: &str = "eod_popup_time";
/// Optional HH:MM planning target for the day's main goal. Empty means no deadline.
pub const MAIN_GOAL_DEADLINE: &str = "main_goal_deadline";
// Accountability runtime state (not user-configured).
pub const EOD_LAST_SHOWN: &str = "eod_last_shown";
pub const DISTRACTION_SNOOZE_UNTIL: &str = "distraction_snooze_until";
pub const DISTRACTION_MUTE_TARGET: &str = "distraction_mute_target";
pub const DISTRACTION_MUTE_UNTIL: &str = "distraction_mute_until";

// Data retention (days of raw activity to keep; 0 = keep forever).
pub const RETENTION_DAYS: &str = "retention_days";
pub const RECURRING_MATERIALIZED_DAY: &str = "recurring_materialized_day";

// Proof-of-output detection + lock-in plan.
pub const OUTPUT_WATCH_ENABLED: &str = "output_watch_enabled";
pub const LOCKIN_AUTO_ENABLED: &str = "lockin_auto_enabled";

pub const DEFAULT_DISTRACTION_MINUTES: i64 = 20;
pub const DEFAULT_EOD_TIME: &str = "21:00";
pub const DEFAULT_RETENTION_DAYS: i64 = 90;

pub const DEFAULT_SMART_INTERVAL: i64 = 60;
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
pub const DEFAULT_OLLAMA_MODEL: &str = "llama3.1:8b";
pub const DEFAULT_OPENAI_CLASSIFICATION_MODEL: &str = "gpt-5.4-nano";
pub const DEFAULT_OPENAI_REVIEW_MODEL: &str = "gpt-5.4-mini";

pub const DEFAULT_PORT: u16 = 48710;
pub const DEFAULT_MAX_TEXT_LENGTH: i64 = 8000;
pub const SAMPLE_SECONDS: i64 = 10;
pub const IDLE_SECONDS: i64 = 60;

// ----------------------------------------------------------------- key/value

pub fn get_setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        [key],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn get_bool(conn: &Connection, key: &str, default: bool) -> bool {
    match get_setting(conn, key) {
        Some(v) => v == "1" || v.eq_ignore_ascii_case("true"),
        None => default,
    }
}

pub fn get_int(conn: &Connection, key: &str, default: i64) -> i64 {
    get_setting(conn, key)
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(default)
}

pub fn tracking_paused_until(conn: &Connection) -> Option<String> {
    let value = get_setting(conn, TRACKING_PAUSED_UNTIL)?;
    if value == "indefinite" {
        return Some(value);
    }
    let future = chrono::DateTime::parse_from_rfc3339(&value)
        .map(|until| until.with_timezone(&chrono::Utc) > chrono::Utc::now())
        .unwrap_or(false);
    if future {
        Some(value)
    } else {
        let _ = conn.execute(
            "DELETE FROM app_settings WHERE key = ?1",
            [TRACKING_PAUSED_UNTIL],
        );
        None
    }
}

pub fn set_tracking_paused_until(conn: &Connection, until: Option<&str>) -> rusqlite::Result<()> {
    match until {
        Some(value) => set_setting(conn, TRACKING_PAUSED_UNTIL, value),
        None => {
            conn.execute(
                "DELETE FROM app_settings WHERE key = ?1",
                [TRACKING_PAUSED_UNTIL],
            )?;
            Ok(())
        }
    }
}
pub fn title_excluded_apps(conn: &Connection) -> Vec<String> {
    get_setting(conn, TITLE_EXCLUDED_APPS)
        .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

pub fn title_capture_allowed(conn: &Connection, app_name: &str) -> bool {
    let app = app_name.trim().to_ascii_lowercase();
    !title_excluded_apps(conn).iter().any(|rule| {
        let rule = rule.to_ascii_lowercase();
        rule.len() >= 2 && (app == rule || app.contains(&rule))
    })
}

// ------------------------------------------------------------------ defaults

/// Seed defaults the first time only (won't clobber user changes on restart).
pub fn ensure_defaults(conn: &Connection) -> rusqlite::Result<()> {
    set_if_absent(conn, CAPTURE_PAGE_CONTENT, "0")?; // OFF by default
    set_if_absent(conn, STORE_RAW_TEXT, "0")?; // OFF by default
    set_if_absent(conn, MAX_TEXT_LENGTH, &DEFAULT_MAX_TEXT_LENGTH.to_string())?;
    set_if_absent(conn, DELETE_RAW_AFTER, "0")?;
    set_if_absent(conn, SMART_TRACKING_ENABLED, "0")?; // OFF by default
    set_if_absent(conn, TITLE_EXCLUDED_APPS, "[]")?;
    set_if_absent(conn, SMART_INTERVAL, &DEFAULT_SMART_INTERVAL.to_string())?;
    set_if_absent(conn, LLM_ENABLED, "0")?; // OFF by default
    set_if_absent(conn, LLM_PROVIDER, "ollama")?;
    set_if_absent(conn, OLLAMA_URL, DEFAULT_OLLAMA_URL)?;
    set_if_absent(conn, OLLAMA_MODEL, DEFAULT_OLLAMA_MODEL)?;
    set_if_absent(
        conn,
        OPENAI_CLASSIFICATION_MODEL,
        DEFAULT_OPENAI_CLASSIFICATION_MODEL,
    )?;
    set_if_absent(conn, OPENAI_REVIEW_MODEL, DEFAULT_OPENAI_REVIEW_MODEL)?;
    set_if_absent(conn, OPENAI_INCLUDE_CONTENT, "0")?; // private content stays local by default
    set_if_absent(conn, DISTRACTION_WARN_ENABLED, "1")?; // ON by default
    set_if_absent(
        conn,
        DISTRACTION_WARN_MINUTES,
        &DEFAULT_DISTRACTION_MINUTES.to_string(),
    )?;
    set_if_absent(conn, EOD_POPUP_ENABLED, "0")?; // OFF by default
    set_if_absent(conn, EOD_POPUP_TIME, DEFAULT_EOD_TIME)?;
    set_if_absent(conn, MAIN_GOAL_DEADLINE, "")?;
    set_if_absent(conn, RETENTION_DAYS, &DEFAULT_RETENTION_DAYS.to_string())?;
    set_if_absent(conn, OUTPUT_WATCH_ENABLED, "1")?;
    set_if_absent(conn, LOCKIN_AUTO_ENABLED, "1")?;
    set_if_absent(conn, INGEST_PORT, &DEFAULT_PORT.to_string())?;
    if get_setting(conn, INGEST_TOKEN).is_none() {
        set_setting(conn, INGEST_TOKEN, &generate_token())?;
    }

    // (domain, category, capture_mode)
    let seeds: &[(&str, Option<&str>, &str)] = &[
        ("instagram.com", Some("distraction"), "meta"),
        ("youtube.com", Some("neutral"), "meta"),
        ("chatgpt.com", Some("neutral"), "text"),
        ("remnote.com", Some("study"), "text"),
        ("github.com", Some("productive"), "text"),
        // Sensitive defaults: never capture content.
        ("mail.google.com", None, "never"),
        ("outlook.live.com", None, "never"),
        ("outlook.office.com", None, "never"),
        ("mail.proton.me", None, "never"),
        ("chase.com", None, "never"),
        ("bankofamerica.com", None, "never"),
        ("wellsfargo.com", None, "never"),
        ("paypal.com", None, "never"),
        ("stripe.com", None, "never"),
        ("1password.com", None, "never"),
        ("lastpass.com", None, "never"),
        ("bitwarden.com", None, "never"),
    ];
    for (domain, category, mode) in seeds {
        conn.execute(
            "INSERT OR IGNORE INTO domain_rules (domain, category, capture_mode, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![domain, category, mode, now_rfc3339()],
        )?;
    }
    Ok(())
}

fn set_if_absent(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    if get_setting(conn, key).is_none() {
        set_setting(conn, key, value)?;
    }
    Ok(())
}

// --------------------------------------------------------------- domain rules

fn row_to_domain_rule(r: &rusqlite::Row) -> rusqlite::Result<DomainRule> {
    Ok(DomainRule {
        domain: r.get(0)?,
        category: r.get(1)?,
        capture_mode: r.get(2)?,
        ai_review: r.get::<_, i64>(3)? != 0,
    })
}

pub fn list_domain_rules(conn: &Connection) -> rusqlite::Result<Vec<DomainRule>> {
    let mut stmt = conn.prepare(
        "SELECT domain, category, capture_mode, ai_review FROM domain_rules ORDER BY domain",
    )?;
    let rows = stmt.query_map([], row_to_domain_rule)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn get_domain_rule(conn: &Connection, domain: &str) -> Option<DomainRule> {
    conn.query_row(
        "SELECT domain, category, capture_mode, ai_review FROM domain_rules WHERE domain = ?1",
        [domain],
        row_to_domain_rule,
    )
    .ok()
}

pub fn upsert_domain_rule(
    conn: &Connection,
    domain: &str,
    category: Option<&str>,
    capture_mode: &str,
    ai_review: bool,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO domain_rules (domain, category, capture_mode, ai_review, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(domain) DO UPDATE SET
             category = excluded.category,
             capture_mode = excluded.capture_mode,
             ai_review = excluded.ai_review,
             updated_at = excluded.updated_at",
        params![
            domain,
            category,
            capture_mode,
            ai_review as i64,
            now_rfc3339()
        ],
    )?;
    Ok(())
}

pub fn delete_domain_rule(conn: &Connection, domain: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM domain_rules WHERE domain = ?1",
        params![domain],
    )?;
    Ok(())
}

// ------------------------------------------------------------- capture policy

/// Built-in "never capture content" heuristics, applied on top of the rules.
/// Errs on the side of NOT capturing for anything bank/pay/email/health/secrets.
pub fn is_builtin_blocked(domain: &str) -> bool {
    let d = domain.to_ascii_lowercase();
    if d.ends_with(".gov") || d.ends_with(".bank") || d.ends_with(".gov.uk") {
        return true;
    }
    const NEEDLES: &[&str] = &[
        "bank",
        "paypal",
        "stripe",
        "venmo",
        "wallet",
        "chase",
        "wellsfargo",
        "citibank",
        "barclays",
        "hsbc",
        "santander",
        "coinbase",
        "1password",
        "lastpass",
        "bitwarden",
        "dashlane",
        "keeper",
        "mychart",
        "patient",
        "medicare",
        "medicaid",
        "nhs",
        "healthcare",
    ];
    if NEEDLES.iter().any(|n| d.contains(n)) {
        return true;
    }
    // Common webmail hosts.
    const MAIL: &[&str] = &[
        "mail.google.com",
        "outlook.live.com",
        "outlook.office.com",
        "mail.proton.me",
        "mail.yahoo.com",
        "mail.aol.com",
    ];
    MAIL.contains(&d.as_str())
}

/// Effective capture mode for a domain ("text" | "meta" | "never").
pub fn effective_capture_mode(conn: &Connection, domain: &str) -> String {
    if is_builtin_blocked(domain) {
        return "never".to_string();
    }
    match get_domain_rule(conn, domain) {
        Some(rule) => rule.capture_mode,
        None => "meta".to_string(),
    }
}

// --------------------------------------------------------------------- config

pub fn build_config(conn: &Connection) -> rusqlite::Result<ConfigDto> {
    Ok(ConfigDto {
        capture_page_content: get_bool(conn, CAPTURE_PAGE_CONTENT, false),
        store_raw_text: get_bool(conn, STORE_RAW_TEXT, false),
        max_text_length: get_int(conn, MAX_TEXT_LENGTH, DEFAULT_MAX_TEXT_LENGTH),
        sample_seconds: SAMPLE_SECONDS,
        idle_seconds: IDLE_SECONDS,
        domain_rules: list_domain_rules(conn)?,
    })
}

pub fn ingest_token(conn: &Connection) -> String {
    get_setting(conn, INGEST_TOKEN).unwrap_or_default()
}

pub fn ingest_port(conn: &Connection) -> u16 {
    get_int(conn, INGEST_PORT, DEFAULT_PORT as i64) as u16
}

// ------------------------------------------------------------------- helpers

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Non-cryptographic token, good enough to gate a loopback-only endpoint.
fn generate_token() -> String {
    use std::hash::{Hash, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};

    let mut h1 = std::collections::hash_map::DefaultHasher::new();
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        .hash(&mut h1);
    std::process::id().hash(&mut h1);
    let a = h1.finish();

    let mut h2 = std::collections::hash_map::DefaultHasher::new();
    a.hash(&mut h2);
    "tempo-ingest".hash(&mut h2);
    let b = h2.finish();

    format!("{a:016x}{b:016x}")
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_app_rule_hides_partial_app_name_match() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )
        .unwrap();
        set_setting(&conn, TITLE_EXCLUDED_APPS, r#"["Bitwarden","1Password"]"#).unwrap();
        assert!(!title_capture_allowed(&conn, "Bitwarden Desktop"));
        assert!(!title_capture_allowed(&conn, "1Password"));
        assert!(title_capture_allowed(&conn, "DaVinci Resolve"));
    }
}
