use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{Duration, NaiveDate};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::aggregate;

const MAX_RANGE_DAYS: i64 = 366;
const MAX_PRIVATE_SAMPLES: usize = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountabilityExportOptions {
    pub start_date: String,
    pub end_date: String,
    #[serde(default = "default_true")]
    pub include_activity_names: bool,
    #[serde(default)]
    pub include_titles: bool,
    #[serde(default)]
    pub include_raw_text: bool,
    #[serde(default)]
    pub include_notes: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountabilityExport {
    pub filename: String,
    pub markdown: String,
    pub start_date: String,
    pub end_date: String,
    pub day_count: i64,
    pub tracked_days: i64,
    pub active_seconds: i64,
}

#[derive(Default)]
struct DailyRow {
    day: String,
    active: i64,
    productive: i64,
    distracting: i64,
    idle: i64,
    switches: i64,
    goals_total: i64,
    goals_completed: i64,
    outputs: i64,
}

fn default_true() -> bool {
    true
}

fn parse_range(o: &AccountabilityExportOptions) -> Result<(NaiveDate, NaiveDate, i64), String> {
    let start = NaiveDate::parse_from_str(&o.start_date, "%Y-%m-%d")
        .map_err(|_| "Start date must use YYYY-MM-DD".to_string())?;
    let end = NaiveDate::parse_from_str(&o.end_date, "%Y-%m-%d")
        .map_err(|_| "End date must use YYYY-MM-DD".to_string())?;
    if end < start {
        return Err("End date must be on or after the start date".into());
    }
    let days = (end - start).num_days() + 1;
    if days > MAX_RANGE_DAYS {
        return Err(format!("Choose a range of {MAX_RANGE_DAYS} days or fewer"));
    }
    Ok((start, end, days))
}

fn duration(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let h = seconds / 3600;
    let m = (seconds % 3600) / 60;
    if h > 0 {
        format!("{h}h {m:02}m")
    } else {
        format!("{m}m")
    }
}

fn pct(part: i64, total: i64) -> i64 {
    if total <= 0 {
        0
    } else {
        ((part.max(0) as f64 / total as f64) * 100.0).round() as i64
    }
}

fn clean(value: &str) -> String {
    value
        .replace('|', "\\|")
        .replace(['\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate(value: &str, max: usize) -> String {
    let value = clean(value);
    if value.chars().count() <= max {
        value
    } else {
        format!("{}…", value.chars().take(max).collect::<String>())
    }
}

fn alias(
    raw: &str,
    source: &str,
    names: bool,
    aliases: &mut HashMap<String, String>,
    app: &mut usize,
    site: &mut usize,
) -> String {
    if names {
        return clean(raw);
    }
    let key = format!("{source}:{raw}");
    if let Some(value) = aliases.get(&key) {
        return value.clone();
    }
    let value = if source == "browser" {
        *site += 1;
        format!("Website #{site}")
    } else {
        *app += 1;
        format!("Desktop app #{app}")
    };
    aliases.insert(key, value.clone());
    value
}

fn top(map: &HashMap<String, i64>, limit: usize) -> Vec<(String, i64)> {
    let mut rows: Vec<_> = map.iter().map(|(k, v)| (k.clone(), *v)).collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    rows.truncate(limit);
    rows
}

fn table(out: &mut String, rows: &[(String, i64)], total: i64) {
    if rows.is_empty() {
        out.push_str("_No matching tracked activity._\n\n");
        return;
    }
    out.push_str("| Activity | Time | Share |\n|---|---:|---:|\n");
    for (name, seconds) in rows {
        out.push_str(&format!(
            "| {} | {} | {}% |\n",
            clean(name),
            duration(*seconds),
            pct(*seconds, total)
        ));
    }
    out.push('\n');
}

pub fn generate(
    conn: &Connection,
    o: &AccountabilityExportOptions,
) -> Result<AccountabilityExport, String> {
    let (start, end, day_count) = parse_range(o)?;
    let mut daily = Vec::new();
    let mut sources = HashMap::new();
    let mut distractions = HashMap::new();
    let mut productive = HashMap::new();
    let mut projects = HashMap::new();
    let mut categories = HashMap::new();
    let mut aliases = HashMap::new();
    let (mut app_index, mut site_index) = (0usize, 0usize);
    let mut title_samples = Vec::new();
    let mut seen_titles = HashSet::new();
    let (mut productive_blocks, mut productive_block_seconds) = (0i64, 0i64);

    let mut cursor = start;
    while cursor <= end {
        let day = cursor.format("%Y-%m-%d").to_string();
        let timeline = aggregate::timeline_for_day(conn, &day, 120)?;
        let mut row = DailyRow {
            day: day.clone(),
            active: timeline.active_seconds,
            productive: timeline.productive_seconds,
            distracting: timeline.distracted_seconds,
            idle: timeline.idle_seconds,
            switches: timeline.overview_blocks.len().saturating_sub(1) as i64,
            ..Default::default()
        };
        for block in &timeline.overview_blocks {
            if block.idle {
                continue;
            }
            let name = alias(
                &block.label,
                &block.source,
                o.include_activity_names,
                &mut aliases,
                &mut app_index,
                &mut site_index,
            );
            *sources.entry(name.clone()).or_insert(0) += block.duration_seconds;
            *categories.entry(block.category.clone()).or_insert(0) += block.duration_seconds;
            if block.bucket == "productive" {
                productive_blocks += 1;
                productive_block_seconds += block.duration_seconds;
                *productive.entry(name.clone()).or_insert(0) += block.duration_seconds;
            } else if block.bucket == "distracting" {
                *distractions.entry(name.clone()).or_insert(0) += block.duration_seconds;
            }
            if let Some(project) = &block.project {
                *projects.entry(clean(project)).or_insert(0) += block.duration_seconds;
            }
            if o.include_titles
                && !block.title.trim().is_empty()
                && title_samples.len() < MAX_PRIVATE_SAMPLES
            {
                let title = truncate(&block.title, 180);
                if seen_titles.insert(format!("{name}:{title}")) {
                    title_samples.push((day.clone(), name, title, block.bucket.clone()));
                }
            }
        }
        let goals: (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(CASE WHEN completed = 1 THEN 1 ELSE 0 END), 0) FROM goals WHERE day = ?1",
            [&day], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|e| e.to_string())?;
        row.goals_total = goals.0;
        row.goals_completed = goals.1;
        row.outputs = conn
            .query_row(
                "SELECT COUNT(*) FROM output_events WHERE day = ?1",
                [&day],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        daily.push(row);
        cursor += Duration::days(1);
    }

    let active: i64 = daily.iter().map(|d| d.active).sum();
    let productive_time: i64 = daily.iter().map(|d| d.productive).sum();
    let distracting: i64 = daily.iter().map(|d| d.distracting).sum();
    let neutral = (active - productive_time - distracting).max(0);
    let idle: i64 = daily.iter().map(|d| d.idle).sum();
    let tracked_days = daily.iter().filter(|d| d.active > 0).count() as i64;
    let switches: i64 = daily.iter().map(|d| d.switches).sum();
    let goals_total: i64 = daily.iter().map(|d| d.goals_total).sum();
    let goals_done: i64 = daily.iter().map(|d| d.goals_completed).sum();
    let outputs_total: i64 = daily.iter().map(|d| d.outputs).sum();

    let mut out = String::from("# Tempo accountability report\n\n");
    out.push_str("> **Suggested prompt for ChatGPT:** Be my brutally honest but practical accountability coach. Analyse the evidence below, identify where I am wasting time, distinguish genuine recovery from avoidance, challenge excuses, and give me the three highest-impact changes for the next seven days. Do not invent facts that are not in the report. Call out incomplete tracking or ambiguous classifications before drawing strong conclusions.\n\n");
    out.push_str("## Scope and privacy\n\n");
    out.push_str(&format!(
        "- Date range: **{} to {}** ({} calendar days; {} with tracked activity)\n",
        o.start_date, o.end_date, day_count, tracked_days
    ));
    out.push_str(&format!("- Activity names: **{}**\n- Window/page titles: **{}**\n- Daily notes: **{}**\n- Raw captured text excerpts: **{}**\n",
        if o.include_activity_names { "included" } else { "anonymised" }, if o.include_titles { "included" } else { "excluded" },
        if o.include_notes { "included" } else { "excluded" }, if o.include_raw_text { "included" } else { "excluded" }));
    out.push_str("- URL paths, file paths, tokens, secrets, and database identifiers are never exported.\n\n");

    out.push_str("## Executive snapshot\n\n");
    out.push_str(&format!(
        "- Active tracked time: **{}** (average **{}** per tracked day)\n",
        duration(active),
        duration(if tracked_days > 0 {
            active / tracked_days
        } else {
            0
        })
    ));
    out.push_str(&format!("- Productive: **{}** ({}%)\n- Neutral/ambiguous: **{}** ({}%)\n- Distracting: **{}** ({}%)\n- Detected idle/AFK: **{}** (excluded from active percentages)\n",
        duration(productive_time), pct(productive_time, active), duration(neutral), pct(neutral, active), duration(distracting), pct(distracting, active), duration(idle)));
    out.push_str(&format!("- Meaningful activity switches: **{}**\n- Average productive block: **{}**\n- Goals completed: **{}/{}**\n- Detected outputs: **{}**\n\n",
        switches, duration(if productive_blocks > 0 { productive_block_seconds / productive_blocks } else { 0 }), goals_done, goals_total, outputs_total));

    out.push_str("## Tempo flags worth investigating\n\n");
    let mut flags = Vec::new();
    if tracked_days == 0 {
        flags.push("There is no tracked activity in this range, so strong conclusions would be misleading.".to_string());
    } else {
        if active / tracked_days < 3600 {
            flags.push("Average tracked time is under one hour per tracked day; the dataset may be incomplete.".to_string());
        }
        if distracting > productive_time {
            flags.push(
                "Distracting time exceeds productive time across the selected range.".to_string(),
            );
        }
        if distracting / tracked_days >= 3600 {
            flags.push(
                "Recorded distraction averages at least one hour per tracked day.".to_string(),
            );
        }
        if neutral > productive_time && neutral > 3600 {
            flags.push("Neutral/ambiguous time exceeds productive time; review those classifications before treating it as harmless.".to_string());
        }
        if productive_blocks > 0 && productive_block_seconds / productive_blocks < 900 {
            flags.push("The average productive block is under 15 minutes, suggesting fragmentation or frequent switching.".to_string());
        }
        if goals_total > 0 && goals_done * 2 < goals_total {
            flags.push("Fewer than half of recorded goals were completed.".to_string());
        }
        if outputs_total == 0 && productive_time >= 14400 {
            flags.push("Substantial productive time was recorded without detected output; check whether the work produced a concrete result or output tracking is incomplete.".to_string());
        }
        if let Some((name, seconds)) = top(&distractions, 1).first() {
            if distracting > 0 && *seconds * 2 >= distracting {
                flags.push(format!(
                    "{} accounts for at least half of all recorded distraction.",
                    clean(name)
                ));
            }
        }
    }
    if flags.is_empty() {
        flags.push("No single high-confidence warning crossed Tempo's thresholds. Inspect the daily pattern and classifications rather than assuming everything is fine.".to_string());
    }
    for flag in flags {
        out.push_str(&format!("- {flag}\n"));
    }
    out.push('\n');

    out.push_str("## Biggest recorded distractions\n\n");
    table(&mut out, &top(&distractions, 12), distracting);
    out.push_str("## Most productive activity\n\n");
    table(&mut out, &top(&productive, 12), productive_time);
    out.push_str("## All top activity sources\n\n");
    table(&mut out, &top(&sources, 15), active);
    out.push_str("## Time by category\n\n");
    table(&mut out, &top(&categories, 20), active);
    out.push_str("## Time by matched project\n\n");
    table(&mut out, &top(&projects, 15), active);
    if projects.is_empty() {
        out.push_str(
            "_Project time may be absent because activity was not matched confidently enough._\n\n",
        );
    }

    out.push_str("## Daily pattern\n\n| Day | Active | Productive | Neutral | Distracting | Switches | Goals | Outputs |\n|---|---:|---:|---:|---:|---:|---:|---:|\n");
    for d in &daily {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {}/{} | {} |\n",
            d.day,
            duration(d.active),
            duration(d.productive),
            duration((d.active - d.productive - d.distracting).max(0)),
            duration(d.distracting),
            d.switches,
            d.goals_completed,
            d.goals_total,
            d.outputs
        ));
    }
    out.push('\n');

    out.push_str("## Goals and outcomes\n\n");
    let mut goal_count = 0;
    let mut stmt = conn.prepare("SELECT day, title, COALESCE(project, ''), completed, target_minutes, target_count, COALESCE(target_unit, '') FROM goals WHERE day BETWEEN ?1 AND ?2 ORDER BY day, sort_order, id").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![o.start_date, o.end_date], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)? != 0,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, String>(6)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    out.push_str("| Day | Goal | Project | Target | Result |\n|---|---|---|---|---|\n");
    for row in rows {
        let (day, title, project, done, minutes, count, unit) = row.map_err(|e| e.to_string())?;
        goal_count += 1;
        let target = minutes
            .map(|v| format!("{v} min"))
            .or_else(|| count.map(|v| format!("{v} {}", clean(&unit))))
            .unwrap_or_else(|| "No numeric target".into());
        out.push_str(&format!(
            "| {day} | {} | {} | {target} | {} |\n",
            clean(&title),
            if project.is_empty() {
                "—".into()
            } else {
                clean(&project)
            },
            if done { "Completed" } else { "Not completed" }
        ));
    }
    if goal_count == 0 {
        out.push_str("| — | No goals recorded | — | — | — |\n");
    }
    out.push('\n');

    out.push_str("## Check-ins\n\n");
    let mut checkins = BTreeMap::new();
    let mut stmt = conn.prepare("SELECT cv.checkin_id, COALESCE(cd.label, cv.checkin_id), SUM(cv.value) FROM checkin_values cv LEFT JOIN checkin_definitions cd ON cd.id=cv.checkin_id WHERE cv.day BETWEEN ?1 AND ?2 AND cv.value>0 GROUP BY cv.checkin_id,cd.label").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![o.start_date, o.end_date], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (id, label, value) = row.map_err(|e| e.to_string())?;
        checkins.insert(id, (label, value));
    }
    if checkins.is_empty() {
        out.push_str("_No positive check-ins were recorded._\n");
    } else {
        for (_, (label, value)) in checkins {
            out.push_str(&format!("- {}: **{}**\n", clean(&label), value));
        }
    }
    out.push('\n');

    out.push_str("## Detected outputs\n\n");
    let mut output_types = BTreeMap::new();
    let mut stmt = conn.prepare("SELECT event_type,COUNT(*) FROM output_events WHERE day BETWEEN ?1 AND ?2 GROUP BY event_type").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![o.start_date, o.end_date], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (kind, count) = row.map_err(|e| e.to_string())?;
        output_types.insert(kind, count);
    }
    if output_types.is_empty() {
        out.push_str("_No watched-folder outputs were detected. This does not prove that nothing was produced._\n");
    } else {
        for (kind, count) in output_types {
            out.push_str(&format!(
                "- {}: **{}**\n",
                clean(&kind.replace('_', " ")),
                count
            ));
        }
    }
    out.push('\n');

    if o.include_notes {
        out.push_str("## Daily notes (private opt-in)\n\n");
        let mut any = false;
        let mut stmt=conn.prepare("SELECT day,COALESCE(notes,'') FROM daily_checkin WHERE day BETWEEN ?1 AND ?2 AND TRIM(COALESCE(notes,''))!='' ORDER BY day").map_err(|e|e.to_string())?;
        let rows = stmt
            .query_map(params![o.start_date, o.end_date], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (day, note) = row.map_err(|e| e.to_string())?;
            any = true;
            out.push_str(&format!("- **{day}:** {}\n", truncate(&note, 1000)));
        }
        if !any {
            out.push_str("_No daily notes were recorded._\n");
        }
        out.push('\n');
    }
    if o.include_titles {
        out.push_str("## Window/page title samples (private opt-in)\n\n");
        if title_samples.is_empty() {
            out.push_str("_No title samples were available._\n\n");
        } else {
            out.push_str("| Day | Activity | Classification | Title |\n|---|---|---|---|\n");
            for (day, name, title, bucket) in title_samples {
                out.push_str(&format!(
                    "| {day} | {} | {} | {} |\n",
                    clean(&name),
                    clean(&bucket),
                    clean(&title)
                ));
            }
            out.push('\n');
        }
    }
    if o.include_raw_text {
        out.push_str("## Raw captured text samples (high-sensitivity opt-in)\n\n");
        let mut any = false;
        let mut stmt=conn.prepare("SELECT day,domain,raw_text_excerpt FROM browser_activity WHERE day BETWEEN ?1 AND ?2 AND TRIM(COALESCE(raw_text_excerpt,''))!='' ORDER BY duration_seconds DESC,timestamp DESC LIMIT ?3").map_err(|e|e.to_string())?;
        let rows = stmt
            .query_map(
                params![o.start_date, o.end_date, MAX_PRIVATE_SAMPLES as i64],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (day, domain, text) = row.map_err(|e| e.to_string())?;
            any = true;
            let name = alias(
                &domain,
                "browser",
                o.include_activity_names,
                &mut aliases,
                &mut app_index,
                &mut site_index,
            );
            out.push_str(&format!(
                "- **{day} · {}:** {}\n",
                clean(&name),
                truncate(&text, 500)
            ));
        }
        if !any {
            out.push_str("_No retained raw text was available._\n");
        }
        out.push('\n');
    }
    out.push_str("## Questions for the accountability review\n\n1. Which activities are the clearest avoidable time leaks?\n2. Am I doing meaningful output-producing work, or merely staying busy?\n3. Which neutral activity should be reclassified or investigated?\n4. When does my work appear most fragmented?\n5. What three concrete rules should I follow for the next seven days?\n\n---\nGenerated locally by Tempo. Classification is evidence, not certainty; correct bad app/site rules before relying on the conclusions.\n");

    Ok(AccountabilityExport {
        filename: format!("tempo-accountability-{}_to_{}.md", o.start_date, o.end_date),
        markdown: out,
        start_date: o.start_date.clone(),
        end_date: o.end_date.clone(),
        day_count,
        tracked_days,
        active_seconds: active,
    })
}

