//! Android package identifiers are stable but not useful to a person. The
//! mobile app normally sends Android's display label; this small fallback also
//! keeps Hub imports and historical records readable if package visibility
//! prevented that lookup on the phone.

pub const KNOWN_ANDROID_APPS: &[(&str, &str)] = &[
    ("com.google.android.youtube", "YouTube"),
    ("com.google.android.apps.youtube.music", "YouTube Music"),
    ("com.android.chrome", "Google Chrome"),
    ("com.google.android.gm", "Gmail"),
    ("com.google.android.apps.maps", "Google Maps"),
    ("com.google.android.apps.photos", "Google Photos"),
    ("com.google.android.apps.docs", "Google Drive"),
    ("com.google.android.googlequicksearchbox", "Google"),
    ("com.google.android.apps.messaging", "Google Messages"),
    ("com.android.vending", "Google Play Store"),
    ("com.instagram.android", "Instagram"),
    ("com.facebook.katana", "Facebook"),
    ("com.facebook.orca", "Messenger"),
    ("com.whatsapp", "WhatsApp"),
    ("org.telegram.messenger", "Telegram"),
    ("org.telegram.messenger.web", "Telegram"),
    ("com.zhiliaoapp.musically", "TikTok"),
    ("com.ss.android.ugc.trill", "TikTok"),
    ("com.discord", "Discord"),
    ("com.reddit.frontpage", "Reddit"),
    ("com.twitter.android", "X"),
    ("com.spotify.music", "Spotify"),
    ("com.netflix.mediaclient", "Netflix"),
    ("tv.twitch.android.app", "Twitch"),
    ("com.snapchat.android", "Snapchat"),
    ("com.lemon.lvoverseas", "CapCut"),
    ("com.sec.android.app.sbrowser", "Samsung Internet"),
    ("com.sec.android.gallery3d", "Gallery"),
    ("com.sec.android.app.launcher", "One UI Home"),
    ("com.valvesoftware.android.steam.community", "Steam"),
];

pub fn known_android_label(package: &str) -> Option<&'static str> {
    KNOWN_ANDROID_APPS
        .iter()
        .find_map(|(candidate, label)| candidate.eq_ignore_ascii_case(package).then_some(*label))
}

pub fn looks_like_package_id(value: &str) -> bool {
    let mut parts = value.split('.');
    parts.clone().count() >= 2
        && parts.all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_packages_have_human_labels() {
        assert_eq!(
            known_android_label("com.google.android.youtube"),
            Some("YouTube")
        );
        assert_eq!(known_android_label("COM.WHATSAPP"), Some("WhatsApp"));
        assert!(looks_like_package_id("org.telegram.messenger"));
        assert!(!looks_like_package_id("YouTube"));
    }
}
