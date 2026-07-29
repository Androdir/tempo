//! Aggregation cores shared by the desktop command wrappers and the Tempo Hub
//! server. Every function takes `&Connection` and has no Tauri/GUI dependency,
//! so the hub can reuse the exact same classification + block logic.
//!
//! Functions are moved here incrementally from the desktop `commands.rs`; the
//! desktop crate re-imports them via `use crate::aggregate::*` so existing call
//! sites are unchanged.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, Duration, Local, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use crate::classify;
use crate::llm;
use crate::lockin;
use crate::models::*;
use crate::projects::{self, Project};
use crate::rules;
use crate::scoring;
use crate::settings;

pub fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn category_buckets(conn: &Connection) -> HashMap<String, String> {
    list_category_definitions(conn)
        .unwrap_or_default()
        .into_iter()
        .map(|c| (c.id, c.bucket))
        .collect()
}

fn bucket_for_category(map: &HashMap<String, String>, category: &str) -> String {
    map.get(category)
        .cloned()
        .unwrap_or_else(|| bucket_for(category).to_string())
}

pub fn load_rules(conn: &Connection) -> Result<HashMap<String, String>, String> {
    let mut stmt = conn
        .prepare("SELECT app_name, category FROM category_rules")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut map = HashMap::new();
    for row in rows {
        let (k, v) = row.map_err(|e| e.to_string())?;
        map.insert(k, v);
    }
    Ok(map)
}

pub fn domain_category_map(conn: &Connection) -> Result<HashMap<String, Option<String>>, String> {
    let mut stmt = conn
        .prepare("SELECT domain, category FROM domain_rules")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut map = HashMap::new();
    for row in rows {
        let (k, v) = row.map_err(|e| e.to_string())?;
        map.insert(k, v);
    }
    Ok(map)
}

pub fn parse_keywords(s: Option<String>) -> Vec<String> {
    s.and_then(|v| serde_json::from_str::<Vec<String>>(&v).ok())
        .unwrap_or_default()
}

pub fn load_projects(conn: &Connection) -> Result<Vec<Project>, String> {
    projects::list_projects(conn).map_err(|e| e.to_string())
}

/// One aggregated activity block for a day, with rule-based classification
/// already resolved. Shared by the Activity Log and the LLM worker so cache
/// keys line up.
pub struct Block {
    pub source: String,
    pub label: String,
    pub title: String,
    pub domain: Option<String>,
    pub summary: Option<String>,
    pub content_type: Option<String>,
    pub seconds: i64,
    pub last_seen: String,
    pub first_seen: String,
    pub detail_id: Option<i64>,
    pub block_key: String,
    pub rule_category: String,
    pub rule_reason: String,
    pub rule_project: Option<String>,
    pub rule_project_confidence: u8,
    pub rule_signals: Vec<String>,
    pub confidence: f64,
    pub needs_llm: bool,
}

/// Stable key identifying an activity block within a day.
pub fn block_key(day: &str, source: &str, label: &str, title: &str) -> String {
    let t: String = title.chars().take(80).collect();
    format!("{day}|{source}|{}|{}", label.to_ascii_lowercase(), t.to_ascii_lowercase())
}

