//! Daily productivity score (0–100) from a configurable rule set.
//!
//! Time-based rules read minutes per *final* category; event rules read the
//! day's self-reported check-ins. Weights and thresholds are user-editable and
//! stored locally; everything here is pure/local.

use std::collections::HashMap;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::models::{CategoryMinutes, ScoreLine, ScoreReport};
use crate::settings;

const CONFIG_KEY: &str = "scoring_config";

pub struct Stats {
    pub cat_seconds: HashMap<String, i64>,
    pub instagram_seconds: i64,
    pub youtube_seconds: i64,
    pub first_productive_min: Option<i64>, // minutes since local midnight
}

pub struct Checkins {
    pub main_goal_completed: bool,
    pub videos_posted: i64,
    pub gym_logged: bool,
}

/// Detected proof-of-output counts for the day (folder watcher).
#[derive(Default)]
pub struct OutputSignals {
    pub video_exports: i64,
    pub code_changes: i64,
    pub study_materials: i64,
}

struct RuleDef {
    id: &'static str,
    label: &'static str,
    weight: i64,
    threshold: Option<i64>,
    positive: bool,
}

// The default scoring rules (exactly as specified).
const RULES: &[RuleDef] = &[
    RuleDef { id: "main_goal", label: "Completed main daily goal", weight: 30, threshold: None, positive: true },
    RuleDef { id: "posted_video", label: "Posted 1+ videos", weight: 25, threshold: None, positive: true },
    RuleDef { id: "business_min", label: "90+ min editing / business work", weight: 20, threshold: Some(90), positive: true },
    RuleDef { id: "study_min", label: "60+ min studying", weight: 15, threshold: Some(60), positive: true },
    RuleDef { id: "coding_min", label: "60+ min coding / building", weight: 15, threshold: Some(60), positive: true },
    RuleDef { id: "gym", label: "Gym / wrestling logged", weight: 10, threshold: None, positive: true },
    RuleDef { id: "instagram", label: "Instagram distraction over 30 min", weight: -15, threshold: Some(30), positive: false },
    RuleDef { id: "youtube", label: "YouTube distraction over 45 min", weight: -10, threshold: Some(45), positive: false },
    RuleDef { id: "recovery", label: "Music / pacing / recovery over 60 min", weight: -15, threshold: Some(60), positive: false },
    RuleDef { id: "no_main_goal", label: "No main goal completed", weight: -25, threshold: None, positive: false },
    RuleDef { id: "late_start", label: "First productive block after 14:00", weight: -10, threshold: Some(14), positive: false },
    // Proof-of-output signals (folder watcher). Only fire when outputs are detected,
    // so days without the watcher configured score exactly as before.
    RuleDef { id: "shipped_video", label: "Exported a video (proof of output)", weight: 15, threshold: None, positive: true },
    RuleDef { id: "shipped_code", label: "Shipped code changes", weight: 10, threshold: None, positive: true },
    RuleDef { id: "study_output", label: "Created/opened study material", weight: 5, threshold: None, positive: true },
];

#[derive(Serialize, Deserialize, Default)]
struct ConfigOverrides {
    #[serde(default)]
    weights: HashMap<String, i64>,
    #[serde(default)]
    thresholds: HashMap<String, i64>,
}

