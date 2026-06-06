//! Projects / goals: user-defined intents used to classify activity.
//!
//! Matching combines two signal types — an app/domain match and title/content
//! keyword hits — into a confidence score. More signals ⇒ higher confidence.
//! A match at or above `OVERRIDE_THRESHOLD` lets the project's category replace
//! the content-derived category.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// Confidence at/above which the project's category overrides the activity's.
/// 50 = a bare app/domain match (won't override); 60 needs a second signal.
pub const OVERRIDE_THRESHOLD: u8 = 60;

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    #[serde(default)]
    pub id: i64,
    pub name: String,
    pub category: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub apps: Vec<String>,
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default = "default_priority")]
    pub priority: i64,
}

fn default_priority() -> i64 {
    50
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMatch {
    pub project_id: i64,
    pub project_name: String,
    pub category: String,
    pub confidence: u8,
    pub signals: Vec<String>,
}

// ------------------------------------------------------------------ matching

/// Collapse to lowercase alphanumerics so "ChatGPT", "chatgpt.com" and
/// "chat gpt" all compare equal-ish.
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Best-matching project for an activity, or None. `identifier` is the app name
/// or domain; `title` is the window/page title; `extra` is any extra text
/// (e.g. a page's summary + keywords).
pub fn match_project(
    projects: &[Project],
    identifier: &str,
    title: &str,
    extra: &str,
) -> Option<ProjectMatch> {
    let id_norm = normalize(identifier);
    let hay = format!("{title} {extra}").to_ascii_lowercase();

    let mut best: Option<(i64, i64, ProjectMatch)> = None; // (score, priority, match)

    for p in projects {
        let mut signals = Vec::new();

        let id_match = !id_norm.is_empty()
            && p.apps.iter().chain(p.domains.iter()).any(|entry| {
                let n = normalize(entry);
                n.len() >= 3 && (id_norm.contains(&n) || n.contains(&id_norm))
            });
        if id_match {
            signals.push(format!("app/domain: {identifier}"));
        }

        let mut kw_hits = 0u32;
        for kw in &p.keywords {
            let k = kw.trim().to_ascii_lowercase();
            if k.len() >= 2 && hay.contains(&k) {
                signals.push(format!("keyword: {kw}"));
                kw_hits += 1;
            }
        }

        if !id_match && kw_hits == 0 {
            continue;
        }

        // Confidence: stacks with each independent signal.
        let mut conf: u32 = 0;
        if id_match {
            conf += 50;
        }
        if kw_hits >= 1 {
            conf += 35;
        }
        if kw_hits >= 2 {
            conf += 20;
        }
        if kw_hits >= 3 {
            conf += 10;
        }
        if kw_hits >= 4 {
            conf += 10;
        }
        let confidence = conf.min(100) as u8;

        let score = (if id_match { 2 } else { 0 }) + kw_hits as i64;
        let m = ProjectMatch {
            project_id: p.id,
            project_name: p.name.clone(),
            category: p.category.clone(),
            confidence,
            signals,
        };

        if best
            .as_ref()
            .map_or(true, |(s, pr, _)| (score, p.priority) > (*s, *pr))
        {
            best = Some((score, p.priority, m));
        }
    }

    best.map(|(_, _, m)| m)
}

/// Apply a project match on top of a base (category, reason). Returns the
/// possibly-overridden category/reason plus the match (if any) for display.
pub fn resolve(
    projects: &[Project],
    identifier: &str,
    title: &str,
    extra: &str,
    base_category: &str,
    base_reason: &str,
) -> (String, String, Option<ProjectMatch>) {
    match match_project(projects, identifier, title, extra) {
        Some(m) if m.confidence >= OVERRIDE_THRESHOLD => {
            let reason = format!("project: {} ({}%)", m.project_name, m.confidence);
            (m.category.clone(), reason, Some(m))
        }
        other => (base_category.to_string(), base_reason.to_string(), other),
    }
}

// ---------------------------------------------------------------------- CRUD

fn parse_arr(s: String) -> Vec<String> {
    serde_json::from_str(&s).unwrap_or_default()
}

fn arr_json(v: &[String]) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn list_projects(conn: &Connection) -> rusqlite::Result<Vec<Project>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, category, keywords, apps, domains, priority
         FROM projects ORDER BY priority DESC, name",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Project {
            id: r.get(0)?,
            name: r.get(1)?,
            category: r.get(2)?,
            keywords: parse_arr(r.get::<_, String>(3)?),
            apps: parse_arr(r.get::<_, String>(4)?),
            domains: parse_arr(r.get::<_, String>(5)?),
            priority: r.get(6)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn create_project(conn: &Connection, p: &Project) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO projects (name, category, keywords, apps, domains, priority, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            p.name,
            p.category,
            arr_json(&p.keywords),
            arr_json(&p.apps),
            arr_json(&p.domains),
            p.priority,
            now()
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_project(conn: &Connection, p: &Project) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE projects SET name=?1, category=?2, keywords=?3, apps=?4, domains=?5,
             priority=?6, updated_at=?7 WHERE id=?8",
        params![
            p.name,
            p.category,
            arr_json(&p.keywords),
            arr_json(&p.apps),
            arr_json(&p.domains),
            p.priority,
            now(),
            p.id
        ],
    )?;
    Ok(())
}

