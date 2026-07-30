//! Accountability watcher: distraction warnings, focus-mode violations, and the
//! end-of-day review prompt. Runs as a background thread that reads the latest
//! activity, runs a small state machine, and emits Tauri events to the UI.
//!
//! Enforcement is deliberately *soft* — we never block apps at the OS level.
//! All detection is local; nothing leaves the device.

use crate::db::Db;
use crate::{aggregate, models, settings};
use chrono::{DateTime, Local, Utc};
use rusqlite::Connection;
use serde::Serialize;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

const TICK: Duration = Duration::from_secs(10);
/// A sample older than this means we don't know the current foreground → reset.
const FRESH_SECS: i64 = 30;
/// Minimum gap between focus-violation nudges, so they don't spam.
const FOCUS_WARN_COOLDOWN_SECS: i64 = 60;

/// Send a real OS notification. The caller still emits an in-app event so the
/// open window can offer richer actions such as Snooze and "It's intentional".
pub fn send_native_notification(app: &AppHandle, title: &str, body: &str) -> Result<(), String> {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}

// --------------------------------------------------------------- event payloads

#[derive(Serialize, Clone)]
struct DistractionPayload {
    label: String,
    key: String,
    minutes: i64,
    message: String,
}

#[derive(Serialize, Clone)]
struct FocusViolationPayload {
    label: String,
    goal: Option<String>,
    message: String,
}

// ------------------------------------------------------------- state machine

/// What the user is currently doing, distilled for the streak machine.
#[derive(Clone, Debug, PartialEq)]
struct Observation {
    key: String,    // identity of the target (lowercased app or domain)
    label: String,  // pretty label for messages
    bucket: String, // productive | neutral | distracting
    idle: bool,
    elapsed: i64, // seconds since the previous observation
}

#[derive(Clone, Debug)]
struct FocusCtx {
    goal: Option<String>,
    allowed: Vec<String>,
    blocked: Vec<String>,
}

#[derive(Default)]
struct Watch {
    cur_key: Option<String>,
    streak: i64,
    last_warn: i64,
    focus_cooldown: i64,
}

#[derive(Debug, PartialEq)]
enum Alert {
    Distraction {
        label: String,
        key: String,
        minutes: i64,
    },
    FocusViolation {
        label: String,
    },
}

/// Pure streak / violation state machine. Returns the alerts to emit this tick.
fn observe(
    w: &mut Watch,
    obs: Option<&Observation>,
    warn_enabled: bool,
    warn_minutes: i64,
    focus: Option<&FocusCtx>,
) -> Vec<Alert> {
    let mut alerts = Vec::new();

    if let Some(o) = obs {
        if w.focus_cooldown > 0 {
            w.focus_cooldown = (w.focus_cooldown - o.elapsed).max(0);
        }
    }

    match obs {
        Some(o) if !o.idle && o.bucket == "distracting" => {
            if w.cur_key.as_deref() == Some(o.key.as_str()) {
                w.streak += o.elapsed;
            } else {
                w.cur_key = Some(o.key.clone());
                w.streak = o.elapsed;
                w.last_warn = 0;
            }
            let threshold = warn_minutes.max(1) * 60;
            if warn_enabled && w.streak >= threshold && (w.streak - w.last_warn) >= threshold {
                w.last_warn = w.streak;
                alerts.push(Alert::Distraction {
                    label: o.label.clone(),
                    key: o.key.clone(),
                    minutes: w.streak / 60,
                });
            }
        }
        _ => {
            // idle / productive / unknown → reset the distraction streak
            w.cur_key = None;
            w.streak = 0;
            w.last_warn = 0;
        }
    }

    // Focus-mode violation during an active session.
    if let (Some(f), Some(o)) = (focus, obs) {
        if !o.idle && is_violation(f, o) && w.focus_cooldown == 0 {
            w.focus_cooldown = FOCUS_WARN_COOLDOWN_SECS;
            alerts.push(Alert::FocusViolation {
                label: o.label.clone(),
            });
        }
    }

    alerts
}

fn matches_list(list: &[String], key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    list.iter().any(|item| {
        let it = item.trim().to_ascii_lowercase();
        !it.is_empty() && (k == it || k.contains(&it) || it.contains(&k))
    })
}

fn is_violation(f: &FocusCtx, o: &Observation) -> bool {
    if matches_list(&f.blocked, &o.key) {
        return true;
    }
    // With an allowlist, any non-allowed *distracting* target is a violation.
    !f.allowed.is_empty() && !matches_list(&f.allowed, &o.key) && o.bucket == "distracting"
}