pub fn load_app_ai(conn: &Connection) -> Result<HashSet<String>, String> {
    let mut stmt = conn
        .prepare("SELECT app_name FROM category_rules WHERE ai_review = 1")
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

pub fn load_domain_ai(conn: &Connection) -> Result<HashSet<String>, String> {
    let mut stmt = conn
        .prepare("SELECT domain FROM domain_rules WHERE ai_review = 1")
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

#[allow(clippy::too_many_arguments)]
pub fn make_block(
    source: &str,
    label: String,
    title: String,
    domain: Option<String>,
    summary: Option<String>,
    content_type: Option<String>,
    seconds: i64,
    last_seen: String,
    first_seen: String,
    detail_id: Option<i64>,
    day: &str,
    v: rules::RuleVerdict,
) -> Block {
    let key = block_key(day, source, &label, &title);
    Block {
        source: source.to_string(),
        label,
        title,
        domain,
        summary,
        content_type,
        seconds,
        last_seen,
        first_seen,
        detail_id,
        block_key: key,
        rule_category: v.category,
        rule_reason: v.reason,
        rule_project: v.project,
        rule_project_confidence: v.project_confidence,
        rule_signals: v.project_signals,
        confidence: v.confidence,
        needs_llm: v.needs_llm,
    }
}

/// Collect today's activity blocks (apps + websites + screen OCR), sorted most
/// recent first, each with the rule-based classification resolved.
pub fn collect_blocks(conn: &Connection) -> Result<Vec<Block>, String> {
    collect_blocks_for_day(conn, &today())
}

/// Same as `collect_blocks` but for an arbitrary local day (weekly review).
pub fn collect_blocks_for_day(conn: &Connection, day: &str) -> Result<Vec<Block>, String> {
    let day = day.to_string();
    let app_rules = load_rules(conn)?;
    let app_ai = load_app_ai(conn)?;
    let domain_cat = domain_category_map(conn)?;
    let domain_ai = load_domain_ai(conn)?;
    let projects = load_projects(conn)?;
    let inputs = rules::RuleInputs {
        app_rules: &app_rules,
        app_ai: &app_ai,
        domain_cat: &domain_cat,
        domain_ai: &domain_ai,
        projects: &projects,
    };
    let mut blocks: Vec<Block> = Vec::new();

    // Desktop apps.
    {
        let mut stmt = conn
            .prepare(
                "SELECT app_name, window_title, SUM(duration_seconds), MAX(timestamp), MIN(timestamp)
                 FROM activity_log WHERE day = ?1 AND is_idle = 0
                 GROUP BY app_name, window_title",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([&day], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (app, title, secs, ts, fs) = row.map_err(|e| e.to_string())?;
            let v = rules::classify_block(&inputs, "app", &app, &title, None, None, None, &[]);
            blocks.push(make_block("app", app, title, None, None, None, secs, ts, fs, None, &day, v));
        }
    }

    // Browser pages.
    {
        let mut stmt = conn
            .prepare(
                "SELECT MAX(id), domain, page_title, MAX(content_type), MAX(content_summary),
                        MAX(detected_keywords), SUM(duration_seconds), MAX(timestamp), MIN(timestamp)
                 FROM browser_activity WHERE day = ?1 AND is_idle = 0
                 GROUP BY domain, page_title",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([&day], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (id, domain, title, ctype, summary, kw, secs, ts, fs) = row.map_err(|e| e.to_string())?;
            let keywords = parse_keywords(kw);
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
            blocks.push(make_block(
                "web", domain.clone(), title, Some(domain), summary, ctype, secs, ts, fs, Some(id), &day, v,
            ));
        }
    }

    // Screen OCR snapshots.
    {
        let mut stmt = conn
            .prepare(
                "SELECT app_name, window_title, MAX(ocr_summary), MAX(detected_keywords),
                        MAX(category), MAX(timestamp), MIN(timestamp)
                 FROM smart_activity WHERE day = ?1
                 GROUP BY app_name, window_title",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([&day], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (app, title, ocr_summary, kw, _category, ts, fs) = row.map_err(|e| e.to_string())?;
            let keywords = parse_keywords(kw);
            let v = rules::classify_block(
                &inputs, "screen", &app, &title, None, None, ocr_summary.as_deref(), &keywords,
            );
            blocks.push(make_block(
                "screen", app, title, None, ocr_summary, None, 0, ts, fs, None, &day, v,
            ));
        }
    }

    blocks.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
    Ok(blocks)
}

// ------------------------------------------------------------- today dashboard

pub fn sorted_desc<T>(map: HashMap<String, i64>, make: impl Fn(String, i64) -> T) -> Vec<T> {
    let mut pairs: Vec<(String, i64)> = map.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    pairs.into_iter().map(|(k, s)| make(k, s)).collect()
}

/// Extra haystack text (page summary + keywords) for project keyword matching.
pub fn browser_extra(summary: Option<&str>, keywords: &[String]) -> String {
    let mut s = String::new();
    if let Some(x) = summary {
        s.push_str(x);
        s.push(' ');
    }
    s.push_str(&keywords.join(" "));
    s
}

pub fn is_browser_app(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["chrome", "comet", "edge", "brave", "firefox", "opera", "vivaldi", "chromium", "arc", "safari"]
        .iter()
        .any(|b| n.contains(b))
}

pub struct BrowserRowLite {
    pub domain: String,
    pub title: String,
    pub content_type: Option<String>,
    pub summary: Option<String>,
    pub keywords: Vec<String>,
    pub duration: i64,
}

pub fn query_browser_rows(conn: &Connection, day: &str) -> Result<Vec<BrowserRowLite>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT domain, page_title, content_type, content_summary, detected_keywords, duration_seconds
             FROM browser_activity WHERE day = ?1 AND is_idle = 0",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([day], |r| {
            Ok(BrowserRowLite {
                domain: r.get(0)?,
                title: r.get(1)?,
                content_type: r.get(2)?,
                summary: r.get(3)?,
                keywords: parse_keywords(r.get::<_, Option<String>>(4)?),
                duration: r.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Today's-style aggregation for an arbitrary day: desktop apps merged with
/// browser websites, rolled into categories + buckets. Shared by the desktop
/// dashboard and the hub.
pub fn summary_for_day(conn: &Connection, day: &str) -> Result<TodaySummary, String> {
    let mut app_rows: Vec<(String, i64)> = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT app_name, SUM(duration_seconds) AS secs
                 FROM activity_log WHERE day = ?1 AND is_idle = 0
                 GROUP BY app_name ORDER BY secs DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([day], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(|e| e.to_string())?;
        for row in rows {
            app_rows.push(row.map_err(|e| e.to_string())?);
        }
    }

    let total_idle_seconds: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(duration_seconds), 0) FROM activity_log WHERE day = ?1 AND is_idle = 1",
            [day],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;

    let app_rules = load_rules(conn)?;
    let domain_rules = domain_category_map(conn)?;
    let projects = load_projects(conn)?;
    let browser_rows = query_browser_rows(conn, day)?;
    let bucket_map = category_buckets(conn);

    let mut per_category: HashMap<String, i64> = HashMap::new();
    let mut per_bucket: HashMap<String, i64> = HashMap::new();
    let mut domain_secs: HashMap<String, (i64, i64)> = HashMap::new();
    let mut browser_seconds = 0i64;
    for row in &browser_rows {
        browser_seconds += row.duration;
        let dcat = domain_rules.get(&row.domain).cloned().flatten();
        let base = classify::classify(
            &row.domain,
            &row.title,
            row.content_type.as_deref(),
            row.summary.as_deref(),
            &row.keywords,
            dcat.as_deref(),
        );
        let extra = browser_extra(row.summary.as_deref(), &row.keywords);
        let (cat, _r, _pm) =
            projects::resolve(&projects, &row.domain, &row.title, &extra, &base.category, &base.reason);
        let cat = category_or_fallback(conn, &cat);
        *per_category.entry(cat.clone()).or_insert(0) += row.duration;
        *per_bucket.entry(bucket_for_category(&bucket_map, &cat)).or_insert(0) += row.duration;
        let entry = domain_secs.entry(row.domain.clone()).or_insert((0, 0));
        entry.0 += row.duration;
        entry.1 += 1;
    }
    let has_browser = browser_seconds > 0;

    let mut per_app = Vec::new();
    let mut app_active = 0i64;
    for (name, secs) in &app_rows {
        if has_browser && is_browser_app(name) {
            continue;
        }
        let category = app_rules.get(name).cloned();
        app_active += *secs;
        let cat_key = category.clone().unwrap_or_else(|| "uncategorized".to_string());
        *per_category.entry(cat_key.clone()).or_insert(0) += *secs;
        *per_bucket.entry(bucket_for_category(&bucket_map, &cat_key)).or_insert(0) += *secs;
        per_app.push(AppUsage { app_name: name.clone(), seconds: *secs, category });
    }

    let mut per_website: Vec<WebsiteUsage> = domain_secs
        .into_iter()
        .map(|(domain, (secs, views))| {
            let category = domain_rules.get(&domain).cloned().flatten();
            WebsiteUsage { domain, seconds: secs, category, page_views: views }
        })
        .collect();
    per_website.sort_by(|a, b| b.seconds.cmp(&a.seconds).then_with(|| a.domain.cmp(&b.domain)));

    Ok(TodaySummary {
        date: day.to_string(),
        total_active_seconds: app_active + browser_seconds,
        total_idle_seconds,
        total_browser_seconds: browser_seconds,
        per_app,
        per_website,
        per_category: sorted_desc(per_category, |category, seconds| CategoryUsage { category, seconds }),
        per_bucket: sorted_desc(per_bucket, |bucket, seconds| BucketUsage { bucket, seconds }),
    })
}

// -------------------------------------------------- classification resolution

pub fn load_manual_corrections(conn: &Connection, day: &str) -> Result<HashMap<String, String>, String> {
    let mut stmt = conn
        .prepare("SELECT block_key, category FROM manual_corrections WHERE day = ?1")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([day], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut map = HashMap::new();
    for row in rows {
        let (k, v) = row.map_err(|e| e.to_string())?;
        map.insert(k, v);
    }
    Ok(map)
}

/// Final category for a block: manual correction > cached LLM (when rules wanted
/// review) > rule-based.
pub fn final_category(
    b: &Block,
    manual: &HashMap<String, String>,
    llm_cache: &HashMap<String, llm::LlmClassification>,
) -> String {
    if let Some(c) = manual.get(&b.block_key) {
        return c.clone();
    }
    if b.needs_llm {
        if let Some(c) = llm_cache.get(&b.block_key) {
            return c.category.clone();
        }
    }
    b.rule_category.clone()
}

pub fn parse_utc(s: &str) -> Option<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}

pub fn local_minutes(ts: &str) -> Option<i64> {
    use chrono::{DateTime, Timelike};
    DateTime::parse_from_rfc3339(ts).ok().map(|dt| {
        let l = dt.with_timezone(&Local);
        l.hour() as i64 * 60 + l.minute() as i64
    })
}

// ----------------------------------------------------- daily stats + check-ins

pub fn compute_stats(conn: &Connection) -> Result<scoring::Stats, String> {
    compute_stats_for_day(conn, &today())
}

/// The labels `target` score rules watch (lowercased), e.g. "instagram".
fn target_metrics(conn: &Connection) -> Vec<String> {
    scoring::list_rules(conn)
        .into_iter()
        .filter(|r| r.kind == "target" && !r.metric.is_empty())
        .map(|r| r.metric)
        .collect()
}

pub fn compute_stats_for_day(conn: &Connection, day: &str) -> Result<scoring::Stats, String> {
    let day = day.to_string();
    let llm_cache = llm::load_cache_for_day(conn, &day);
    let manual = load_manual_corrections(conn, &day)?;
    let blocks = collect_blocks_for_day(conn, &day)?;
    let bucket_map = category_buckets(conn);
    let targets = target_metrics(conn);

    let mut cat_seconds: HashMap<String, i64> = HashMap::new();
    let mut target_seconds: HashMap<String, i64> = HashMap::new();
    let mut first_productive: Option<i64> = None;

    for b in &blocks {
        if b.seconds <= 0 {
            continue;
        }
        let cat = category_or_fallback(conn, &final_category(b, &manual, &llm_cache));
        let bucket = bucket_for_category(&bucket_map, &cat);
        *cat_seconds.entry(cat).or_insert(0) += b.seconds;
        if bucket != "productive" {
            // Time on a watched app/site only counts while it isn't productive work
            // (a corrected "research on YouTube" block doesn't ding the score).
            let label = b.domain.clone().unwrap_or_else(|| b.label.clone()).to_ascii_lowercase();
            for t in &targets {
                if label.contains(t.as_str()) {
                    *target_seconds.entry(t.clone()).or_insert(0) += b.seconds;
                }
            }
        }
        if bucket == "productive" {
            if let Some(m) = local_minutes(&b.first_seen) {
                first_productive = Some(first_productive.map_or(m, |cur| cur.min(m)));
            }
        }
    }

    Ok(scoring::Stats { cat_seconds, target_seconds, first_productive_min: first_productive })
}

pub fn top_goal_completed(conn: &Connection, day: &str) -> Option<bool> {
    conn.query_row(
        "SELECT completed FROM goals WHERE day = ?1
         ORDER BY CASE priority WHEN 'high' THEN 0 WHEN 'medium' THEN 1 ELSE 2 END, sort_order, id
         LIMIT 1",
        [day],
        |r| r.get::<_, i64>(0),
    )
    .ok()
    .map(|v| v != 0)
}

pub fn effective_checkins(conn: &Connection, day: &str) -> scoring::Checkins {
    let main_goal_completed = top_goal_completed(conn, day).unwrap_or_else(|| {
        conn.query_row("SELECT main_goal_completed FROM daily_checkin WHERE day = ?1", [day], |r| {
            r.get::<_, i64>(0)
        })
        .map(|v| v != 0)
        .unwrap_or(false)
    });
    scoring::Checkins { main_goal_completed, values: checkin_map_for_day(conn, day) }
}

pub fn top_project_name(conn: &Connection) -> Option<String> {
    projects::list_projects(conn).ok().and_then(|p| p.into_iter().max_by_key(|x| x.priority).map(|x| x.name))
}

// ----------------------------------------------------------------- outputs

pub fn row_to_output(r: &rusqlite::Row) -> rusqlite::Result<OutputEvent> {
    Ok(OutputEvent {
        id: r.get(0)?,
        timestamp: r.get(1)?,
        day: r.get(2)?,
        folder_path: r.get(3)?,
        file_path: r.get(4)?,
        file_name: r.get(5)?,
        extension: r.get(6)?,
        file_size: r.get(7)?,
        event_type: r.get(8)?,
        project: r.get(9)?,
        linked_block_key: r.get(10)?,
        linked_label: r.get(11)?,
        created_at: r.get(12)?,
        modified_at: r.get(13)?,
    })
}

pub fn output_signals_for_day(conn: &Connection, day: &str) -> scoring::OutputSignals {
    let count = |etype: &str| {
        conn.query_row(
            "SELECT COUNT(*) FROM output_events WHERE day = ?1 AND event_type = ?2",
            params![day, etype],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
    };
    scoring::OutputSignals {
        video_exports: count("video_export"),
        code_changes: count("code_change"),
        study_materials: count("study_material"),
    }
}

pub fn link_outputs_for_day(conn: &Connection, day: &str) -> Result<i64, String> {
    let events: Vec<(i64, Option<String>)> = {
        let mut stmt = conn
            .prepare("SELECT id, modified_at FROM output_events WHERE day = ?1 AND linked_block_key IS NULL")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([day], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?)))
            .map_err(|e| e.to_string())?;
        rows.filter_map(Result::ok).collect()
    };
    let mut linked = 0i64;
    for (id, modified_at) in events {
        let Some(when) = modified_at.as_deref().and_then(parse_utc) else { continue };
        let lo = (when - Duration::minutes(10)).to_rfc3339();
        let hi = (when + Duration::minutes(10)).to_rfc3339();
        let app: Option<(String, i64)> = conn
            .query_row(
                "SELECT app_name, SUM(duration_seconds) AS s FROM activity_log
                 WHERE is_idle = 0 AND timestamp BETWEEN ?1 AND ?2 GROUP BY app_name ORDER BY s DESC LIMIT 1",
                params![lo, hi],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let web: Option<(String, i64)> = conn
            .query_row(
                "SELECT domain, SUM(duration_seconds) AS s FROM browser_activity
                 WHERE is_idle = 0 AND timestamp BETWEEN ?1 AND ?2 GROUP BY domain ORDER BY s DESC LIMIT 1",
                params![lo, hi],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let chosen = match (app, web) {
            (Some(a), Some(w)) => Some(if w.1 > a.1 { ("web", w.0) } else { ("app", a.0) }),
            (Some(a), None) => Some(("app", a.0)),
            (None, Some(w)) => Some(("web", w.0)),
            (None, None) => None,
        };
        if let Some((source, label)) = chosen {
            let key = format!("{day}|{source}|{}", label.to_ascii_lowercase());
            conn.execute(
                "UPDATE output_events SET linked_block_key = ?1, linked_label = ?2 WHERE id = ?3",
                params![key, label, id],
            )
            .map_err(|e| e.to_string())?;
            linked += 1;
        }
    }
    Ok(linked)
}

// ------------------------------------------------------------------- streaks

pub const STREAK_WINDOW_DAYS: i64 = 28;

pub fn day_metrics(conn: &Connection, day: &str) -> crate::streaks::DayMetrics {
    let main_goal = top_goal_completed(conn, day).unwrap_or_else(|| {
        conn.query_row("SELECT main_goal_completed FROM daily_checkin WHERE day = ?1", [day], |r| {
            r.get::<_, i64>(0)
        })
        .map(|v| v != 0)
        .unwrap_or(false)
    });
    let count = |etype: &str| {
        conn.query_row(
            "SELECT COUNT(*) FROM output_events WHERE day = ?1 AND event_type = ?2",
            params![day, etype],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
    };

    let mut m = crate::streaks::DayMetrics {
        checkins: checkin_map_for_day(conn, day),
        main_goal_completed: main_goal,
        video_exports: count("video_export"),
        editing_changes: count("editing_project_changed"),
        ..Default::default()
    };

    if let Ok(blocks) = collect_blocks_for_day(conn, day) {
        let manual = load_manual_corrections(conn, day).unwrap_or_default();
        let llm = llm::load_cache_for_day(conn, day);
        for b in &blocks {
            if b.seconds <= 0 {
                continue;
            }
            let cat = category_or_fallback(conn, &final_category(b, &manual, &llm));
            let bucket = bucket_for(&cat);
            let min = b.seconds / 60;
            *m.cat_minutes.entry(cat).or_insert(0) += min;
            m.tracked_min += min;
            if bucket == "productive" {
                m.max_productive_block_min = m.max_productive_block_min.max(min);
            } else if bucket == "distracting" {
                m.max_distraction_block_min = m.max_distraction_block_min.max(min);
            }
        }
    }
    m
}

pub fn compute_streaks(conn: &Connection) -> Result<Vec<Streak>, String> {
    let defs: Vec<crate::streaks::StreakDef> =
        crate::streaks::load_defs(conn).into_iter().filter(|d| d.enabled).collect();
    if defs.is_empty() {
        return Ok(Vec::new());
    }
    let today = Local::now().date_naive();
    let days: Vec<String> = (0..STREAK_WINDOW_DAYS)
        .rev()
        .map(|i| (today - Duration::days(i)).format("%Y-%m-%d").to_string())
        .collect();
    let metrics: Vec<crate::streaks::DayMetrics> = days.iter().map(|d| day_metrics(conn, d)).collect();

    let today_weekday0 = today.weekday().num_days_from_monday() as usize;
    let mut out = Vec::new();
    for def in &defs {
        let status: Vec<bool> = metrics.iter().map(|m| crate::streaks::streak_met(def, m)).collect();
        // Daily streaks run in days; weekly ("N days per week") streaks in weeks.
        let (current, window_best, week_met_days) = if def.days_per_week > 0 {
            crate::streaks::weekly_runs(&status, today_weekday0, def.days_per_week)
        } else {
            (crate::streaks::current_run(&status), crate::streaks::best_run(&status), 0)
        };
        let best = def.best_streak.max(window_best).max(current);
        let last = days
            .iter()
            .zip(&status)
            .rev()
            .find(|(_, &s)| s)
            .map(|(d, _)| d.clone())
            .or_else(|| def.last_completed_day.clone());
        let _ = conn.execute(
            "UPDATE streak_definitions SET best_streak = ?1,
                last_completed_day = COALESCE(?2, last_completed_day) WHERE id = ?3",
            params![best, last, def.id],
        );
        let calendar: Vec<StreakDay> =
            days.iter().zip(&status).map(|(d, &met)| StreakDay { day: d.clone(), met }).collect();
        out.push(Streak {
            id: def.id.clone(),
            name: def.name.clone(),
            kind: def.kind.clone(),
            metric: def.metric.clone(),
            threshold: def.threshold,
            enabled: def.enabled,
            days_per_week: def.days_per_week,
            current,
            best,
            week_met_days,
            last_completed_day: last,
            calendar,
        });
    }
    Ok(out)
}

// ------------------------------------------------------- proof-of-work timeline

/// One classified raw sample, before adjacent ones are merged into a block.
struct TlSample {
    ts: chrono::DateTime<Utc>,
    source: String,
    label: String,
    title: String,
    idle: bool,
    eff_seconds: i64,
    is_web: bool,
    summary: Option<String>,
    category: String,
    bucket: String,
    project: Option<String>,
    project_confidence: u8,
    confidence: f64,
    classifier: String,
    block_key: String,
}

/// Merge a source's samples into continuous blocks. Splits on a change of
/// app/domain, category, project, or idle status, or a gap > `max_gap` seconds.
fn merge_samples(mut samples: Vec<TlSample>, max_gap: i64) -> Vec<TimelineBlock> {
    samples.sort_by(|a, b| a.ts.cmp(&b.ts));
    let mut out: Vec<TimelineBlock> = Vec::new();
    let mut cur: Option<TimelineBlock> = None;
    let mut prev_ts: Option<chrono::DateTime<Utc>> = None;
    for s in samples {
        let s_ts = s.ts;
        let s_eff = s.eff_seconds;
        let continues = match (cur.as_ref(), prev_ts) {
            (Some(c), Some(p)) => {
                let gap = (s_ts - p).num_seconds();
                c.label == s.label
                    && c.category == s.category
                    && c.project == s.project
                    && c.idle == s.idle
                    && (0..=max_gap).contains(&gap)
            }
            _ => false,
        };
        if continues {
            let c = cur.as_mut().unwrap();
            c.duration_seconds += s_eff;
            c.end = (s_ts + Duration::seconds(s_eff)).to_rfc3339();
            c.sample_count += 1;
            c.title = s.title;
            c.classifier = s.classifier;
            c.confidence = s.confidence;
            c.block_key = s.block_key;
            c.project_confidence = s.project_confidence;
            if c.summary.is_none() {
                c.summary = s.summary;
            }
        } else {
            if let Some(c) = cur.take() {
                out.push(c);
            }
            cur = Some(TimelineBlock {
                source: s.source,
                start: s_ts.to_rfc3339(),
                end: (s_ts + Duration::seconds(s_eff)).to_rfc3339(),
                duration_seconds: s_eff,
                label: s.label,
                title: s.title,
                category: s.category,
                bucket: s.bucket,
                project: s.project,
                project_confidence: s.project_confidence,
                confidence: s.confidence,
                classifier: s.classifier,
                idle: s.idle,
                summary: s.summary,
                block_key: s.block_key,
                is_web: s.is_web,
                sample_count: 1,
                absorbed_seconds: 0,
                absorbed_count: 0,
                longest_productive: false,
                biggest_distraction: false,
                first_productive: false,
                goal_related: false,
                output_linked: false,
            });
        }
        prev_ts = Some(s_ts);
    }
    if let Some(c) = cur.take() {
        out.push(c);
    }
    out
}

/// Whether two blocks represent the same meaningful activity for the simplified
/// overview. Titles may change inside an app, but source, label, project,
/// category and idle state must still agree.
fn same_overview_activity(a: &TimelineBlock, b: &TimelineBlock) -> bool {
    a.source == b.source
        && a.label.eq_ignore_ascii_case(&b.label)
        && a.category == b.category
        && a.project == b.project
        && a.idle == b.idle
        && a.is_web == b.is_web
}

fn blocks_are_close(a: &TimelineBlock, b: &TimelineBlock, tolerance_seconds: i64) -> bool {
    match (parse_utc(&a.end), parse_utc(&b.start)) {
        (Some(a_end), Some(b_start)) => {
            let gap = (b_start - a_end).num_seconds();
            (-5..=tolerance_seconds).contains(&gap)
        }
        _ => false,
    }
}

fn refresh_timeline_highlights(blocks: &mut [TimelineBlock]) {
    let mut longest_prod: Option<(usize, i64)> = None;
    let mut biggest_dist: Option<(usize, i64)> = None;
    let mut first_prod: Option<usize> = None;

    for (i, block) in blocks.iter_mut().enumerate() {
        block.longest_productive = false;
        block.biggest_distraction = false;
        block.first_productive = false;
        if block.idle {
            continue;
        }
        match block.bucket.as_str() {
            "productive" => {
                longest_prod = Some(match longest_prod {
                    Some((j, duration)) if duration >= block.duration_seconds => (j, duration),
                    _ => (i, block.duration_seconds),
                });
                if first_prod.is_none() {
                    first_prod = Some(i);
                }
            }
            "distracting" => {
                biggest_dist = Some(match biggest_dist {
                    Some((j, duration)) if duration >= block.duration_seconds => (j, duration),
                    _ => (i, block.duration_seconds),
                });
            }
            _ => {}
        }
    }

    if let Some((i, _)) = longest_prod {
        blocks[i].longest_productive = true;
    }
    if let Some((i, _)) = biggest_dist {
        blocks[i].biggest_distraction = true;
    }
    if let Some(i) = first_prod {
        blocks[i].first_productive = true;
    }
}

/// Build the low-noise Activity overview. A brief intervening block is
/// absorbed only for an A → B → A pattern where A resumes immediately. Exact
/// blocks remain untouched and are returned separately.
fn smooth_brief_interruptions(
    blocks: &[TimelineBlock],
    max_interrupt_seconds: i64,
) -> Vec<TimelineBlock> {
    let mut overview = blocks.to_vec();
    if max_interrupt_seconds <= 0 {
        return overview;
    }

    loop {
        let mut found: Option<usize> = None;
        for i in 1..overview.len().saturating_sub(1) {
            let left = &overview[i - 1];
            let interruption = &overview[i];
            let right = &overview[i + 1];
            if interruption.duration_seconds <= max_interrupt_seconds
                && !same_overview_activity(left, interruption)
                && same_overview_activity(left, right)
                && blocks_are_close(left, interruption, max_interrupt_seconds)
                && blocks_are_close(interruption, right, max_interrupt_seconds)
            {
                found = Some(i);
                break;
            }
        }

        let Some(i) = found else { break };
        let left = overview[i - 1].clone();
        let interruption = overview[i].clone();
        let right = overview[i + 1].clone();
        let mut merged = left.clone();
        merged.end = right.end.clone();
        merged.duration_seconds =
            left.duration_seconds + interruption.duration_seconds + right.duration_seconds;
        merged.title = right.title.clone();
        merged.summary = right.summary.clone().or(left.summary.clone());
        merged.sample_count = left.sample_count + interruption.sample_count + right.sample_count;
        merged.absorbed_seconds = left.absorbed_seconds
            + interruption.duration_seconds
            + right.absorbed_seconds;
        merged.absorbed_count =
            left.absorbed_count + interruption.absorbed_count + right.absorbed_count + 1;
        merged.confidence = left.confidence.min(right.confidence);
        merged.project_confidence = left.project_confidence.min(right.project_confidence);
        merged.classifier = if left.classifier == "manual" || right.classifier == "manual" {
            "manual".to_string()
        } else if left.classifier == "llm" || right.classifier == "llm" {
            "llm".to_string()
        } else {
            "rule".to_string()
        };
        merged.goal_related = left.goal_related || right.goal_related;
        merged.output_linked = left.output_linked || right.output_linked;
        merged.longest_productive = false;
        merged.biggest_distraction = false;
        merged.first_productive = false;
        overview.splice((i - 1)..=(i + 1), [merged]);
    }

    refresh_timeline_highlights(&mut overview);
    overview
}
fn overlaps(a: &TimelineBlock, b: &TimelineBlock) -> bool {
    match (parse_utc(&a.start), parse_utc(&a.end), parse_utc(&b.start), parse_utc(&b.end)) {
        (Some(a0), Some(a1), Some(b0), Some(b1)) => a0 < b1 && b0 < a1,
        _ => false,
    }
}

/// The proof-of-work timeline for a day: raw samples merged into continuous,
/// classified blocks, with the day's highlights flagged.
pub fn timeline_for_day(conn: &Connection, day: &str, max_gap: i64) -> Result<TimelineDay, String> {
    let app_rules = load_rules(conn)?;
    let app_ai = load_app_ai(conn)?;
    let domain_cat = domain_category_map(conn)?;
    let domain_ai = load_domain_ai(conn)?;
    let projects = load_projects(conn)?;
    let inputs = rules::RuleInputs {
        app_rules: &app_rules,
        app_ai: &app_ai,
        domain_cat: &domain_cat,
        domain_ai: &domain_ai,
        projects: &projects,
    };
    let manual = load_manual_corrections(conn, day)?;
    let llm_cache = llm::load_cache_for_day(conn, day);
    let bucket_map = category_buckets(conn);
    let smart_interval =
        settings::get_int(conn, settings::SMART_INTERVAL, settings::DEFAULT_SMART_INTERVAL).max(1);

    let (mut desk, mut web, mut scr): (Vec<TlSample>, Vec<TlSample>, Vec<TlSample>) =
        (Vec::new(), Vec::new(), Vec::new());

    {
        let mut resolved: HashMap<String, (String, Option<String>, u8, f64, String)> = HashMap::new();
        let mut classify = |code: &str,
                            label: &str,
                            title: &str,
                            domain: Option<&str>,
                            ctype: Option<&str>,
                            summary: Option<&str>,
                            keywords: &[String]|
         -> (String, (String, Option<String>, u8, f64, String)) {
            let key = block_key(day, code, label, title);
            let val = resolved
                .entry(key.clone())
                .or_insert_with(|| {
                    let v = rules::classify_block(&inputs, code, label, title, domain, ctype, summary, keywords);
                    // A weak keyword hit is useful diagnostic evidence, not a real
                    // assignment. Only expose projects once the deterministic
                    // matcher has enough independent evidence to override.
                    let visible_project = v
                        .project
                        .clone()
                        .filter(|_| v.project_confidence >= projects::OVERRIDE_THRESHOLD);
                    let visible_project_confidence =
                        if visible_project.is_some() { v.project_confidence } else { 0 };
                    if let Some(c) = manual.get(&key) {
                        (c.clone(), visible_project, visible_project_confidence, 1.0, "manual".to_string())
                    } else if v.needs_llm {
                        match llm_cache.get(&key) {
                            Some(c) => (
                                c.category.clone(),
                                // Project attribution always comes from deterministic evidence;
                                // the LLM may refine category, never invent a project.
                                visible_project,
                                visible_project_confidence,
                                c.confidence,
                                "llm".to_string(),
                            ),
                            None => (v.category, visible_project, visible_project_confidence, v.confidence, "rule".to_string()),
                        }
                    } else {
                        (v.category, visible_project, visible_project_confidence, v.confidence, "rule".to_string())
                    }
                })
                .clone();
            let (category, project, pconf, conf, classifier) = val;
            (key, (category_or_fallback(conn, &category), project, pconf, conf, classifier))
        };

        {
            let mut stmt = conn
                .prepare(
                    "SELECT timestamp, app_name, window_title, duration_seconds, is_idle
                     FROM activity_log WHERE day = ?1 ORDER BY timestamp",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([day], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)? != 0,
                    ))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                let (ts_s, app, title, secs, idle) = row.map_err(|e| e.to_string())?;
                let Some(ts) = parse_utc(&ts_s) else { continue };
                let (key, (category, project, pconf, conf, classifier)) =
                    classify("app", &app, &title, None, None, None, &[]);
                let bucket = bucket_for_category(&bucket_map, &category);
                desk.push(TlSample {
                    ts, source: "desktop".to_string(), label: app, title, idle,
                    eff_seconds: secs.max(1), is_web: false, summary: None, category, bucket,
                    project, project_confidence: pconf, confidence: conf, classifier, block_key: key,
                });
            }
        }

        {
            let mut stmt = conn
                .prepare(
                    "SELECT timestamp, domain, page_title, duration_seconds, is_idle,
                            content_type, content_summary, detected_keywords
                     FROM browser_activity WHERE day = ?1 ORDER BY timestamp",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([day], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)? != 0,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, Option<String>>(6)?,
                        r.get::<_, Option<String>>(7)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                let (ts_s, domain, title, secs, idle, ctype, summary, kw) = row.map_err(|e| e.to_string())?;
                let Some(ts) = parse_utc(&ts_s) else { continue };
                let keywords = parse_keywords(kw);
                let (key, (category, project, pconf, conf, classifier)) =
                    classify("web", &domain, &title, Some(&domain), ctype.as_deref(), summary.as_deref(), &keywords);
                let bucket = bucket_for_category(&bucket_map, &category);
                web.push(TlSample {
                    ts, source: "browser".to_string(), label: domain, title, idle,
                    eff_seconds: secs.max(1), is_web: true, summary, category, bucket,
                    project, project_confidence: pconf, confidence: conf, classifier, block_key: key,
                });
            }
        }

        {
            let mut stmt = conn
                .prepare(
                    "SELECT timestamp, app_name, window_title, ocr_summary, detected_keywords, is_idle
                     FROM smart_activity WHERE day = ?1 ORDER BY timestamp",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([day], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, i64>(5)? != 0,
                    ))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                let (ts_s, app, title, ocr, kw, idle) = row.map_err(|e| e.to_string())?;
                let Some(ts) = parse_utc(&ts_s) else { continue };
                let keywords = parse_keywords(kw);
                let (key, (category, project, pconf, conf, classifier)) =
                    classify("screen", &app, &title, None, None, ocr.as_deref(), &keywords);
                let bucket = bucket_for_category(&bucket_map, &category);
                scr.push(TlSample {
                    ts, source: "screen".to_string(), label: app, title, idle,
                    eff_seconds: smart_interval, is_web: false, summary: ocr, category, bucket,
                    project, project_confidence: pconf, confidence: conf, classifier, block_key: key,
                });
            }
        }
    }

    let web_blocks = merge_samples(web, max_gap);
    let mut desk_blocks = merge_samples(desk, max_gap);
    let mut scr_blocks = merge_samples(scr, max_gap);

    // Prefer the richest source for an interval. Browser records replace the
    // browser process, while Smart Tracking replaces its matching desktop row.
    // This prevents double-counting and contradictory labels for the same work.
    scr_blocks.retain(|s| {
        !is_browser_app(&s.label) || !web_blocks.iter().any(|w| overlaps(s, w))
    });
    desk_blocks.retain(|d| {
        let covered_by_browser =
            is_browser_app(&d.label) && web_blocks.iter().any(|w| overlaps(d, w));
        let covered_by_screen = scr_blocks
            .iter()
            .any(|s| s.label.eq_ignore_ascii_case(&d.label) && overlaps(d, s));
        !covered_by_browser && !covered_by_screen
    });

    let mut blocks: Vec<TimelineBlock> = Vec::new();
    blocks.extend(desk_blocks);
    blocks.extend(web_blocks);
    blocks.extend(scr_blocks);
    blocks.sort_by(|a, b| a.start.cmp(&b.start));

    let mut active = 0i64;
    let mut idle = 0i64;
    let mut prod = 0i64;
    let mut dist = 0i64;
    let mut longest_prod: Option<(usize, i64)> = None;
    let mut biggest_dist: Option<(usize, i64)> = None;
    let mut first_prod: Option<usize> = None;
    for (i, b) in blocks.iter().enumerate() {
        if b.idle {
            idle += b.duration_seconds;
            continue;
        }
        active += b.duration_seconds;
        match b.bucket.as_str() {
            "productive" => {
                prod += b.duration_seconds;
                longest_prod = Some(match longest_prod {
                    Some((j, dj)) if dj >= b.duration_seconds => (j, dj),
                    _ => (i, b.duration_seconds),
                });
                if first_prod.is_none() {
                    first_prod = Some(i);
                }
            }
            "distracting" => {
                dist += b.duration_seconds;
                biggest_dist = Some(match biggest_dist {
                    Some((j, dj)) if dj >= b.duration_seconds => (j, dj),
                    _ => (i, b.duration_seconds),
                });
            }
            _ => {}
        }
    }

    let goals: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT title FROM goals WHERE day = ?1
                 ORDER BY CASE priority WHEN 'high' THEN 0 WHEN 'medium' THEN 1 ELSE 2 END, sort_order, id",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([day], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
        rows.filter_map(Result::ok).collect()
    };
    let goal_projects: HashSet<String> = {
        let mut stmt = conn
            .prepare("SELECT DISTINCT project FROM goals WHERE day = ?1 AND project IS NOT NULL AND project <> ''")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([day], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
        rows.filter_map(Result::ok).map(|p| p.to_ascii_lowercase()).collect()
    };

    if let Some((i, _)) = longest_prod {
        blocks[i].longest_productive = true;
    }
    if let Some((i, _)) = biggest_dist {
        blocks[i].biggest_distraction = true;
    }
    if let Some(i) = first_prod {
        blocks[i].first_productive = true;
    }
    let first_productive_start = first_prod.map(|i| blocks[i].start.clone());

    for b in blocks.iter_mut() {
        if let Some(p) = &b.project {
            if goal_projects.contains(&p.to_ascii_lowercase()) {
                b.goal_related = true;
            }
        }
        if b.category == "business" {
            b.output_linked = true;
        }
    }

    let overview_blocks = smooth_brief_interruptions(&blocks, 20);
    let outputs: Vec<CheckinValue> =
        checkin_values_for_day(conn, day).into_iter().filter(|c| c.value > 0).collect();

    Ok(TimelineDay {
        day: day.to_string(),
        max_gap_seconds: max_gap,
        blocks,
        overview_blocks,
        outputs,
        active_seconds: active,
        idle_seconds: idle,
        productive_seconds: prod,
        distracted_seconds: dist,
        first_productive_start,
        goals,
    })
}

// ------------------------------------------------------------- weekly review

pub fn weekly_review(conn: &Connection) -> Result<WeeklyReview, String> {
    let today_local = Local::now().date_naive();

    let mut days_vec: Vec<WeeklyDay> = Vec::new();
    let mut productive = 0i64;
    let mut distraction = 0i64;
    let mut study = 0i64;
    let mut checkin_totals: HashMap<String, i64> = HashMap::new();
    let mut leak_by: HashMap<String, i64> = HashMap::new();

    for i in (0..7).rev() {
        let date = today_local - Duration::days(i);
        let day = date.format("%Y-%m-%d").to_string();
        let weekday = date.format("%a").to_string();

        let llm_cache = llm::load_cache_for_day(conn, &day);
        let manual = load_manual_corrections(conn, &day)?;
        let blocks = collect_blocks_for_day(conn, &day)?;
        let bucket_map = category_buckets(conn);

        let mut p = 0i64;
        let mut d = 0i64;
        let mut tracked = 0i64;
        for b in &blocks {
            if b.seconds <= 0 {
                continue;
            }
            let cat = category_or_fallback(conn, &final_category(b, &manual, &llm_cache));
            let bucket = bucket_for_category(&bucket_map, &cat);
            tracked += b.seconds;
            if cat == "study" {
                study += b.seconds;
            }
            match bucket.as_str() {
                "productive" => p += b.seconds,
                "distracting" => {
                    d += b.seconds;
                    let key = b.domain.clone().unwrap_or_else(|| b.label.clone());
                    *leak_by.entry(key).or_insert(0) += b.seconds;
                }
                _ => {}
            }
        }
        productive += p;
        distraction += d;
        // Counters sum their values across the week; toggles count met days.
        for c in checkin_values_for_day(conn, &day) {
            let inc = if c.kind == "counter" { c.value } else { (c.value > 0) as i64 };
            *checkin_totals.entry(c.id).or_insert(0) += inc;
        }

        let stats = compute_stats_for_day(conn, &day)?;
        let checkins = effective_checkins(conn, &day);
        let goal = top_project_name(conn);
        let outputs = output_signals_for_day(conn, &day);
        let report = scoring::build_report(conn, day.clone(), &stats, &checkins, &outputs, goal);

        days_vec.push(WeeklyDay {
            day,
            weekday,
            score: report.score,
            productive_seconds: p,
            distraction_seconds: d,
            tracked_seconds: tracked,
        });
    }

    let active: Vec<&WeeklyDay> = days_vec.iter().filter(|d| d.tracked_seconds > 0).collect();
    let best_day = active.iter().max_by_key(|d| d.score).map(|d| (*d).clone());
    let worst_day = active.iter().min_by_key(|d| d.score).map(|d| (*d).clone());

    let (most_common_leak, most_common_leak_seconds) = leak_by
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|(k, v)| (Some(k), v))
        .unwrap_or((None, 0));

    // Present totals in definition order, skipping check-ins never logged this week.
    let totals: Vec<CheckinTotal> = list_checkin_definitions(conn)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|d| {
            let total = checkin_totals.get(&d.id).copied().unwrap_or(0);
            (total > 0).then(|| CheckinTotal {
                id: d.id,
                label: d.label,
                icon: d.icon,
                kind: d.kind,
                total,
            })
        })
        .collect();

    Ok(WeeklyReview {
        start_day: days_vec.first().map(|d| d.day.clone()).unwrap_or_default(),
        end_day: days_vec.last().map(|d| d.day.clone()).unwrap_or_default(),
        productive_seconds: productive,
        distraction_seconds: distraction,
        study_seconds: study,
        checkin_totals: totals,
        best_day,
        worst_day,
        most_common_leak,
        most_common_leak_seconds,
        days: days_vec,
    })
}

