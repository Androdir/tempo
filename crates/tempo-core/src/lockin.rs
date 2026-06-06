//! Daily "Lock-In Plan": a concrete plan for tomorrow built from today's
//! results. Uses the local Ollama LLM when enabled, otherwise rule-based
//! templates. Input-gathering (DB) lives in `commands.rs`; this module holds the
//! pure plan generation + prompt + parsing, all unit-tested.

use crate::models::LockinPlan;

/// Everything the plan generator reads about the source day.
#[derive(Default, Clone)]
pub struct PlanInputs {
    pub score: i64,
    pub verdict: String,
    pub completed_goals: Vec<String>,
    pub missed_goals: Vec<String>,
    pub top_distraction: Option<(String, i64)>, // (label, minutes)
    pub video_exports: i64,
    pub code_changes: i64,
    pub first_productive_min: Option<i64>,
    pub recurring_goals: Vec<String>,
    pub notes: String,
}

fn clock(min_of_day: i64) -> String {
    format!("{:02}:{:02}", min_of_day / 60, min_of_day % 60)
}

/// Rule-based plan — always works, no network. Mirrors what the LLM is asked for.
pub fn fallback_plan(day: &str, i: &PlanInputs) -> LockinPlan {
    // Main mission: the most important unfinished thing.
    let main_mission = if let Some(g) = i.missed_goals.first() {
        format!("Finish what you ducked: {g}")
    } else if let Some(g) = i.recurring_goals.first() {
        g.clone()
    } else {
        "Ship one concrete output — a video, a shipped feature, or 60 focused min of study.".to_string()
    };

    // Up to 2 secondary missions from remaining missed/recurring goals + a default.
    let mut secondary: Vec<String> = Vec::new();
    for g in i.missed_goals.iter().skip(1) {
        if secondary.len() >= 2 {
            break;
        }
        secondary.push(g.clone());
    }
    for g in &i.recurring_goals {
        if secondary.len() >= 2 {
            break;
        }
        if !secondary.contains(g) && main_mission != *g {
            secondary.push(g.clone());
        }
    }
    if secondary.is_empty() {
        secondary.push("One 50-min deep-work block before noon.".to_string());
    }
    if secondary.len() < 2 {
        secondary.push("Move your body — gym or training.".to_string());
    }

    // First block: nudged earlier if yesterday started late.
    let first_block = match i.first_productive_min {
        Some(m) if m > 11 * 60 => {
            format!("You didn't get going until {} yesterday — open your main project by 09:30, phone in another room.", clock(m))
        }
        _ => "Open your main project first thing — a 50-min focus block before anything else.".to_string(),
    };

    // Distraction rule + avoid-trap from yesterday's biggest leak.
    let (distraction_rule, avoid_trap) = match &i.top_distraction {
        Some((label, min)) if *min >= 10 => (
            format!("No {label} until the main mission is done — it stole {min}m yesterday."),
            format!("The trap is opening {label} 'just to check'. It's never just a check."),
        ),
        _ => (
            "Phone out of reach during every focus block; no feeds before the first win.".to_string(),
            "The trap is a slow, comfortable morning that quietly eats your best hours.".to_string(),
        ),
    };

    let block_target = i
        .top_distraction
        .as_ref()
        .map(|(l, _)| l.clone())
        .unwrap_or_else(|| "instagram.com, youtube.com, tiktok.com".to_string());
    let focus_mode = format!("Start a 50-min Focus session on the main mission; block {block_target}.");

    let roast_line = roast_for(i);

    LockinPlan {
        day: day.to_string(),
        main_mission,
        secondary_missions: secondary,
        first_block,
        distraction_rule,
        focus_mode,
        avoid_trap,
        roast_line,
        source: "fallback".to_string(),
        edited: false,
    }
}

fn roast_for(i: &PlanInputs) -> String {
    if i.video_exports > 0 || !i.completed_goals.is_empty() {
        if i.score >= 70 {
            "Solid day — now do it again before you start believing your own highlight reel.".to_string()
        } else {
            "You shipped something, which beats most people. Tomorrow, ship it before lunch.".to_string()
        }
    } else if let Some((label, min)) = &i.top_distraction {
        format!("{min} minutes on {label} and nothing to show for it. Tomorrow you owe yourself an output.")
    } else if i.score < 30 {
        "Yesterday was a write-off. No speeches — just open the main project and start.".to_string()
    } else {
        "Decent, forgettable day. Tomorrow, make it count for something you can point at.".to_string()
    }
}