pub fn save_verified(
    directory: &std::path::Path,
    export: &AccountabilityExport,
) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let mut path = directory.join(&export.filename);
    for suffix in 2..=999 {
        if !path.exists() {
            break;
        }
        let stem = export.filename.trim_end_matches(".md");
        path = directory.join(format!("{stem}-{suffix}.md"));
    }
    if path.exists() {
        return Err("Too many exports already use this filename".into());
    }
    std::fs::write(&path, export.markdown.as_bytes()).map_err(|e| e.to_string())?;
    let saved = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    if saved != export.markdown {
        let _ = std::fs::remove_file(&path);
        return Err("The saved export did not pass verification".into());
    }
    Ok(path)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    fn fixture() -> Connection {
        let conn = db::test_conn();
        conn.execute("INSERT INTO category_rules(app_name,category,ai_review,updated_at) VALUES ('DaVinci Resolve','productive',0,'now'),('Telegram Desktop','distraction',0,'now')",[]).unwrap();
        conn.execute("INSERT INTO activity_log(timestamp,day,app_name,window_title,duration_seconds,is_idle) VALUES ('2026-07-28T09:00:00Z','2026-07-28','DaVinci Resolve','Video first cut',3600,0),('2026-07-28T10:00:00Z','2026-07-28','Telegram Desktop','Private chat',1800,0)",[]).unwrap();
        conn.execute("INSERT INTO goals(day,title,project,target_minutes,priority,completed,sort_order,recurring,created_at) VALUES ('2026-07-28','Export first cut','Content Creation',60,'high',1,0,0,'now')",[]).unwrap();
        conn
    }
    #[test]
    fn report_is_chatgpt_ready_and_private_by_default() {
        let c = fixture();
        let r = generate(
            &c,
            &AccountabilityExportOptions {
                start_date: "2026-07-28".into(),
                end_date: "2026-07-29".into(),
                include_activity_names: true,
                include_titles: false,
                include_raw_text: false,
                include_notes: false,
            },
        )
        .unwrap();
        assert_eq!(r.day_count, 2);
        assert!(r.markdown.contains("Suggested prompt for ChatGPT"));
        assert!(r.markdown.contains("DaVinci Resolve"));
        assert!(r.markdown.contains("Export first cut"));
        assert!(!r.markdown.contains("Video first cut"));
        assert!(!r.markdown.contains("Raw captured text samples"));
    }
    #[test]
    fn saved_markdown_is_readable_and_exact() {
        let conn = fixture();
        let export = generate(
            &conn,
            &AccountabilityExportOptions {
                start_date: "2026-07-28".into(),
                end_date: "2026-07-28".into(),
                include_activity_names: true,
                include_titles: false,
                include_raw_text: false,
                include_notes: false,
            },
        )
        .unwrap();
        let dir = std::env::temp_dir().join(format!("tempo-export-test-{}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).unwrap();
        }
        let path = save_verified(&dir, &export).unwrap();
        assert_eq!(
            path.extension().and_then(|value| value.to_str()),
            Some("md")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), export.markdown);
        assert!(path.metadata().unwrap().len() > 1000);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn sources_can_be_anonymised_and_range_is_validated() {
        let c = fixture();
        let r = generate(
            &c,
            &AccountabilityExportOptions {
                start_date: "2026-07-28".into(),
                end_date: "2026-07-28".into(),
                include_activity_names: false,
                include_titles: false,
                include_raw_text: false,
                include_notes: false,
            },
        )
        .unwrap();
        assert!(r.markdown.contains("Desktop app #1"));
        assert!(!r.markdown.contains("DaVinci Resolve"));
        let e = generate(
            &c,
            &AccountabilityExportOptions {
                start_date: "2026-07-29".into(),
                end_date: "2026-07-28".into(),
                include_activity_names: true,
                include_titles: false,
                include_raw_text: false,
                include_notes: false,
            },
        )
        .unwrap_err();
        assert!(e.contains("End date"));
    }
}
