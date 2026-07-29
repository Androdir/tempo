//! Step 1 (rule-based classification with a confidence) + Step 2 (decide when
//! the local LLM should be consulted) of the hybrid classifier.

use std::collections::{HashMap, HashSet};

use crate::classify;
use crate::projects::{self, Project};

struct BuiltInAppVerdict {
    category: &'static str,
    confidence: f64,
    reason: &'static str,
    block_project_matching: bool,
}

fn normalized_app(label: &str) -> String {
    label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Conservative defaults for contexts where an LLM should not infer intent
/// from the user's project list. Explicit user app rules still take priority.
fn built_in_app_verdict(label: &str, title: &str) -> Option<BuiltInAppVerdict> {
    let app = normalized_app(label);
    let title = normalized_app(title);
    if title.starts_with("tempooptions") {
        return Some(BuiltInAppVerdict {
            category: "neutral",
            confidence: 0.98,
            reason: "Tempo settings → neutral",
            block_project_matching: true,
        });
    }
    match app.as_str() {
        // Tempo is tracking/admin time, never evidence that another project was worked on.
        "tempo" | "tempoexe" | "productivitytracker" | "productivitytrackerexe" => {
            Some(BuiltInAppVerdict {
                category: "neutral",
                confidence: 0.98,
                reason: "Tempo app → neutral",
                block_project_matching: true,
            })
        }
        // Transient Windows surfaces are neither productive work nor distractions.
        "windowsshellexperiencehost" | "startmenuexperiencehost" | "searchhost"
        | "textinputhost" | "lockapp" => Some(BuiltInAppVerdict {
            category: "neutral",
            confidence: 0.95,
            reason: "Windows system surface → neutral",
            block_project_matching: true,
        }),
        // Messaging is distracting by default. A strong configured project match or
        // an explicit user app rule can still classify a work conversation differently.
        "telegram" | "telegramexe" | "telegramdesktop" => Some(BuiltInAppVerdict {
            category: "distraction",
            confidence: 0.82,
            reason: "Telegram default → distraction",
            block_project_matching: false,
        }),
        _ => None,
    }
}

/// Below this rule confidence, the block is a candidate for LLM review.
const CONFIDENCE_THRESHOLD: f64 = 0.75;

/// Pre-loaded rule data (loaded once per read by the caller).
pub struct RuleInputs<'a> {
    pub app_rules: &'a HashMap<String, String>,          // app -> category
    pub app_ai: &'a HashSet<String>,                     // apps flagged "AI review"
    pub domain_cat: &'a HashMap<String, Option<String>>, // domain -> category
    pub domain_ai: &'a HashSet<String>,                  // domains flagged "AI review"
    pub projects: &'a [Project],
}

pub struct RuleVerdict {
    pub category: String,
    pub confidence: f64,
    pub reason: String,
    pub project: Option<String>,
    pub project_confidence: u8,
    pub project_signals: Vec<String>,
    /// Step 2: should the LLM be asked to review this block?
    pub needs_llm: bool,
}

pub fn classify_block(
    inputs: &RuleInputs,
    source: &str,
    label: &str,
    title: &str,
    domain: Option<&str>,
    content_type: Option<&str>,
    summary: Option<&str>,
    keywords: &[String],
) -> RuleVerdict {
    // --- base: app rule (apps/screen) or content rules (web) ---
    let (mut category, mut confidence, mut reason) = if source == "web" {
        let dcat = domain.and_then(|d| inputs.domain_cat.get(d)).cloned().flatten();
        let base = classify::classify(
            domain.unwrap_or(label),
            title,
            content_type,
            summary,
            keywords,
            dcat.as_deref(),
        );
        (base.category, base.confidence, base.reason)
    } else {
        match inputs.app_rules.get(label) {
            Some(c) => (c.clone(), 0.9, format!("app rule → {c}")),
            None => match built_in_app_verdict(label, title) {
                Some(v) => (v.category.to_string(), v.confidence, v.reason.to_string()),
                None => ("uncategorized".to_string(), 0.3, "no app rule".to_string()),
            },
        }
    };
    let block_project_matching = source != "web"
        && built_in_app_verdict(label, title).is_some_and(|v| v.block_project_matching);

    // --- project rules (app/domain + keyword) may override ---
    let extra = build_extra(summary, keywords);
    let mut project = None;
    let mut project_confidence = 0u8;
    let mut project_signals = Vec::new();
    if !block_project_matching {
        if let Some(m) = projects::match_project(inputs.projects, label, title, &extra) {
            project = Some(m.project_name.clone());
            project_confidence = m.confidence;
            project_signals = m.signals.clone();
            if m.confidence >= projects::OVERRIDE_THRESHOLD {
                category = m.category.clone();
                confidence = (m.confidence as f64 / 100.0).max(0.6);
                reason = format!("project: {} ({}%)", m.project_name, m.confidence);
            }
        }
    }

    // --- Step 2 gating signals ---
    let ai_review = match source {
        "web" => domain.map(|d| inputs.domain_ai.contains(d)).unwrap_or(false),
        _ => inputs.app_ai.contains(label),
    };

    let ocr_conflict = match summary {
        Some(s) if !s.trim().is_empty() => {
            let combined = format!("{title} {s} {}", keywords.join(" "));
            classify::keyword_category(&combined).is_some_and(|kc| kc != category)
        }
        _ => false,
    };

    let ambiguous = category == "uncategorized";

    let needs_llm = confidence < CONFIDENCE_THRESHOLD || ambiguous || ai_review || ocr_conflict;

    RuleVerdict {
        category,
        confidence,
        reason,
        project,
        project_confidence,
        project_signals,
        needs_llm,
    }
}