pub fn build_prompt(i: &PlanInputs) -> String {
    let missed = if i.missed_goals.is_empty() { "none".to_string() } else { i.missed_goals.join("; ") };
    let done = if i.completed_goals.is_empty() { "none".to_string() } else { i.completed_goals.join("; ") };
    let leak = i
        .top_distraction
        .as_ref()
        .map(|(l, m)| format!("{l} ({m} min)"))
        .unwrap_or_else(|| "none".to_string());
    let recurring = if i.recurring_goals.is_empty() { "none".to_string() } else { i.recurring_goals.join("; ") };
    let notes = if i.notes.trim().is_empty() { "none".to_string() } else { i.notes.trim().to_string() };

    format!(
        "You are a blunt, funny accountability coach. Make a concrete lock-in plan for TOMORROW \
based on today. Today's score: {score}/100 ({verdict}). Completed goals: {done}. Missed goals: \
{missed}. Biggest distraction: {leak}. Outputs detected: {videos} video export(s), {code} code \
change(s). Recurring goals: {recurring}. User notes: {notes}.\n\n\
Reply ONLY with strict JSON, no prose, in exactly this shape:\n\
{{\"main_mission\": \"one sentence\", \"secondary_missions\": [\"...\", \"...\"], \
\"first_block\": \"what to do first and when\", \"distraction_rule\": \"one concrete rule\", \
\"focus_mode_suggestion\": \"a focus session setup\", \"avoid_trap\": \"one trap to avoid\", \
\"roast_line\": \"one short, funny, blunt line\"}}\n\
Max 2 secondary missions. Keep every field under 25 words. Be specific, casual, and a little savage.",
        score = i.score,
        verdict = i.verdict,
        videos = i.video_exports,
        code = i.code_changes,
    )
}

pub fn parse_plan(day: &str, json: &str) -> Result<LockinPlan, String> {
    // Tolerate models that wrap JSON in prose/code fences.
    let start = json.find('{').ok_or("no JSON object in response")?;
    let end = json.rfind('}').ok_or("no JSON object in response")?;
    let slice = &json[start..=end];
    let v: serde_json::Value = serde_json::from_str(slice).map_err(|e| format!("invalid JSON: {e}"))?;

    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
    let main_mission = s("main_mission");
    if main_mission.is_empty() {
        return Err("missing main_mission".to_string());
    }
    let secondary: Vec<String> = v
        .get("secondary_missions")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .take(2)
                .collect()
        })
        .unwrap_or_default();

    Ok(LockinPlan {
        day: day.to_string(),
        main_mission,
        secondary_missions: secondary,
        first_block: s("first_block"),
        distraction_rule: s("distraction_rule"),
        focus_mode: s("focus_mode_suggestion"),
        avoid_trap: s("avoid_trap"),
        roast_line: s("roast_line"),
        source: "llm".to_string(),
        edited: false,
    }
    .non_empty_or_default())
}

impl LockinPlan {
    /// Fill any blank fields the model omitted with sane placeholders.
    fn non_empty_or_default(mut self) -> Self {
        if self.secondary_missions.is_empty() {
            self.secondary_missions.push("One focused deep-work block before noon.".to_string());
        }
        if self.first_block.is_empty() {
            self.first_block = "Open your main project first — 50 focused minutes.".to_string();
        }
        if self.distraction_rule.is_empty() {
            self.distraction_rule = "No social feeds before the first win.".to_string();
        }
        if self.focus_mode.is_empty() {
            self.focus_mode = "A 50-min Focus session on the main mission.".to_string();
        }
        if self.avoid_trap.is_empty() {
            self.avoid_trap = "Don't let a slow morning eat your best hours.".to_string();
        }
        if self.roast_line.is_empty() {
            self.roast_line = "Less planning, more shipping. Go.".to_string();
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_is_complete_without_llm() {
        let inputs = PlanInputs {
            score: 40,
            verdict: "bad".into(),
            missed_goals: vec!["Post 1 video".into()],
            top_distraction: Some(("instagram.com".into(), 42)),
            ..Default::default()
        };
        let p = fallback_plan("2026-04-01", &inputs);
        assert_eq!(p.source, "fallback");
        assert!(p.main_mission.contains("Post 1 video"));
        assert!(!p.secondary_missions.is_empty());
        assert!(p.distraction_rule.contains("instagram.com"));
        assert!(!p.focus_mode.is_empty());
        assert!(!p.roast_line.is_empty());
    }

    #[test]
    fn fallback_caps_two_secondary_missions() {
        let inputs = PlanInputs {
            missed_goals: vec!["A".into(), "B".into(), "C".into(), "D".into()],
            ..Default::default()
        };
        let p = fallback_plan("d", &inputs);
        assert!(p.secondary_missions.len() <= 2);
    }

    #[test]
    fn parse_plan_reads_json() {
        let json = r#"prose... {"main_mission":"Ship the editor","secondary_missions":["gym","study 1h","extra"],"first_block":"open VS Code 9am","distraction_rule":"no IG","focus_mode_suggestion":"50m focus","avoid_trap":"slow morning","roast_line":"go"} trailing"#;
        let p = parse_plan("2026-04-02", json).unwrap();
        assert_eq!(p.main_mission, "Ship the editor");
        assert_eq!(p.secondary_missions.len(), 2); // capped at 2
        assert_eq!(p.source, "llm");
    }

    #[test]
    fn parse_plan_rejects_missing_main() {
        assert!(parse_plan("d", r#"{"secondary_missions":[]}"#).is_err());
    }
}