// --------------------------------------------------------------- daily score

/// Assemble the full daily score report for a day (stats + check-ins + outputs).
pub fn score_report_for_day(conn: &Connection, day: &str) -> Result<ScoreReport, String> {
    let stats = compute_stats_for_day(conn, day)?;
    let checkins = effective_checkins(conn, day);
    let outputs = output_signals_for_day(conn, day);
    let goal = top_project_name(conn);
    Ok(scoring::build_report(conn, day.to_string(), &stats, &checkins, &outputs, goal))
}

// --------------------------------------------------------------- goal helpers

/// Next free `sort_order` for a day's goal list (max + 1, or 1 when empty).
pub fn next_sort_order(conn: &Connection, day: &str) -> i64 {
    conn.query_row("SELECT COALESCE(MAX(sort_order), 0) + 1 FROM goals WHERE day = ?1", [day], |r| {
        r.get(0)
    })
    .unwrap_or(0)
}

/// Whether a goal with this exact title already exists for the day.
pub fn goal_exists(conn: &Connection, day: &str, title: &str) -> bool {
    conn.query_row("SELECT 1 FROM goals WHERE day = ?1 AND title = ?2 LIMIT 1", params![day, title], |_| Ok(()))
        .optional()
        .unwrap_or(None)
        .is_some()
}