// --------------------------------------------------------------- DB helpers

fn parse_json_array(s: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(s).unwrap_or_default()
}

fn is_fresh(ts: &str) -> bool {
    DateTime::parse_from_rfc3339(ts)
        .map(|t| (Utc::now() - t.with_timezone(&Utc)).num_seconds() <= FRESH_SECS)
        .unwrap_or(false)
}

fn is_browser(app: &str) -> bool {
    // Match whole tokens, not substrings, so "Monarch"/"Search" aren't treated
    // as browsers just because they contain "arc"/... .
    app.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|tok| {
            matches!(
                tok,
                "chrome"
                    | "msedge"
                    | "edge"
                    | "firefox"
                    | "brave"
                    | "opera"
                    | "arc"
                    | "vivaldi"
                    | "chromium"
                    | "safari"
            )
        })
}

/// Pretty, human label for a domain or process name ("instagram.com" → "Instagram").
fn pretty_label(s: &str) -> String {
    let mut t = s.trim().to_string();
    if let Some(stripped) = t.strip_suffix(".exe") {
        t = stripped.to_string();
    }
    t = t.trim_start_matches("www.").to_string();
    if t.contains('.') {
        let parts: Vec<&str> = t.split('.').filter(|p| !p.is_empty()).collect();
        if parts.len() >= 2 {
            t = parts[parts.len() - 2].to_string();
        } else if let Some(first) = parts.first() {
            t = first.to_string();
        }
    }
    let mut chars = t.chars();
    match chars.next() {
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
        None => s.to_string(),
    }
}

fn lookup_category(
    conn: &Connection,
    key: &str,
    is_web: bool,
    title: &str,
    executable_path: Option<&str>,
    content_type: Option<&str>,
    summary: Option<&str>,
    keywords: &[String],
) -> String {
    let day = Local::now().format("%Y-%m-%d").to_string();
    aggregate::resolve_activity(
        conn,
        &day,
        if is_web { "web" } else { "app" },
        key,
        title,
        executable_path,
        is_web.then_some(key),
        content_type,
        summary,
        keywords,
    )
    .map(|verdict| verdict.category)
    .unwrap_or_else(|_| "uncategorized".into())
}

fn latest_browser(
    conn: &Connection,
) -> Option<(
    String,
    String,
    Option<String>,
    Option<String>,
    Vec<String>,
    bool,
    String,
)> {
    conn.query_row(
        "SELECT domain, page_title, content_type, content_summary, detected_keywords, is_idle, timestamp
         FROM browser_activity ORDER BY id DESC LIMIT 1",
        [],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                aggregate::parse_keywords(r.get::<_, Option<String>>(4)?),
                r.get::<_, i64>(5)? != 0,
                r.get::<_, String>(6)?,
            ))
        },
    )
    .ok()
}

fn current_observation(conn: &Connection, elapsed: i64) -> Option<Observation> {
    let (app_name, title, executable_path, idle, ts) = conn
        .query_row(
            "SELECT app_name, window_title, executable_path, is_idle, timestamp
             FROM activity_log ORDER BY id DESC LIMIT 1",
            [],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, i64>(3)? != 0,
                    r.get::<_, String>(4)?,
                ))
            },
        )
        .ok()?;
    if !is_fresh(&ts) {
        return None;
    }

    if is_browser(&app_name) {
        if let Some((domain, page_title, content_type, summary, keywords, b_idle, b_ts)) =
            latest_browser(conn)
        {
            if is_fresh(&b_ts) {
                let cat = lookup_category(
                    conn,
                    &domain,
                    true,
                    &page_title,
                    None,
                    content_type.as_deref(),
                    summary.as_deref(),
                    &keywords,
                );
                return Some(Observation {
                    key: domain.to_ascii_lowercase(),
                    label: pretty_label(&domain),
                    bucket: models::bucket_for(&cat).to_string(),
                    idle: idle || b_idle,
                    elapsed,
                });
            }
        }
    }

    let cat = lookup_category(
        conn,
        &app_name,
        false,
        &title,
        executable_path.as_deref(),
        None,
        None,
        &[],
    );
    Some(Observation {
        key: app_name.to_ascii_lowercase(),
        label: pretty_label(&app_name),
        bucket: models::bucket_for(&cat).to_string(),
        idle,
        elapsed,
    })
}