fn build_extra(summary: Option<&str>, keywords: &[String]) -> String {
    let mut s = String::new();
    if let Some(x) = summary {
        s.push_str(x);
        s.push(' ');
    }
    s.push_str(&keywords.join(" "));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> (HashMap<String, String>, HashSet<String>, HashMap<String, Option<String>>, HashSet<String>, Vec<Project>) {
        (HashMap::new(), HashSet::new(), HashMap::new(), HashSet::new(), Vec::new())
    }

    #[test]
    fn confident_app_rule_skips_llm() {
        let (mut ar, aa, dc, da, pj) = empty();
        ar.insert("Visual Studio Code".into(), "productive".into());
        let inp = RuleInputs { app_rules: &ar, app_ai: &aa, domain_cat: &dc, domain_ai: &da, projects: &pj };
        let v = classify_block(&inp, "app", "Visual Studio Code", "main.rs", None, None, None, &[]);
        assert_eq!(v.category, "productive");
        assert!(v.confidence >= 0.75);
        assert!(!v.needs_llm);
    }

    #[test]
    fn tempo_is_neutral_and_cannot_inherit_a_project() {
        let (ar, aa, dc, da, mut pj) = empty();
        pj.push(Project {
            id: 1,
            name: "Bible Reading".into(),
            category: "study".into(),
            keywords: vec!["Tempo".into()],
            apps: vec!["Tempo".into()],
            domains: vec![],
            excluded_apps: vec![],
            excluded_domains: vec![],
            excluded_keywords: vec![],
            priority: 100,
        });
        let inp = RuleInputs { app_rules: &ar, app_ai: &aa, domain_cat: &dc, domain_ai: &da, projects: &pj };
        let v = classify_block(&inp, "app", "Tempo", "Tempo", None, None, None, &[]);
        assert_eq!(v.category, "neutral");
        assert!(v.project.is_none());
        assert!(!v.needs_llm);
    }

    #[test]
    fn tempo_extension_options_are_neutral() {
        let (ar, aa, dc, da, pj) = empty();
        let inp = RuleInputs { app_rules: &ar, app_ai: &aa, domain_cat: &dc, domain_ai: &da, projects: &pj };
        let v = classify_block(
            &inp,
            "app",
            "Firefox",
            "Tempo — Options — Mozilla Firefox",
            None,
            None,
            None,
            &[],
        );
        assert_eq!(v.category, "neutral");
        assert!(v.project.is_none());
        assert!(!v.needs_llm);
    }

    #[test]
    fn telegram_is_consistently_distracting_without_work_evidence() {
        let (ar, aa, dc, da, pj) = empty();
        let inp = RuleInputs { app_rules: &ar, app_ai: &aa, domain_cat: &dc, domain_ai: &da, projects: &pj };
        let v = classify_block(
            &inp,
            "app",
            "Telegram Desktop",
            "Anonymous Chat @chatbot — (139)",
            None,
            None,
            None,
            &[],
        );
        assert_eq!(v.category, "distraction");
        assert!(v.confidence >= CONFIDENCE_THRESHOLD);
        assert!(!v.needs_llm);
    }

    #[test]
    fn explicit_rule_can_mark_telegram_as_work() {
        let (mut ar, aa, dc, da, pj) = empty();
        ar.insert("Telegram Desktop".into(), "business".into());
        let inp = RuleInputs { app_rules: &ar, app_ai: &aa, domain_cat: &dc, domain_ai: &da, projects: &pj };
        let v = classify_block(&inp, "app", "Telegram Desktop", "Client chat", None, None, None, &[]);
        assert_eq!(v.category, "business");
        assert!(!v.needs_llm);
    }

    #[test]
    fn windows_notification_surface_is_neutral() {
        let (ar, aa, dc, da, pj) = empty();
        let inp = RuleInputs { app_rules: &ar, app_ai: &aa, domain_cat: &dc, domain_ai: &da, projects: &pj };
        let v = classify_block(
            &inp,
            "app",
            "Windows Shell Experience Host",
            "New notification",
            None,
            None,
            None,
            &[],
        );
        assert_eq!(v.category, "neutral");
        assert!(!v.needs_llm);
    }

    #[test]
    fn uncategorized_app_needs_llm() {
        let (ar, aa, dc, da, pj) = empty();
        let inp = RuleInputs { app_rules: &ar, app_ai: &aa, domain_cat: &dc, domain_ai: &da, projects: &pj };
        let v = classify_block(&inp, "app", "SomeUnknownApp", "a window", None, None, None, &[]);
        assert_eq!(v.category, "uncategorized");
        assert!(v.needs_llm); // low-confidence / ambiguous
    }

    #[test]
    fn ai_review_forces_llm_even_when_confident() {
        let (mut ar, mut aa, dc, da, pj) = empty();
        ar.insert("Slack".into(), "business".into());
        aa.insert("Slack".into());
        let inp = RuleInputs { app_rules: &ar, app_ai: &aa, domain_cat: &dc, domain_ai: &da, projects: &pj };
        let v = classify_block(&inp, "app", "Slack", "#team", None, None, None, &[]);
        // Confident rule, but the AI-review flag still forces LLM review.
        assert!(v.confidence >= 0.75 && v.needs_llm);
    }

    #[test]
    fn ocr_conflict_forces_llm() {
        let (mut ar, aa, dc, da, pj) = empty();
        ar.insert("Chrome".into(), "productive".into());
        let inp = RuleInputs { app_rules: &ar, app_ai: &aa, domain_cat: &dc, domain_ai: &da, projects: &pj };
        let v = classify_block(&inp, "screen", "Chrome", "window", None, None, Some("reels explore feed trending"), &[]);
        // App rule says productive, but OCR text screams distraction -> review.
        assert_eq!(v.category, "productive");
        assert!(v.needs_llm);
    }
}