// --------------------------------------------------------------- daily lock-in plan

/// The single biggest distracting block of the day (label + minutes), if any.
pub fn top_distraction_for_day(conn: &Connection, day: &str) -> Option<(String, i64)> {
    let blocks = collect_blocks_for_day(conn, day).ok()?;
    let manual = load_manual_corrections(conn, day).unwrap_or_default();
    let llm = llm::load_cache_for_day(conn, day);
    let bucket_map = category_buckets(conn);
    let mut best: Option<(String, i64)> = None;
    for b in &blocks {
        if b.seconds <= 0 {
            continue;
        }
        let cat = category_or_fallback(conn, &final_category(b, &manual, &llm));
        if bucket_for_category(&bucket_map, &cat) == "distracting" {
            let label = b.domain.clone().unwrap_or_else(|| b.label.clone());
            let min = b.seconds / 60;
            if best.as_ref().map_or(true, |(_, m)| min > *m) {
                best = Some((label, min));
            }
        }
    }
    best
}

/// Assemble the inputs the lock-in engine needs to draft tomorrow's plan.
pub fn gather_plan_inputs(conn: &Connection, day: &str) -> lockin::PlanInputs {
    let stats = compute_stats_for_day(conn, day).unwrap_or_default();
    let checkins = effective_checkins(conn, day);
    let outputs = output_signals_for_day(conn, day);
    let report =
        scoring::build_report(conn, day.to_string(), &stats, &checkins, &outputs, top_project_name(conn));

    let mut completed = Vec::new();
    let mut missed = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT title, completed FROM goals WHERE day = ?1
         ORDER BY CASE priority WHEN 'high' THEN 0 WHEN 'medium' THEN 1 ELSE 2 END, sort_order, id",
    ) {
        if let Ok(rows) = stmt.query_map([day], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0))) {
            for (title, done) in rows.flatten() {
                if done {
                    completed.push(title);
                } else {
                    missed.push(title);
                }
            }
        }
    }

    let recurring_goals: Vec<String> = {
        let mut v = Vec::new();
        if let Ok(mut stmt) =
            conn.prepare("SELECT DISTINCT title FROM goals WHERE recurring = 1 ORDER BY day DESC LIMIT 5")
        {
            if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                v = rows.filter_map(Result::ok).collect();
            }
        }
        v
    };

    let notes: String = conn
        .query_row("SELECT notes FROM daily_checkin WHERE day = ?1", [day], |r| {
            r.get::<_, Option<String>>(0)
        })
        .ok()
        .flatten()
        .unwrap_or_default();

    lockin::PlanInputs {
        score: report.score,
        verdict: report.verdict,
        completed_goals: completed,
        missed_goals: missed,
        top_distraction: top_distraction_for_day(conn, day),
        video_exports: outputs.video_exports,
        code_changes: outputs.code_changes,
        first_productive_min: stats.first_productive_min,
        recurring_goals,
        notes,
    }
}