pub fn delete_project(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM projects WHERE id=?1", params![id])?;
    Ok(())
}

/// Seed example projects on first run only (so deletions persist).
pub fn ensure_default_projects(conn: &Connection) -> rusqlite::Result<()> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))?;
    if count > 0 {
        return Ok(());
    }

    let defaults: &[(&str, &str, &[&str], &[&str], &[&str], i64)] = &[
        (
            "Exam studying",
            "study",
            &["Hungarian Algorithm", "Stable Marriage", "LCS", "suffix trees", "skip lists"],
            &["RemNote", "PDF reader", "ChatGPT"],
            &["chatgpt.com", "remnote.com"],
            80,
        ),
        (
            "Video content business",
            "business",
            &["Premiere", "CapCut", "DaVinci", "hooks", "captions", "TikTok", "Instagram upload", "YouTube Shorts"],
            &["Premiere Pro", "CapCut", "DaVinci Resolve"],
            &["tiktok.com", "instagram.com", "youtube.com"],
            70,
        ),
        (
            "Coding",
            "productive",
            &["bug", "refactor", "compile", "function", "repository", "pull request"],
            &["Visual Studio Code", "Terminal", "IntelliJ IDEA"],
            &["github.com", "stackoverflow.com"],
            75,
        ),
        (
            "Fitness",
            "recovery",
            &["workout", "exercise", "reps", "sets", "protein", "gym"],
            &["Strava"],
            &["strava.com", "myfitnesspal.com"],
            50,
        ),
    ];

    for (name, category, keywords, apps, domains, priority) in defaults {
        let p = Project {
            id: 0,
            name: name.to_string(),
            category: category.to_string(),
            keywords: keywords.iter().map(|s| s.to_string()).collect(),
            apps: apps.iter().map(|s| s.to_string()).collect(),
            domains: domains.iter().map(|s| s.to_string()).collect(),
            priority: *priority,
        };
        create_project(conn, &p)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str, cat: &str, kw: &[&str], apps: &[&str], dom: &[&str], prio: i64) -> Project {
        Project {
            id: 0,
            name: name.into(),
            category: cat.into(),
            keywords: kw.iter().map(|s| s.to_string()).collect(),
            apps: apps.iter().map(|s| s.to_string()).collect(),
            domains: dom.iter().map(|s| s.to_string()).collect(),
            priority: prio,
        }
    }

    fn sample() -> Vec<Project> {
        vec![
            project(
                "Exam studying",
                "study",
                &["Hungarian Algorithm", "Stable Marriage", "LCS", "suffix trees", "skip lists"],
                &["RemNote", "PDF reader", "ChatGPT"],
                &["chatgpt.com", "remnote.com"],
                80,
            ),
            project(
                "Video content business",
                "business",
                &["hooks", "captions", "Premiere", "CapCut", "Instagram upload", "YouTube Shorts"],
                &["Premiere Pro", "CapCut"],
                &["tiktok.com", "instagram.com", "youtube.com"],
                70,
            ),
        ]
    }

    #[test]
    fn chatgpt_hungarian_matches_exam_studying_strongly() {
        let m = match_project(&sample(), "chatgpt.com", "Hungarian Algorithm — assignment problem", "minimum cost matching").unwrap();
        assert_eq!(m.project_name, "Exam studying");
        assert!(m.confidence >= 80, "confidence was {}", m.confidence);
    }

    #[test]
    fn youtube_hooks_overrides_to_business() {
        let (cat, _r, m) = resolve(
            &sample(),
            "youtube.com",
            "Short-form editing: hooks & retention",
            "viral analysis captions",
            "neutral",
            "content",
        );
        assert_eq!(cat, "business");
        assert!(m.unwrap().confidence >= 80);
    }

    #[test]
    fn instagram_feed_is_low_confidence_and_keeps_distraction() {
        // Domain-only match (50) shows the project but must NOT override category.
        let m = match_project(&sample(), "instagram.com", "Reels", "explore feed").unwrap();
        assert_eq!(m.project_name, "Video content business");
        assert_eq!(m.confidence, 50);
        let (cat, _r, _m) = resolve(&sample(), "instagram.com", "Reels", "explore feed", "distraction", "content");
        assert_eq!(cat, "distraction");
    }

    #[test]
    fn desktop_app_match_by_name() {
        let m = match_project(&sample(), "RemNote", "suffix trees and skip lists notes", "").unwrap();
        assert_eq!(m.project_name, "Exam studying");
        // app match (50) + 2 keywords (35+20) => capped high
        assert!(m.confidence >= 90, "confidence was {}", m.confidence);
    }

    #[test]
    fn no_signal_no_match() {
        assert!(match_project(&sample(), "example.com", "A page about gardening", "").is_none());
    }
}