fn active_focus(conn: &Connection) -> Option<FocusCtx> {
    let (goal, allowed, blocked, ends_at) = conn
        .query_row(
            "SELECT goal, allowed, blocked, ends_at FROM focus_sessions
             WHERE status = 'active' ORDER BY id DESC LIMIT 1",
            [],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .ok()?;
    if let Ok(end) = DateTime::parse_from_rfc3339(&ends_at) {
        if Utc::now() > end.with_timezone(&Utc) {
            return None; // session window has elapsed
        }
    }
    Some(FocusCtx {
        goal,
        allowed: parse_json_array(&allowed),
        blocked: parse_json_array(&blocked),
    })
}

/// Returns true once when the configured end-of-day time is reached (per day).
fn check_eod(conn: &Connection) -> bool {
    if !settings::get_bool(conn, settings::EOD_POPUP_ENABLED, false) {
        return false;
    }
    let time = settings::get_setting(conn, settings::EOD_POPUP_TIME)
        .unwrap_or_else(|| settings::DEFAULT_EOD_TIME.to_string());
    let now = Local::now();
    let today = now.format("%Y-%m-%d").to_string();
    // Persisted so the popup doesn't re-fire if the app restarts after EOD time.
    if settings::get_setting(conn, settings::EOD_LAST_SHOWN).as_deref() == Some(today.as_str()) {
        return false;
    }
    if now.format("%H:%M").to_string().as_str() >= time.as_str() {
        let _ = settings::set_setting(conn, settings::EOD_LAST_SHOWN, &today);
        return true;
    }
    false
}

fn parse_ts(s: Option<String>) -> Option<DateTime<Utc>> {
    s.and_then(|v| DateTime::parse_from_rfc3339(&v).ok())
        .map(|t| t.with_timezone(&Utc))
}

/// Drop distraction alerts that are currently snoozed (all) or muted for their
/// specific target, per the "Snooze" / "It's intentional" toast actions.
fn apply_suppression(conn: &Connection, alerts: &mut Vec<Alert>) {
    let now = Utc::now();
    let snoozed = parse_ts(settings::get_setting(
        conn,
        settings::DISTRACTION_SNOOZE_UNTIL,
    ))
    .is_some_and(|t| now < t);
    let mute_target = settings::get_setting(conn, settings::DISTRACTION_MUTE_TARGET);
    let mute_until = parse_ts(settings::get_setting(
        conn,
        settings::DISTRACTION_MUTE_UNTIL,
    ));
    alerts.retain(|a| match a {
        Alert::Distraction { key, .. } => {
            if snoozed {
                return false;
            }
            if let (Some(t), Some(u)) = (&mute_target, mute_until) {
                if now < u && t.eq_ignore_ascii_case(key) {
                    return false;
                }
            }
            true
        }
        _ => true,
    });
}

// ------------------------------------------------------------------ worker

pub fn start(db: Db, app: AppHandle) {
    std::thread::spawn(move || {
        let mut watch = Watch::default();
        let mut last = Instant::now();

        loop {
            std::thread::sleep(TICK);
            // Clamp so a sleep/resume gap can't add hours to a streak in one tick
            // (the freshness check usually resets it anyway).
            let elapsed = last.elapsed().as_secs().clamp(1, FRESH_SECS as u64) as i64;
            last = Instant::now();

            let (alerts, focus_goal, eod_due) = {
                let Ok(conn) = db.lock() else { continue };
                let warn_enabled =
                    settings::get_bool(&conn, settings::DISTRACTION_WARN_ENABLED, true);
                let warn_minutes = settings::get_int(
                    &conn,
                    settings::DISTRACTION_WARN_MINUTES,
                    settings::DEFAULT_DISTRACTION_MINUTES,
                );
                let focus = active_focus(&conn);
                let obs = current_observation(&conn, elapsed);
                let mut alerts = observe(
                    &mut watch,
                    obs.as_ref(),
                    warn_enabled,
                    warn_minutes,
                    focus.as_ref(),
                );
                apply_suppression(&conn, &mut alerts);
                let eod_due = check_eod(&conn);
                (alerts, focus.and_then(|f| f.goal), eod_due)
            }; // lock released here, before emitting

            for a in alerts {
                match a {
                    Alert::Distraction {
                        label,
                        key,
                        minutes,
                    } => {
                        let message = format!(
                            "You've been on {label} for {minutes} minutes. Still intentional?"
                        );
                        let _ =
                            send_native_notification(&app, "Tempo · Distraction check", &message);
                        let _ = app.emit(
                            "distraction-warning",
                            DistractionPayload {
                                label,
                                key,
                                minutes,
                                message,
                            },
                        );
                    }
                    Alert::FocusViolation { label } => {
                        let message =
                            format!("{label} isn't part of your focus session. Back to it?");
                        let _ = send_native_notification(&app, "Tempo · Focus mode", &message);
                        let _ = app.emit(
                            "focus-violation",
                            FocusViolationPayload {
                                label,
                                goal: focus_goal.clone(),
                                message,
                            },
                        );
                    }
                }
            }
            if eod_due {
                let _ = send_native_notification(
                    &app,
                    "Tempo · End-of-day review",
                    "Your daily review is ready. Open Tempo to see it.",
                );
                let _ = app.emit("daily-review-due", ());
            }
        }
    });
}

// ------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(key: &str, bucket: &str, idle: bool, elapsed: i64) -> Observation {
        Observation {
            key: key.into(),
            label: key.into(),
            bucket: bucket.into(),
            idle,
            elapsed,
        }
    }

    #[test]
    fn warns_after_threshold_and_again_at_double() {
        let mut w = Watch::default();
        // 20-minute threshold; feed 60s ticks.
        let mut warned_minutes = Vec::new();
        for _ in 0..40 {
            let a = observe(
                &mut w,
                Some(&obs("instagram.com", "distracting", false, 60)),
                true,
                20,
                None,
            );
            for alert in a {
                if let Alert::Distraction { minutes, .. } = alert {
                    warned_minutes.push(minutes);
                }
            }
        }
        // 40 minutes of streak → warnings at 20 and 40.
        assert_eq!(warned_minutes, vec![20, 40]);
    }

    #[test]
    fn productive_resets_streak() {
        let mut w = Watch::default();
        for _ in 0..25 {
            observe(
                &mut w,
                Some(&obs("instagram.com", "distracting", false, 60)),
                true,
                20,
                None,
            );
        }
        assert!(w.streak >= 20 * 60);
        // switch to productive → streak clears, no warning
        let a = observe(
            &mut w,
            Some(&obs("code.exe", "productive", false, 60)),
            true,
            20,
            None,
        );
        assert!(a.is_empty());
        assert_eq!(w.streak, 0);
    }

    #[test]
    fn disabled_never_warns() {
        let mut w = Watch::default();
        let mut any = false;
        for _ in 0..40 {
            if !observe(
                &mut w,
                Some(&obs("instagram.com", "distracting", false, 60)),
                false,
                20,
                None,
            )
            .is_empty()
            {
                any = true;
            }
        }
        assert!(!any);
    }

    #[test]
    fn focus_violation_is_throttled() {
        let mut w = Watch::default();
        let focus = FocusCtx {
            goal: Some("Ship feature".into()),
            allowed: vec![],
            blocked: vec!["instagram.com".into()],
        };
        let mut hits = 0;
        // 10s ticks on a blocked target for 3 minutes.
        for _ in 0..18 {
            for a in observe(
                &mut w,
                Some(&obs("instagram.com", "distracting", false, 10)),
                false,
                20,
                Some(&focus),
            ) {
                if matches!(a, Alert::FocusViolation { .. }) {
                    hits += 1;
                }
            }
        }
        // Cooldown is 60s → at most one nudge per minute, so ~3 over 3 minutes.
        assert!((1..=3).contains(&hits), "expected 1..=3 nudges, got {hits}");
    }

    #[test]
    fn allowlist_flags_nonallowed_distraction() {
        let f = FocusCtx {
            goal: None,
            allowed: vec!["github.com".into()],
            blocked: vec![],
        };
        assert!(is_violation(
            &f,
            &obs("instagram.com", "distracting", false, 10)
        ));
        assert!(!is_violation(
            &f,
            &obs("github.com", "productive", false, 10)
        ));
    }

    #[test]
    fn pretty_labels() {
        assert_eq!(pretty_label("instagram.com"), "Instagram");
        assert_eq!(pretty_label("mail.google.com"), "Google");
        assert_eq!(pretty_label("chrome.exe"), "Chrome");
        assert_eq!(pretty_label("Slack"), "Slack");
    }

    #[test]
    fn browser_detection_is_token_based() {
        assert!(is_browser("chrome.exe"));
        assert!(is_browser("Google Chrome"));
        assert!(is_browser("Microsoft Edge"));
        assert!(is_browser("msedge.exe"));
        assert!(is_browser("Arc"));
        // Not browsers — must not match as substrings.
        assert!(!is_browser("Monarch"));
        assert!(!is_browser("Search"));
        assert!(!is_browser("Slack"));
        assert!(!is_browser("code.exe"));
    }
}