/// Upsert a lock-in plan for its day.
pub fn store_plan(conn: &Connection, p: &LockinPlan) -> Result<(), String> {
    let sec = serde_json::to_string(&p.secondary_missions).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "INSERT INTO lockin_plans
           (day, main_mission, secondary_missions, first_block, distraction_rule, focus_mode,
            avoid_trap, roast_line, source, edited, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(day) DO UPDATE SET
           main_mission = excluded.main_mission, secondary_missions = excluded.secondary_missions,
           first_block = excluded.first_block, distraction_rule = excluded.distraction_rule,
           focus_mode = excluded.focus_mode, avoid_trap = excluded.avoid_trap,
           roast_line = excluded.roast_line, source = excluded.source, edited = excluded.edited,
           created_at = excluded.created_at",
        params![
            p.day, p.main_mission, sec, p.first_block, p.distraction_rule, p.focus_mode,
            p.avoid_trap, p.roast_line, p.source, p.edited as i64, Utc::now().to_rfc3339(),
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Load the stored lock-in plan for a day, if one exists.
pub fn load_plan(conn: &Connection, day: &str) -> Option<LockinPlan> {
    conn.query_row(
        "SELECT day, main_mission, secondary_missions, first_block, distraction_rule, focus_mode,
                avoid_trap, roast_line, source, edited
         FROM lockin_plans WHERE day = ?1",
        [day],
        |r| {
            let sec: String = r.get(2)?;
            Ok(LockinPlan {
                day: r.get(0)?,
                main_mission: r.get(1)?,
                secondary_missions: serde_json::from_str(&sec).unwrap_or_default(),
                first_block: r.get(3)?,
                distraction_rule: r.get(4)?,
                focus_mode: r.get(5)?,
                avoid_trap: r.get(6)?,
                roast_line: r.get(7)?,
                source: r.get(8)?,
                edited: r.get::<_, i64>(9)? != 0,
            })
        },
    )
    .optional()
    .ok()
    .flatten()
}

/// Generate (LLM or fallback) and persist the lock-in plan drafted from `day`.
pub fn generate_plan_core(conn: &Connection, day: &str) -> LockinPlan {
    let inputs = gather_plan_inputs(conn, day);
    let enabled = settings::get_bool(conn, settings::LLM_ENABLED, false);
    let plan = if enabled {
        let url = settings::get_setting(conn, settings::OLLAMA_URL)
            .unwrap_or_else(|| settings::DEFAULT_OLLAMA_URL.to_string());
        let model = settings::get_setting(conn, settings::OLLAMA_MODEL)
            .unwrap_or_else(|| settings::DEFAULT_OLLAMA_MODEL.to_string());
        let cfg = llm::OllamaConfig { url, model };
        match llm::generate_json(&cfg, &lockin::build_prompt(&inputs), 0.7)
            .and_then(|j| lockin::parse_plan(day, &j))
        {
            Ok(p) => p,
            Err(e) => {
                let _ = conn.execute(
                    "INSERT INTO llm_errors (timestamp, context, message) VALUES (?1, 'lockin_plan', ?2)",
                    params![Utc::now().to_rfc3339(), e],
                );
                lockin::fallback_plan(day, &inputs)
            }
        }
    } else {
        lockin::fallback_plan(day, &inputs)
    };
    let _ = store_plan(conn, &plan);
    plan
}

/// Copy a day's plan missions into the *next* day's goal list, returning the count added.
pub fn copy_plan_core(conn: &Connection, day: &str) -> Result<i64, String> {
    let Some(plan) = load_plan(conn, day) else {
        return Ok(0);
    };
    // The plan generated FROM `day` is for the following day.
    let target = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map(|d| (d + Duration::days(1)).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| today());

    let mut items: Vec<(String, &str)> = vec![(plan.main_mission.clone(), "high")];
    for s in &plan.secondary_missions {
        items.push((s.clone(), "medium"));
    }

    let mut order = next_sort_order(conn, &target);
    let mut count = 0i64;
    for (title, priority) in items {
        let t = title.trim();
        if t.is_empty() || goal_exists(conn, &target, t) {
            continue;
        }
        if conn
            .execute(
                "INSERT INTO goals (day, title, project, target_minutes, priority, completed, sort_order, recurring, created_at)
                 VALUES (?1, ?2, NULL, NULL, ?3, 0, ?4, 0, ?5)",
                params![target, t, priority, order, Utc::now().to_rfc3339()],
            )
            .is_ok()
        {
            order += 1;
            count += 1;
        }
    }
    Ok(count)
}

// --------------------------------------------------------------- daily review

struct ReviewInput {
    date: String,
    score: i64,
    verdict: String,
    main_goal_name: Option<String>,
    main_goal_completed: bool,
    completed: Vec<String>,
    missed: Vec<String>,
    productive_min: i64,
    study_min: i64,
    business_min: i64,
    distracting_min: i64,
    top_productive: Vec<(String, i64)>,
    top_distracting: Vec<(String, i64)>,
    biggest_focus: Option<(String, i64)>,
    biggest_distraction: Option<(String, i64)>,
    goals_done: Vec<String>,
    goals_todo: Vec<String>,
    logged: Vec<String>,
    notes: String,
}

/// Today's goals split into completed / still-open lists (title + target hint).
fn goal_lines(conn: &Connection, day: &str) -> (Vec<String>, Vec<String>) {
    let mut done = Vec::new();
    let mut todo = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT title, target_minutes, target_count, target_unit, completed FROM goals WHERE day = ?1
         ORDER BY CASE priority WHEN 'high' THEN 0 WHEN 'medium' THEN 1 ELSE 2 END, sort_order, id",
    ) {
        let rows = stmt.query_map([day], |r| {
            let title: String = r.get(0)?;
            let target: Option<i64> = r.get(1)?;
            let target_count: Option<i64> = r.get(2)?;
            let target_unit: Option<String> = r.get(3)?;
            let completed: i64 = r.get(4)?;
            let label = match (target, target_count, target_unit) {
                (Some(m), _, _) if m > 0 => format!("{title} ({m}m)"),
                (_, Some(n), Some(unit)) if n > 0 => format!("{title} ({n} {unit})"),
                _ => title,
            };
            Ok((label, completed != 0))
        });
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                if row.1 {
                    done.push(row.0);
                } else {
                    todo.push(row.0);
                }
            }
        }
    }
    (done, todo)
}

