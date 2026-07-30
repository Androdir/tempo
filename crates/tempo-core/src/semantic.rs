use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::settings;

const SETTING_KEY: &str = "classification_policies";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationPolicy {
    pub id: String,
    pub name: String,
    pub category: String,
    #[serde(default)]
    pub kinds: Vec<String>,
    pub terms: Vec<String>,
    pub enabled: bool,
    pub built_in: bool,
    pub priority: i64,
}

pub fn defaults() -> Vec<ClassificationPolicy> {
    vec![
        ClassificationPolicy {
            id: "recognized-gameplay".into(),
            name: "Recognized gameplay".into(),
            category: "distraction".into(),
            kinds: vec!["game".into()],
            terms: [
                "gameplay",
                "video game",
                "playing a game",
                "steam",
                "epic games",
                "battle.net",
                "riot client",
                "minecraft",
                "roblox",
                "fortnite",
                "counter-strike",
                "grand theft auto",
                "call of duty",
                "overwatch",
                "rocket league",
                "world of warcraft",
                "elden ring",
                "cyberpunk",
                "baldur's gate",
                "league of legends",
                "valorant",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            enabled: true,
            built_in: true,
            priority: 100,
        },
        ClassificationPolicy {
            id: "generic-telegram".into(),
            name: "Generic Telegram use".into(),
            category: "distraction".into(),
            kinds: vec!["chat".into()],
            terms: vec!["telegram".into()],
            enabled: true,
            built_in: true,
            priority: 80,
        },
    ]
}

pub fn load(conn: &Connection) -> Vec<ClassificationPolicy> {
    settings::get_setting(conn, SETTING_KEY)
        .and_then(|raw| serde_json::from_str::<Vec<ClassificationPolicy>>(&raw).ok())
        .unwrap_or_else(defaults)
}

pub fn save(conn: &Connection, policies: &[ClassificationPolicy]) -> Result<(), String> {
    let raw = serde_json::to_string(policies).map_err(|e| e.to_string())?;
    settings::set_setting(conn, SETTING_KEY, &raw).map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM llm_classification", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn matched<'a>(
    policies: &'a [ClassificationPolicy],
    evidence: &str,
    activity_kind: &str,
) -> Option<(&'a ClassificationPolicy, String)> {
    let haystack = evidence.to_ascii_lowercase();
    policies
        .iter()
        .filter(|p| p.enabled)
        .filter_map(|policy| {
            let kind_match = policy.kinds.iter().any(|kind| kind == activity_kind);
            let term_match = policy
                .terms
                .iter()
                .filter(|term| !term.trim().is_empty())
                .filter(|term| haystack.contains(&term.trim().to_ascii_lowercase()))
                .max_by_key(|term| term.len());
            if kind_match {
                Some((policy, activity_kind.to_string()))
            } else {
                term_match.map(|term| (policy, term.to_string()))
            }
        })
        .max_by_key(|(policy, term)| (policy.priority, term.len()))
}

/// Identify the observable kind before applying the user's productivity policy.
pub fn detect_activity_kind(
    source: &str,
    label: &str,
    title: &str,
    executable_path: Option<&str>,
    content_type: Option<&str>,
    summary: Option<&str>,
    keywords: &[String],
) -> String {
    let evidence = format!(
        "{label} {title} {} {} {}",
        executable_path.unwrap_or_default(),
        summary.unwrap_or_default(),
        keywords.join(" ")
    )
    .to_ascii_lowercase();
    let normalized = evidence
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>();
    let contains_any = |terms: &[&str]| terms.iter().any(|term| evidence.contains(term));
    let token_any = |terms: &[&str]| {
        normalized
            .split_whitespace()
            .any(|token| terms.contains(&token))
    };

    if executable_path.is_some_and(|path| {
        let path = path.to_ascii_lowercase();
        [
            r"\steamapps\common\",
            r"\epic games\",
            r"\riot games\",
            r"\ubisoft game launcher\games\",
            r"\gog galaxy\games\",
            r"\xboxgames\",
            r"\battle.net\",
        ]
        .iter()
        .any(|segment| path.contains(segment))
    }) || contains_any(&[
        "gameplay",
        "playing a game",
        "video game",
        "riot client",
        "battle.net",
        "epic games launcher",
        "steamapps",
        "minecraft",
        "roblox",
        "fortnite",
        "counter-strike",
        "counter strike",
        "grand theft auto",
        "call of duty",
        "overwatch",
        "rocket league",
        "world of warcraft",
        "elden ring",
        "cyberpunk",
        "baldur's gate",
        "league of legends",
        "valorant",
    ]) || token_any(&[
        "steam",
        "steamwebhelper",
        "minecraftlauncher",
        "robloxplayerbeta",
        "valorant",
        "fortniteclient",
        "overwatch",
        "wow",
        "gta5",
        "eldenring",
        "cyberpunk2077",
        "bg3",
        "leagueclient",
    ]) {
        return "game".into();
    }
    if content_type == Some("social_feed")
        || contains_any(&["reels", "for you", "shorts feed", "infinite scroll"])
    {
        return "social_feed".into();
    }
    if content_type == Some("chat")
        || contains_any(&["telegram", "discord", "whatsapp", "messenger", "slack"])
    {
        return "chat".into();
    }
    if content_type == Some("docs_editor")
        || contains_any(&[
            "davinci resolve",
            "premiere pro",
            "after effects",
            "final cut",
            "visual studio code",
            "vscode",
            "intellij",
            "photoshop",
            "blender",
        ])
    {
        return "creator_tool".into();
    }
    if content_type == Some("video") || contains_any(&["youtube", "vimeo", "twitch"]) {
        return "video".into();
    }
    if content_type == Some("article") || content_type == Some("search_results") {
        return "research".into();
    }
    if (source == "app" || source == "screen")
        && contains_any(&[
            "windows shell experience host",
            "startmenuexperiencehost",
            "searchhost",
            "textinputhost",
            "lockapp",
        ])
    {
        return "system".into();
    }
    "unknown".into()
}

pub fn validate(
    mut policies: Vec<ClassificationPolicy>,
) -> Result<Vec<ClassificationPolicy>, String> {
    if policies.len() > 50 {
        return Err("At most 50 classification policies are allowed".into());
    }
    for policy in &mut policies {
        policy.id = policy.id.trim().to_ascii_lowercase();
        policy.name = policy.name.trim().to_string();
        policy.category = policy.category.trim().to_ascii_lowercase();
        if policy.id.is_empty()
            || !policy
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err("Policy ids must use lowercase letters, numbers, and dashes".into());
        }
        if policy.name.is_empty() {
            return Err("Every policy needs a name".into());
        }
        policy.kinds = policy
            .kinds
            .iter()
            .map(|kind| kind.trim().to_ascii_lowercase())
            .filter(|kind| !kind.is_empty())
            .collect();
        policy.kinds.sort();
        policy.kinds.dedup();
        policy.terms = policy
            .terms
            .iter()
            .map(|term| term.trim().to_ascii_lowercase())
            .filter(|term| !term.is_empty())
            .collect();
        policy.terms.sort();
        policy.terms.dedup();
        if (policy.terms.is_empty() && policy.kinds.is_empty())
            || policy.terms.len() > 50
            || policy.kinds.len() > 20
        {
            return Err(format!(
                "{} needs an activity type or between 1 and 50 terms",
                policy.name
            ));
        }
    }
    Ok(policies)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gameplay_policy_matches_ai_reason() {
        let policies = defaults();
        let (policy, term) = matched(
            &policies,
            "Short gameplay session without clear work-related context.",
            "game",
        )
        .unwrap();
        assert_eq!(policy.category, "distraction");
        assert_eq!(term, "game");
    }

    #[test]
    fn epic_library_path_is_detected_as_game() {
        assert_eq!(
            detect_activity_kind(
                "app",
                "Example",
                "",
                Some(r"C:\Program Files\Epic Games\Example\game.exe"),
                None,
                None,
                &[],
            ),
            "game"
        );
    }
    #[test]
    fn steam_library_path_is_detected_as_game() {
        assert_eq!(
            detect_activity_kind(
                "app",
                "Unknown",
                "",
                Some(r"C:\Games\Steam\steamapps\common\Example\game.exe"),
                None,
                None,
                &[],
            ),
            "game"
        );
    }
}