fn load_overrides(conn: &Connection) -> ConfigOverrides {
    settings::get_setting(conn, CONFIG_KEY)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_overrides(conn: &Connection, o: &ConfigOverrides) -> rusqlite::Result<()> {
    let s = serde_json::to_string(o).unwrap_or_else(|_| "{}".to_string());
    settings::set_setting(conn, CONFIG_KEY, &s)
}

fn eff_weight(o: &ConfigOverrides, def: &RuleDef) -> i64 {
    *o.weights.get(def.id).unwrap_or(&def.weight)
}

fn eff_threshold(o: &ConfigOverrides, def: &RuleDef) -> Option<i64> {
    def.threshold.map(|d| *o.thresholds.get(def.id).unwrap_or(&d))
}

pub fn set_weight(conn: &Connection, id: &str, weight: i64) -> Result<(), String> {
    if !RULES.iter().any(|r| r.id == id) {
        return Err(format!("unknown rule: {id}"));
    }
    let mut o = load_overrides(conn);
    o.weights.insert(id.to_string(), weight.clamp(-100, 100));
    save_overrides(conn, &o).map_err(|e| e.to_string())
}

pub fn set_threshold(conn: &Connection, id: &str, threshold: i64) -> Result<(), String> {
    let def = RULES.iter().find(|r| r.id == id).ok_or_else(|| format!("unknown rule: {id}"))?;
    if def.threshold.is_none() {
        return Err(format!("rule {id} has no threshold"));
    }
    let mut o = load_overrides(conn);
    o.thresholds.insert(id.to_string(), threshold.clamp(0, 1440));
    save_overrides(conn, &o).map_err(|e| e.to_string())
}

pub fn reset(conn: &Connection) -> Result<(), String> {
    save_overrides(conn, &ConfigOverrides::default()).map_err(|e| e.to_string())
}

pub fn verdict(score: i64) -> &'static str {
    match score {
        85..=100 => "excellent",
        70..=84 => "good",
        50..=69 => "mid",
        30..=49 => "bad",
        _ => "cooked",
    }
}

fn clock(min_of_day: i64) -> String {
    format!("{:02}:{:02}", min_of_day / 60, min_of_day % 60)
}

pub fn build_report(
    conn: &Connection,
    day: String,
    stats: &Stats,
    checkins: &Checkins,
    outputs: &OutputSignals,
    main_goal_name: Option<String>,
) -> ScoreReport {
    let o = load_overrides(conn);
    let mins = |cat: &str| stats.cat_seconds.get(cat).copied().unwrap_or(0) / 60;
    let business = mins("business");
    let study = mins("study");
    let coding = mins("productive");
    let recovery = mins("recovery");
    let insta = stats.instagram_seconds / 60;
    let yt = stats.youtube_seconds / 60;
    let has_main_goal = main_goal_name.is_some();

    let mut raw = 0i64;
    let mut lines: Vec<ScoreLine> = Vec::new();

    for def in RULES {
        let weight = eff_weight(&o, def);
        let threshold = eff_threshold(&o, def);
        let (triggered, value): (bool, String) = match def.id {
            "main_goal" => (
                has_main_goal && checkins.main_goal_completed,
                if !has_main_goal {
                    "no goal".into()
                } else if checkins.main_goal_completed {
                    "done".into()
                } else {
                    "not done".into()
                },
            ),
            "posted_video" => (checkins.videos_posted >= 1, format!("{} posted", checkins.videos_posted)),
            "business_min" => (business >= threshold.unwrap_or(90), format!("{business}m")),
            "study_min" => (study >= threshold.unwrap_or(60), format!("{study}m")),
            "coding_min" => (coding >= threshold.unwrap_or(60), format!("{coding}m")),
            "gym" => (
                checkins.gym_logged,
                if checkins.gym_logged { "logged".into() } else { "not logged".into() },
            ),
            "instagram" => (insta > threshold.unwrap_or(30), format!("{insta}m")),
            "youtube" => (yt > threshold.unwrap_or(45), format!("{yt}m")),
            "recovery" => (recovery > threshold.unwrap_or(60), format!("{recovery}m")),
            "no_main_goal" => (
                has_main_goal && !checkins.main_goal_completed,
                if !has_main_goal {
                    "no goal".into()
                } else if checkins.main_goal_completed {
                    "completed".into()
                } else {
                    "not completed".into()
                },
            ),
            "late_start" => {
                let cutoff = threshold.unwrap_or(14) * 60;
                match stats.first_productive_min {
                    Some(m) => (m > cutoff, format!("first at {}", clock(m))),
                    None => (false, "no productive block".into()),
                }
            }
            "shipped_video" => (outputs.video_exports > 0, format!("{} export(s)", outputs.video_exports)),
            "shipped_code" => (outputs.code_changes > 0, format!("{} change(s)", outputs.code_changes)),
            "study_output" => (outputs.study_materials > 0, format!("{} file(s)", outputs.study_materials)),
            _ => (false, String::new()),
        };
        if triggered {
            raw += weight;
        }
        lines.push(ScoreLine {
            id: def.id.into(),
            label: def.label.into(),
            weight,
            threshold,
            has_threshold: def.threshold.is_some(),
            positive: def.positive,
            triggered,
            value,
        });
    }

    let score = raw.clamp(0, 100);

    let mut top_wins: Vec<ScoreLine> =
        lines.iter().filter(|l| l.positive && l.triggered).cloned().collect();
    top_wins.sort_by(|a, b| b.weight.cmp(&a.weight));
    top_wins.truncate(3);

    let mut biggest_leaks: Vec<ScoreLine> =
        lines.iter().filter(|l| !l.positive && l.triggered).cloned().collect();
    biggest_leaks.sort_by(|a, b| a.weight.cmp(&b.weight)); // most negative first
    biggest_leaks.truncate(3);

    let suggestion = build_suggestion(&o, &lines, checkins, &main_goal_name, business, study, coding);

    let mut category_minutes: Vec<CategoryMinutes> = stats
        .cat_seconds
        .iter()
        .map(|(c, s)| CategoryMinutes { category: c.clone(), minutes: s / 60 })
        .filter(|c| c.minutes > 0)
        .collect();
    category_minutes.sort_by(|a, b| b.minutes.cmp(&a.minutes));

    ScoreReport {
        date: day,
        score,
        verdict: verdict(score).to_string(),
        top_wins,
        biggest_leaks,
        suggestion,
        lines,
        category_minutes,
        main_goal_completed: checkins.main_goal_completed,
        videos_posted: checkins.videos_posted,
        gym_logged: checkins.gym_logged,
        main_goal_name,
    }
}