/// Human-readable list of the quick check-ins the user logged today.
fn logged_checkins(values: &[CheckinValue]) -> Vec<String> {
    values
        .iter()
        .filter(|c| c.value > 0)
        .map(|c| {
            if c.kind == "counter" && c.value > 1 {
                format!("{} ×{}", c.label.to_lowercase(), c.value)
            } else {
                c.label.to_lowercase()
            }
        })
        .collect()
}

fn get_daily_note(conn: &Connection, day: &str) -> String {
    conn.query_row("SELECT notes FROM daily_checkin WHERE day = ?1", [day], |r| {
        r.get::<_, Option<String>>(0)
    })
    .ok()
    .flatten()
    .unwrap_or_default()
}

fn top_n(map: HashMap<String, i64>, n: usize) -> Vec<(String, i64)> {
    let mut v: Vec<(String, i64)> = map
        .into_iter()
        .map(|(k, s)| (k, s / 60))
        .filter(|(_, m)| *m > 0)
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(n);
    v
}

fn build_review_input(conn: &Connection) -> Result<(ScoreReport, ReviewInput), String> {
    let day = today();
    let llm_cache = llm::load_today_cache(conn);
    let manual = load_manual_corrections(conn, &day)?;
    let blocks = collect_blocks(conn)?;

    let bucket_map = category_buckets(conn);
    let targets = target_metrics(conn);
    let mut cat_seconds: HashMap<String, i64> = HashMap::new();
    let mut target_seconds: HashMap<String, i64> = HashMap::new();
    let mut first_prod: Option<i64> = None;
    let mut prod_by_label: HashMap<String, i64> = HashMap::new();
    let mut dist_by_label: HashMap<String, i64> = HashMap::new();
    let mut biggest_focus: Option<(String, i64)> = None;
    let mut biggest_dist: Option<(String, i64)> = None;

    for b in &blocks {
        if b.seconds <= 0 {
            continue;
        }
        let cat = category_or_fallback(conn, &final_category(b, &manual, &llm_cache));
        let bucket = bucket_for_category(&bucket_map, &cat);
        *cat_seconds.entry(cat).or_insert(0) += b.seconds;
        if bucket != "productive" {
            let label = b.domain.clone().unwrap_or_else(|| b.label.clone()).to_ascii_lowercase();
            for t in &targets {
                if label.contains(t.as_str()) {
                    *target_seconds.entry(t.clone()).or_insert(0) += b.seconds;
                }
            }
        }
        if bucket == "productive" {
            if let Some(m) = local_minutes(&b.first_seen) {
                first_prod = Some(first_prod.map_or(m, |c| c.min(m)));
            }
            *prod_by_label.entry(b.label.clone()).or_insert(0) += b.seconds;
            if biggest_focus.as_ref().map_or(true, |(_, s)| b.seconds > *s) {
                biggest_focus = Some((b.label.clone(), b.seconds));
            }
        } else if bucket == "distracting" {
            *dist_by_label.entry(b.label.clone()).or_insert(0) += b.seconds;
            if biggest_dist.as_ref().map_or(true, |(_, s)| b.seconds > *s) {
                biggest_dist = Some((b.label.clone(), b.seconds));
            }
        }
    }

    let stats = scoring::Stats {
        cat_seconds: cat_seconds.clone(),
        target_seconds,
        first_productive_min: first_prod,
    };
    let checkins = effective_checkins(conn, &day);
    let goal = top_project_name(conn);
    let outputs = output_signals_for_day(conn, &day);
    let report = scoring::build_report(conn, day.clone(), &stats, &checkins, &outputs, goal.clone());

    let completed = report.lines.iter().filter(|l| l.positive && l.triggered).map(|l| l.label.clone()).collect();
    let missed = report.lines.iter().filter(|l| l.positive && !l.triggered).map(|l| l.label.clone()).collect();
    let mins = |c: &str| cat_seconds.get(c).copied().unwrap_or(0) / 60;
    let (goals_done, goals_todo) = goal_lines(conn, &day);
    let logged = logged_checkins(&checkin_values_for_day(conn, &day));

    let input = ReviewInput {
        date: report.date.clone(),
        score: report.score,
        verdict: report.verdict.clone(),
        main_goal_name: goal,
        main_goal_completed: checkins.main_goal_completed,
        completed,
        missed,
        productive_min: mins("productive"),
        study_min: mins("study"),
        business_min: mins("business"),
        distracting_min: mins("distraction"),
        top_productive: top_n(prod_by_label, 3),
        top_distracting: top_n(dist_by_label, 3),
        biggest_focus: biggest_focus.map(|(l, s)| (l, s / 60)),
        biggest_distraction: biggest_dist.map(|(l, s)| (l, s / 60)),
        goals_done,
        goals_todo,
        logged,
        notes: get_daily_note(conn, &day),
    };
    Ok((report, input))
}

fn build_review_prompt(i: &ReviewInput) -> String {
    let list = |v: &[String]| if v.is_empty() { "none".to_string() } else { v.join("; ") };
    let apps = |v: &[(String, i64)]| {
        if v.is_empty() {
            "none".to_string()
        } else {
            v.iter().map(|(l, m)| format!("{l} {m}m")).collect::<Vec<_>>().join(", ")
        }
    };
    let blk = |b: &Option<(String, i64)>| {
        b.as_ref().map(|(l, m)| format!("{l} {m}m")).unwrap_or_else(|| "none".to_string())
    };
    let notes = if i.notes.trim().is_empty() { "none".to_string() } else { i.notes.clone() };

    format!(
        "You are the user's blunt, funny accountability coach. Write a short daily review.\n\
         Tone: casual, blunt, funny, motivating. NOT cringe. NO corporate productivity buzzwords \
         (no 'leverage', 'synergy', 'circle back', 'crush it', 'productivity journey'). Light swearing is fine.\n\
         Roast-line style example: \"Decent day. You actually shipped, but Instagram still robbed you like a Maltese parking ticket.\"\n\
         Return ONLY a JSON object with keys: verdict (one sentence), wins (array of exactly 3 short strings), \
         problems (array of exactly 3 short strings), tomorrow (one concrete goal), roast (one short funny line).\n\
         Keep the whole thing under 200 words. Reference the actual data.\n\n\
         Today's data:\n\
         - score: {}/100 ({})\n\
         - main goal: {} — {}\n\
         - goals completed: {}\n\
         - goals not done: {}\n\
         - manual check-ins logged: {}\n\
         - score wins: {}\n\
         - score missed: {}\n\
         - productive/coding: {}m, study: {}m, business: {}m, distracting: {}m\n\
         - top productive apps/sites: {}\n\
         - top distracting apps/sites: {}\n\
         - biggest uninterrupted focus block: {}\n\
         - biggest distraction block: {}\n\
         - user notes: {}\n\n\
         JSON:",
        i.score,
        i.verdict,
        i.main_goal_name.as_deref().unwrap_or("none"),
        if i.main_goal_completed { "completed" } else { "not completed" },
        list(&i.goals_done),
        list(&i.goals_todo),
        list(&i.logged),
        list(&i.completed),
        list(&i.missed),
        i.productive_min,
        i.study_min,
        i.business_min,
        i.distracting_min,
        apps(&i.top_productive),
        apps(&i.top_distracting),
        blk(&i.biggest_focus),
        blk(&i.biggest_distraction),
        notes,
    )
}

