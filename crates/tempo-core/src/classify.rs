//! Rule-based activity classification (no AI/LLM).
//!
//! Combines domain rule + page title + content_type + content_summary +
//! detected keywords into one of the six categories, with a short reason.

pub struct Classification {
    pub category: String,
    pub reason: String,
    /// How confident the rule layer is, 0.0–1.0.
    pub confidence: f64,
}

/// Keyword signal sets per category. Lowercase, matched as substrings against
/// the combined "title + summary + keywords" haystack.
const STUDY: &[&str] = &[
    "algorithm",
    "assignment problem",
    "minimum cost",
    "matching",
    "theorem",
    "proof",
    "lecture",
    "homework",
    "calculus",
    "linear algebra",
    "dataset",
    "study",
    "exam",
    "research paper",
    "complexity",
    "hungarian algorithm",
    "equation",
    "derivative",
    "tutorial",
    "documentation",
    "flashcard",
];
const BUSINESS: &[&str] = &[
    "retention",
    "hook",
    "viral",
    "audience",
    "marketing",
    "conversion",
    "client",
    "invoice",
    "revenue",
    "analytics",
    "short-form editing",
    "content strategy",
    "engagement",
    "monetization",
    "campaign",
    "pricing",
    "upload",
    "new post",
    "caption",
    "schedule post",
    "composer",
    "create reel",
    "thumbnail",
    "cta",
];
const DISTRACTION: &[&str] = &[
    "reels",
    "explore",
    "for you",
    "feed",
    "meme",
    "tiktok",
    "shorts feed",
    "trending",
    "infinite scroll",
    "celebrity",
    "gossip",
];
const RECOVERY: &[&str] = &[
    "meditation",
    "relax",
    "lofi",
    "lo-fi",
    "calm",
    "sleep",
    "break",
];

fn count_hits(haystack: &str, needles: &[&str]) -> usize {
    needles.iter().filter(|n| haystack.contains(**n)).count()
}

/// `domain_category` is the manually-assigned domain rule category (if any).
pub fn classify(
    domain: &str,
    title: &str,
    content_type: Option<&str>,
    summary: Option<&str>,
    keywords: &[String],
    domain_category: Option<&str>,
) -> Classification {
    let mut hay = String::new();
    hay.push_str(&title.to_ascii_lowercase());
    hay.push(' ');
    if let Some(s) = summary {
        hay.push_str(&s.to_ascii_lowercase());
        hay.push(' ');
    }
    for k in keywords {
        hay.push_str(&k.to_ascii_lowercase());
        hay.push(' ');
    }

    let ct = content_type.unwrap_or("other");

    // Base category: domain rule, else a sensible default from content_type.
    let base = domain_category
        .map(|c| c.to_string())
        .unwrap_or_else(|| default_for_type(ct, domain));

    // Score content signals.
    let study = count_hits(&hay, STUDY);
    let business = count_hits(&hay, BUSINESS);
    let distraction = count_hits(&hay, DISTRACTION) + if ct == "social_feed" { 1 } else { 0 };
    let recovery = count_hits(&hay, RECOVERY);

    let (best, score) = [
        ("study", study),
        ("business", business),
        ("distraction", distraction),
        ("recovery", recovery),
    ]
    .into_iter()
    .max_by_key(|(_, s)| *s)
    .unwrap();

    // Strong content signal overrides the domain default.
    if score >= 2 {
        return Classification {
            category: best.to_string(),
            reason: format!("content signals → {best} ({score} keyword matches)"),
            confidence: 0.8,
        };
    }
    if score == 1 && (domain_category.is_none() || base == "neutral") {
        return Classification {
            category: best.to_string(),
            reason: format!("content hint → {best}"),
            confidence: 0.55,
        };
    }

    Classification {
        category: base.clone(),
        reason: if domain_category.is_some() {
            format!("domain rule → {base}")
        } else {
            format!("default for {ct} → {base}")
        },
        // A user-set domain rule is trusted; a content-type default is a guess.
        confidence: if domain_category.is_some() {
            0.85
        } else {
            0.45
        },
    }
}

/// If the text strongly signals a specific category (>= 2 keyword hits), return
/// it. Used to detect when OCR/content text conflicts with the default category.
pub fn keyword_category(text: &str) -> Option<String> {
    let hay = text.to_ascii_lowercase();
    let scores = [
        ("study", count_hits(&hay, STUDY)),
        ("business", count_hits(&hay, BUSINESS)),
        ("distraction", count_hits(&hay, DISTRACTION)),
        ("recovery", count_hits(&hay, RECOVERY)),
    ];
    let (best, score) = scores.into_iter().max_by_key(|(_, s)| *s).unwrap();
    if score >= 2 {
        Some(best.to_string())
    } else {
        None
    }
}

fn default_for_type(content_type: &str, domain: &str) -> String {
    let d = domain.to_ascii_lowercase();
    match content_type {
        "social_feed" => "distraction",
        "video" => "neutral",
        "chat" => "neutral",
        "docs_editor" => "productive",
        "article" => "neutral",
        "search_results" => "neutral",
        _ => {
            if d.contains("github") {
                "productive"
            } else {
                "neutral"
            }
        }
    }
    .to_string()
}