fn build_suggestion(
    o: &ConfigOverrides,
    lines: &[ScoreLine],
    checkins: &Checkins,
    main_goal_name: &Option<String>,
    business: i64,
    study: i64,
    coding: i64,
) -> String {
    let w = |id: &str| RULES.iter().find(|r| r.id == id).map(|d| eff_weight(o, d)).unwrap_or(0);
    let mut cands: Vec<(i64, String)> = Vec::new();

    if !checkins.main_goal_completed {
        let gain = w("main_goal") + w("no_main_goal").abs();
        let suffix = main_goal_name
            .as_ref()
            .map(|n| format!(" ({n})"))
            .unwrap_or_default();
        cands.push((gain, format!("Finish your main goal{suffix} before 2pm — worth {gain} points and removes the penalty.")));
    }

    for l in lines.iter().filter(|l| !l.positive && l.triggered && l.has_threshold) {
        let label = match l.id.as_str() {
            "instagram" => "Instagram",
            "youtube" => "YouTube",
            "recovery" => "recovery/music time",
            _ => l.label.as_str(),
        };
        cands.push((
            l.weight.abs(),
            format!("Cut {label} below {}m to save {} points.", l.threshold.unwrap_or(0), l.weight.abs()),
        ));
    }

    for l in lines.iter().filter(|l| l.positive && !l.triggered && l.has_threshold) {
        let (have, label) = match l.id.as_str() {
            "business_min" => (business, "business/editing work"),
            "study_min" => (study, "studying"),
            "coding_min" => (coding, "coding"),
            _ => (0, l.label.as_str()),
        };
        let need = l.threshold.unwrap_or(0) - have;
        if have > 0 && need > 0 {
            cands.push((l.weight, format!("{need} more minutes of {label} earns {} points.", l.weight)));
        }
    }

    cands.sort_by(|a, b| b.0.cmp(&a.0));
    cands
        .first()
        .map(|(_, t)| t.clone())
        .unwrap_or_else(|| "Strong, balanced day — keep the momentum tomorrow.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_bands() {
        assert_eq!(verdict(100), "excellent");
        assert_eq!(verdict(85), "excellent");
        assert_eq!(verdict(84), "good");
        assert_eq!(verdict(70), "good");
        assert_eq!(verdict(50), "mid");
        assert_eq!(verdict(49), "bad");
        assert_eq!(verdict(30), "bad");
        assert_eq!(verdict(29), "cooked");
        assert_eq!(verdict(0), "cooked");
    }
}