fn clean_list(v: Option<Vec<String>>, n: usize) -> Vec<String> {
    v.unwrap_or_default()
        .into_iter()
        .map(|s| s.trim().chars().take(140).collect::<String>())
        .filter(|s| !s.is_empty())
        .take(n)
        .collect()
}

fn parse_review(json: &str, input: &ReviewInput, model: &str) -> Result<DailyAiReview, String> {
    #[derive(serde::Deserialize)]
    struct Raw {
        verdict: Option<String>,
        wins: Option<Vec<String>>,
        problems: Option<Vec<String>>,
        tomorrow: Option<String>,
        roast: Option<String>,
    }
    let raw: Raw = serde_json::from_str(json.trim()).map_err(|e| format!("invalid JSON: {e}"))?;
    let verdict: String = raw.verdict.unwrap_or_default().trim().chars().take(200).collect();
    if verdict.is_empty() {
        return Err("empty verdict".into());
    }
    Ok(DailyAiReview {
        date: input.date.clone(),
        verdict,
        wins: clean_list(raw.wins, 3),
        problems: clean_list(raw.problems, 3),
        tomorrow: raw.tomorrow.unwrap_or_default().trim().chars().take(200).collect(),
        roast: raw.roast.unwrap_or_default().trim().chars().take(200).collect(),
        source: "llm".into(),
        model: Some(model.to_string()),
        generated_at: Some(Utc::now().to_rfc3339()),
        notes: input.notes.clone(),
    })
}

fn fallback_review(report: &ScoreReport, input: &ReviewInput) -> DailyAiReview {
    let verdict = match report.verdict.as_str() {
        "excellent" => "Locked in today — genuinely strong.",
        "good" => "Solid day. Not flawless, but you moved.",
        "mid" => "Mid. You coasted more than you'd admit.",
        "bad" => "Rough one. Time to regroup.",
        _ => "Cooked. Tomorrow is a hard reset.",
    }
    .to_string();

    // Completed goals lead the wins; score wins fill the rest.
    let mut wins: Vec<String> =
        input.goals_done.iter().take(3).map(|g| format!("Finished: {g}")).collect();
    for l in &report.top_wins {
        if wins.len() >= 3 {
            break;
        }
        wins.push(l.label.clone());
    }
    if wins.is_empty() {
        wins.push("You showed up and tracked the day.".into());
    }
    wins.truncate(3);

    // Unfinished goals lead the problems; score leaks fill the rest.
    let mut problems: Vec<String> =
        input.goals_todo.iter().take(2).map(|g| format!("Didn't finish: {g}")).collect();
    for l in &report.biggest_leaks {
        if problems.len() >= 3 {
            break;
        }
        problems.push(format!("{} ({})", l.label, l.value));
    }
    if problems.is_empty() {
        problems.push("Nothing major — don't get smug.".into());
    }
    problems.truncate(3);

    let roast = match &input.biggest_distraction {
        Some((l, m)) => format!("{l} ate {m}m of your day. Riveting content, I'm sure."),
        None => "No big leaks today — suspicious, but I'll allow it.".into(),
    };

    let tomorrow = match input.goals_todo.first() {
        Some(g) => format!("Knock out: {g}"),
        None => report.suggestion.clone(),
    };

    DailyAiReview {
        date: input.date.clone(),
        verdict,
        wins,
        problems,
        tomorrow,
        roast,
        source: "fallback".into(),
        model: None,
        generated_at: None,
        notes: input.notes.clone(),
    }
}

fn load_cached_review(conn: &Connection, day: &str) -> Option<DailyAiReview> {
    conn.query_row(
        "SELECT verdict, wins, problems, tomorrow, roast, source, model, created_at
         FROM daily_review WHERE day = ?1",
        [day],
        |r| {
            Ok(DailyAiReview {
                date: day.to_string(),
                verdict: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                wins: parse_keywords(r.get::<_, Option<String>>(1)?),
                problems: parse_keywords(r.get::<_, Option<String>>(2)?),
                tomorrow: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                roast: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                source: r.get::<_, Option<String>>(5)?.unwrap_or_else(|| "fallback".into()),
                model: r.get::<_, Option<String>>(6)?,
                generated_at: r.get::<_, Option<String>>(7)?,
                notes: String::new(),
            })
        },
    )
    .ok()
}

fn store_review(conn: &Connection, day: &str, r: &DailyAiReview) -> Result<(), String> {
    let wins = serde_json::to_string(&r.wins).unwrap_or_else(|_| "[]".into());
    let problems = serde_json::to_string(&r.problems).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "INSERT INTO daily_review (day, verdict, wins, problems, tomorrow, roast, source, model, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(day) DO UPDATE SET
             verdict=excluded.verdict, wins=excluded.wins, problems=excluded.problems,
             tomorrow=excluded.tomorrow, roast=excluded.roast, source=excluded.source,
             model=excluded.model, created_at=excluded.created_at",
        params![day, r.verdict, wins, problems, r.tomorrow, r.roast, r.source, r.model, r.generated_at],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Read-path: cached review if present, else a data-driven fallback (no LLM call).
pub fn daily_review(conn: &Connection) -> Result<DailyAiReview, String> {
    let day = today();
    let notes = get_daily_note(conn, &day);
    if let Some(mut r) = load_cached_review(conn, &day) {
        r.notes = notes;
        return Ok(r);
    }
    let (report, input) = build_review_input(conn)?;
    let mut r = fallback_review(&report, &input);
    r.notes = notes;
    Ok(r)
}

/// Generate-path: try the LLM (when enabled), fall back to a data-driven review,
/// persist the result, and return it.
pub fn generate_review(conn: &Connection) -> Result<DailyAiReview, String> {
    let day = today();
    let (report, input) = build_review_input(conn)?;

    let enabled = settings::get_bool(conn, settings::LLM_ENABLED, false);
    let url = settings::get_setting(conn, settings::OLLAMA_URL)
        .unwrap_or_else(|| settings::DEFAULT_OLLAMA_URL.to_string());
    let model = settings::get_setting(conn, settings::OLLAMA_MODEL)
        .unwrap_or_else(|| settings::DEFAULT_OLLAMA_MODEL.to_string());

    let review = if enabled {
        let cfg = llm::OllamaConfig { url, model: model.clone() };
        match llm::generate_json(&cfg, &build_review_prompt(&input), 0.6)
            .and_then(|j| parse_review(&j, &input, &model))
        {
            Ok(r) => r,
            Err(e) => {
                // Log locally and fall back to a data-driven review (no crash).
                let _ = conn.execute(
                    "INSERT INTO llm_errors (timestamp, context, message) VALUES (?1, 'daily_review', ?2)",
                    params![Utc::now().to_rfc3339(), e],
                );
                fallback_review(&report, &input)
            }
        }
    } else {
        fallback_review(&report, &input)
    };

    store_review(conn, &day, &review)?;
    Ok(review)
}

// --------------------------------------------------------------- focus sessions

pub const FOCUS_COLS: &str =
    "id, goal, started_at, duration_minutes, ends_at, allowed, blocked, status, ended_at";

pub fn row_to_focus(r: &rusqlite::Row) -> rusqlite::Result<FocusSession> {
    let ends_at: String = r.get(4)?;
    let status: String = r.get(7)?;
    let remaining = if status == "active" {
        chrono::DateTime::parse_from_rfc3339(&ends_at)
            .map(|e| (e.with_timezone(&Utc) - Utc::now()).num_seconds().max(0))
            .unwrap_or(0)
    } else {
        0
    };
    Ok(FocusSession {
        id: r.get(0)?,
        goal: r.get(1)?,
        started_at: r.get(2)?,
        duration_minutes: r.get(3)?,
        ends_at,
        allowed: parse_keywords(r.get::<_, Option<String>>(5)?),
        blocked: parse_keywords(r.get::<_, Option<String>>(6)?),
        status,
        ended_at: r.get(8)?,
        remaining_seconds: remaining,
    })
}

fn list_match(list: &[String], key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    list.iter().any(|item| {
        let it = item.trim().to_ascii_lowercase();
        !it.is_empty() && (k == it || k.contains(&it) || it.contains(&k))
    })
}

fn quick_category(conn: &Connection, key: &str, is_web: bool) -> String {
    if is_web {
        if let Ok(Some(c)) = conn.query_row(
            "SELECT category FROM domain_rules WHERE domain = ?1",
            [key],
            |r| r.get::<_, Option<String>>(0),
        ) {
            if is_valid_category(&c) {
                return c;
            }
        }
    } else if let Ok(c) =
        conn.query_row("SELECT category FROM category_rules WHERE app_name = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
    {
        if is_valid_category(&c) {
            return c;
        }
    }
    classify::classify(key, "", None, None, &[], None).category
}

/// 0 = focused, 1 = distracted, 2 = other.
fn focus_class(bucket: &str, key: &str, allowed: &[String], blocked: &[String]) -> u8 {
    if list_match(blocked, key) {
        return 1;
    }
    if list_match(allowed, key) {
        return 0;
    }
    if !allowed.is_empty() {
        return if bucket == "distracting" { 1 } else { 2 };
    }
    match bucket {
        "productive" => 0,
        "distracting" => 1,
        _ => 2,
    }
}

/// One classified activity interval inside a focus window (unix seconds).
struct FocusSpan {
    start: i64,
    end: i64,
    class: u8,
}

/// Clip a raw activity row to the focus window, classify it, and (if it is a
/// distraction) credit its label so we can name the worst offender.
#[allow(clippy::too_many_arguments)]
fn add_focus_span(
    conn: &Connection,
    sess: &FocusSession,
    win_start: i64,
    win_end: i64,
    ts: &str,
    dur: i64,
    key: String,
    is_web: bool,
    spans: &mut Vec<FocusSpan>,
    dist_by: &mut HashMap<String, i64>,
) {
    if dur <= 0 {
        return;
    }
    let Some(t0) = parse_utc(ts).map(|t| t.timestamp()) else { return };
    let a = t0.max(win_start);
    let b = (t0 + dur).min(win_end);
    if b <= a {
        return;
    }
    let bucket = bucket_for(&quick_category(conn, &key, is_web)).to_string();
    let class = focus_class(&bucket, &key, &sess.allowed, &sess.blocked);
    if class == 1 {
        *dist_by.entry(key).or_insert(0) += b - a;
    }
    spans.push(FocusSpan { start: a, end: b, class });
}

/// Union the classified spans onto a single wall-clock timeline so overlapping
/// time — from multiple devices, or the app + browser streams of one device — is
/// counted once. When classes overlap, the most-accountable wins:
/// distracted (1) > focused (0) > other (2). Returns (focused, distracted, other).
fn merge_focus_spans(spans: &[FocusSpan]) -> (i64, i64, i64) {
    let mut points: Vec<i64> = Vec::with_capacity(spans.len() * 2);
    for s in spans {
        points.push(s.start);
        points.push(s.end);
    }
    points.sort_unstable();
    points.dedup();

    let (mut focused, mut distracted, mut other) = (0i64, 0i64, 0i64);
    for w in points.windows(2) {
        let (p, q) = (w[0], w[1]);
        if q <= p {
            continue;
        }
        let (mut has_focus, mut has_dist, mut has_other) = (false, false, false);
        for s in spans {
            if s.start <= p && s.end >= q {
                match s.class {
                    0 => has_focus = true,
                    1 => has_dist = true,
                    _ => has_other = true,
                }
            }
        }
        let len = q - p;
        if has_dist {
            distracted += len;
        } else if has_focus {
            focused += len;
        } else if has_other {
            other += len;
        }
    }
    (focused, distracted, other)
}

/// Adherence breakdown for a focus session. On the hub `activity_log` /
/// `browser_activity` hold every device's rows, so this is naturally a
/// cross-device view: a distraction on *any* device during the window counts
/// against the block, and overlapping time is never double-counted.
pub fn focus_summary(conn: &Connection, id: i64) -> Result<FocusSummary, String> {
    let sess = conn
        .query_row(&format!("SELECT {FOCUS_COLS} FROM focus_sessions WHERE id = ?1"), [id], row_to_focus)
        .map_err(|e| e.to_string())?;
    let start = sess.started_at.clone();
    let end = sess.ended_at.clone().unwrap_or_else(|| Utc::now().to_rfc3339());
    let win_start = parse_utc(&start).map(|t| t.timestamp()).unwrap_or(0);
    let win_end = parse_utc(&end).map(|t| t.timestamp()).unwrap_or(win_start);

    let mut spans: Vec<FocusSpan> = Vec::new();
    let mut dist_by: HashMap<String, i64> = HashMap::new();

    // App-window samples in the window (all devices).
    {
        let mut stmt = conn
            .prepare(
                "SELECT app_name, timestamp, duration_seconds FROM activity_log
                 WHERE is_idle = 0 AND timestamp >= ?1 AND timestamp <= ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![start, end], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
            })
            .map_err(|e| e.to_string())?;
        for (app, ts, dur) in rows.flatten() {
            add_focus_span(conn, &sess, win_start, win_end, &ts, dur, app, false, &mut spans, &mut dist_by);
        }
    }
    // Browser-tab samples in the window (all devices).
    {
        let mut stmt = conn
            .prepare(
                "SELECT domain, timestamp, duration_seconds FROM browser_activity
                 WHERE is_idle = 0 AND timestamp >= ?1 AND timestamp <= ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![start, end], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
            })
            .map_err(|e| e.to_string())?;
        for (domain, ts, dur) in rows.flatten() {
            add_focus_span(conn, &sess, win_start, win_end, &ts, dur, domain, true, &mut spans, &mut dist_by);
        }
    }

    let (focused, distracted, other) = merge_focus_spans(&spans);
    let tracked = focused + distracted + other;
    let adherence = if tracked > 0 { focused * 100 / tracked } else { 0 };
    let top_distraction = dist_by.into_iter().max_by_key(|(_, s)| *s).map(|(k, _)| k);

    Ok(FocusSummary {
        goal: sess.goal,
        duration_minutes: sess.duration_minutes,
        status: sess.status,
        focused_seconds: focused,
        distracted_seconds: distracted,
        other_seconds: other,
        top_distraction,
        adherence,
    })
}

/// The session a hub dashboard should summarize: the most recent one that
/// arrived today (sessions sync to the hub once they end). `None` outside a
/// focus day. Live active-session mirroring to other devices is future work.
pub fn latest_focus_session_today(conn: &Connection) -> Option<FocusSession> {
    let day = today();
    let mut s = conn
        .query_row(
            &format!(
                "SELECT {FOCUS_COLS} FROM focus_sessions
                 WHERE substr(started_at, 1, 10) = ?1 ORDER BY id DESC LIMIT 1"
            ),
            [day],
            row_to_focus,
        )
        .optional()
        .ok()
        .flatten()?;
    // Hub sessions are already finished. Present the real end time so a countdown
    // UI resolves straight to the summary instead of showing a phantom timer for a
    // session that was ended early (ends_at in the future, ended_at in the past).
    if s.status != "active" {
        if let Some(ended) = s.ended_at.clone() {
            s.ends_at = ended;
        }
        s.remaining_seconds = 0;
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline_test_block(
        label: &str,
        start: &str,
        duration_seconds: i64,
        category: &str,
        bucket: &str,
    ) -> TimelineBlock {
        let start_dt = parse_utc(start).unwrap();
        TimelineBlock {
            source: "desktop".to_string(),
            start: start_dt.to_rfc3339(),
            end: (start_dt + Duration::seconds(duration_seconds)).to_rfc3339(),
            duration_seconds,
            label: label.to_string(),
            title: label.to_string(),
            category: category.to_string(),
            bucket: bucket.to_string(),
            project: Some("Content Creation".to_string()),
            project_confidence: 90,
            confidence: 0.9,
            classifier: "rule".to_string(),
            idle: false,
            summary: None,
            block_key: format!("test|{label}|{start}"),
            is_web: false,
            sample_count: 1,
            absorbed_seconds: 0,
            absorbed_count: 0,
            longest_productive: false,
            biggest_distraction: false,
            first_productive: false,
            goal_related: false,
            output_linked: false,
        }
    }

    #[test]
    fn overview_absorbs_one_brief_switch_when_same_work_resumes() {
        let blocks = vec![
            timeline_test_block("DaVinci Resolve", "2026-07-28T12:00:00Z", 900, "business", "productive"),
            timeline_test_block("Telegram", "2026-07-28T12:15:00Z", 10, "distraction", "distracting"),
            timeline_test_block("DaVinci Resolve", "2026-07-28T12:15:10Z", 900, "business", "productive"),
        ];

        let overview = smooth_brief_interruptions(&blocks, 20);

        assert_eq!(overview.len(), 1);
        assert_eq!(overview[0].label, "DaVinci Resolve");
        assert_eq!(overview[0].duration_seconds, 1810);
        assert_eq!(overview[0].absorbed_seconds, 10);
        assert_eq!(overview[0].absorbed_count, 1);
        assert!(overview[0].longest_productive);
    }

    #[test]
    fn overview_keeps_a_meaningful_interruption() {
        let blocks = vec![
            timeline_test_block("DaVinci Resolve", "2026-07-28T12:00:00Z", 900, "business", "productive"),
            timeline_test_block("Telegram", "2026-07-28T12:15:00Z", 60, "distraction", "distracting"),
            timeline_test_block("DaVinci Resolve", "2026-07-28T12:16:00Z", 900, "business", "productive"),
        ];

        assert_eq!(smooth_brief_interruptions(&blocks, 20).len(), 3);
    }

    #[test]
    fn overview_never_merges_different_surrounding_work() {
        let blocks = vec![
            timeline_test_block("DaVinci Resolve", "2026-07-28T12:00:00Z", 900, "business", "productive"),
            timeline_test_block("Telegram", "2026-07-28T12:15:00Z", 10, "distraction", "distracting"),
            timeline_test_block("Adobe Premiere Pro", "2026-07-28T12:15:10Z", 900, "business", "productive"),
        ];

        assert_eq!(smooth_brief_interruptions(&blocks, 20).len(), 3);
    }

    #[test]
    fn weak_ocr_keyword_does_not_assign_unrelated_project() {
        let conn = crate::db::test_conn();
        crate::settings::ensure_defaults(&conn).unwrap();
        let day = "2026-07-28";
        conn.execute(
            "INSERT INTO projects (name, category, keywords, apps, domains, priority, updated_at)
             VALUES ('AFM Business', 'business', '[\"tate\"]', '[]', '[]', 100, 't')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO smart_activity
               (timestamp, day, app_name, window_title, ocr_summary, detected_keywords, category, is_idle)
             VALUES ('2026-07-28T12:00:00Z', ?1, 'Telegram Desktop', 'Anonymous Chat',
                     'Chat list includes Tatespeech', '[\"tate\"]', 'distraction', 0)",
            [day],
        ).unwrap();

        let timeline = timeline_for_day(&conn, day, 20).unwrap();
        assert_eq!(timeline.blocks.len(), 1);
        assert_eq!(timeline.blocks[0].label, "Telegram Desktop");
        assert!(timeline.blocks[0].project.is_none());
        assert_eq!(timeline.blocks[0].project_confidence, 0);
        assert!(!timeline.blocks[0].goal_related);
    }

    #[test]
    fn smart_context_replaces_overlapping_desktop_sample() {
        let conn = crate::db::test_conn();
        crate::settings::ensure_defaults(&conn).unwrap();
        let day = "2026-07-28";
        let ts = "2026-07-28T12:00:00Z";
        conn.execute(
            "INSERT INTO activity_log
               (timestamp, day, app_name, window_title, duration_seconds, is_idle)
             VALUES (?1, ?2, 'Tempo', 'Tempo', 10, 0)",
            params![ts, day],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO smart_activity
               (timestamp, day, app_name, window_title, ocr_summary,
                detected_keywords, category, is_idle)
             VALUES (?1, ?2, 'Tempo', 'Tempo', 'Tempo dashboard', '[]', 'neutral', 0)",
            params![ts, day],
        )
        .unwrap();

        let timeline = timeline_for_day(&conn, day, 15).unwrap();
        let tempo: Vec<_> = timeline.blocks.iter().filter(|b| b.label == "Tempo").collect();
        assert_eq!(tempo.len(), 1);
        assert_eq!(tempo[0].source, "screen");
        assert_eq!(tempo[0].category, "neutral");
        assert!(tempo[0].project.is_none());
    }
}