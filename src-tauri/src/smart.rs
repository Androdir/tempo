//! Personal Smart Tracking Mode (optional, OFF by default).
//!
//! Every N seconds (default 60) while enabled and the user is active:
//!   1. capture the screen *into memory*,
//!   2. run OCR locally (Windows.Media.Ocr — no network, no model files),
//!   3. summarize + extract keywords + classify,
//!   4. store ONLY: timestamp, app, window title, OCR summary, keywords, category.
//!
//! The pixel buffer is never written to disk and is dropped immediately after
//! OCR. If the OCR text looks like it contains a card number / SSN / OTP /
//! password context, the entire sample is discarded.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::{Local, Utc};
use rusqlite::params;

use crate::db::Db;
use crate::{platform, projects, settings};

const CHECK_INTERVAL: Duration = Duration::from_secs(5);
const IDLE_SKIP_SECS: u64 = 120;

pub fn start(db: Db) {
    std::thread::spawn(move || {
        #[cfg(windows)]
        init_winrt();

        // Force the first eligible tick to fire promptly after enabling.
        let mut last_capture = Instant::now()
            .checked_sub(Duration::from_secs(3600))
            .unwrap_or_else(Instant::now);

        loop {
            std::thread::sleep(CHECK_INTERVAL);

            let (enabled, interval) = match db.lock() {
                Ok(conn) => (
                    settings::get_bool(&conn, settings::SMART_TRACKING_ENABLED, false)
                        && settings::tracking_paused_until(&conn).is_none(),
                    settings::get_int(
                        &conn,
                        settings::SMART_INTERVAL,
                        settings::DEFAULT_SMART_INTERVAL,
                    )
                    .clamp(10, 3600) as u64,
                ),
                Err(_) => (false, 60),
            };

            if !enabled || last_capture.elapsed().as_secs() < interval {
                continue;
            }
            // Don't capture a screen the user has walked away from.
            if platform::idle_seconds() >= IDLE_SKIP_SECS {
                last_capture = Instant::now();
                continue;
            }
            let active_app = platform::active_window().0;
            let title_allowed = db
                .lock()
                .map(|conn| settings::title_capture_allowed(&conn, &active_app))
                .unwrap_or(false);
            if !title_allowed {
                last_capture = Instant::now();
                continue;
            }
            last_capture = Instant::now();

            // Capture + OCR happen outside the DB lock (OCR can take ~1s).
            if let Ok(text) = capture_and_ocr() {
                process_and_store(&db, &text);
            }
        }
    });
}

fn process_and_store(db: &Db, text: &str) {
    let cleaned = text.trim();
    if cleaned.chars().count() < 8 {
        return; // nothing useful on screen
    }
    if looks_sensitive(cleaned) {
        return; // discard the whole sample
    }

    let (app, title) = platform::active_window();
    let summary = summarize(cleaned);
    let kw = keywords(cleaned);
    let kw_json = serde_json::to_string(&kw).unwrap_or_else(|_| "[]".to_string());
    let extra = format!("{} {}", summary, kw.join(" "));

    let timestamp = Utc::now().to_rfc3339();
    let day = Local::now().format("%Y-%m-%d").to_string();

    if let Ok(conn) = db.lock() {
        if !settings::title_capture_allowed(&conn, &app) {
            return;
        }
        let project_list = projects::list_projects(&conn).unwrap_or_default();
        let app_cat = conn
            .query_row(
                "SELECT category FROM category_rules WHERE app_name = ?1",
                [&app],
                |r| r.get::<_, String>(0),
            )
            .ok();
        let base = app_cat.unwrap_or_else(|| "uncategorized".to_string());
        let (category, _reason, _pm) =
            projects::resolve(&project_list, &app, &title, &extra, &base, "screen ocr");

        let _ = conn.execute(
            "INSERT INTO smart_activity
               (timestamp, day, app_name, window_title, ocr_summary, detected_keywords, category, is_idle)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
            params![timestamp, day, app, title, summary, kw_json, category],
        );
    }
}

// ------------------------------------------------------------ summarization

fn summarize(text: &str) -> String {
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(300).collect()
}

const STOPWORDS: &[&str] = &[
    "the", "and", "for", "are", "was", "this", "that", "you", "your", "with", "from", "not", "but",
    "can", "will", "new", "all", "page", "home", "menu", "search", "sign", "log", "out",
    "settings", "file", "edit", "view", "help",
];

fn keywords(text: &str) -> Vec<String> {
    let mut freq: HashMap<String, u32> = HashMap::new();
    for raw in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        let w = raw.to_ascii_lowercase();
        if w.len() < 3 || STOPWORDS.contains(&w.as_str()) || w.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        *freq.entry(w).or_insert(0) += 1;
    }
    let mut items: Vec<(String, u32)> = freq.into_iter().collect();
    items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    items.into_iter().take(10).map(|(w, _)| w).collect()
}

// --------------------------------------------------- sensitive-content guard

fn looks_sensitive(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();

    if card_like(text) || ssn_like(text) {
        return true;
    }

    // A visible credential value, e.g. "password: hunter2" / "cvv = 123".
    // Requires a label followed by ':'/'=' then a value, so plain mentions
    // ("Change password", "Forgot password?") are NOT discarded.
    if has_credential_value(&lower) {
        return true;
    }

    const SECRET_WORDS: &[&str] = &[
        "password",
        "passcode",
        "cvv",
        "cvc",
        "one-time",
        "verification code",
        "2fa",
        "otp",
        "security code",
        "card number",
        "routing number",
        "account number",
        "social security",
        "seed phrase",
        "recovery phrase",
    ];
    let has_secret = SECRET_WORDS.iter().any(|w| lower.contains(w));
    if has_secret && max_contiguous_digits(text) >= 4 {
        return true;
    }

    let otp_context = lower.contains("otp")
        || lower.contains("verification")
        || lower.contains("one-time")
        || lower.contains("2fa")
        || lower.contains("authenticator");
    if otp_context && has_isolated_digit_group(text, 6) {
        return true;
    }

    false
}

/// True if a credential label is directly followed by `:`/`=` and a value
/// token of length >= 3 (alphanumeric). Masked values (•••) are ignored.
fn has_credential_value(lower: &str) -> bool {
    const LABELS: &[&str] = &["password", "passcode", "cvv", "cvc"];
    for label in LABELS {
        let mut start = 0;
        while let Some(pos) = lower[start..].find(label) {
            let idx = start + pos + label.len();
            let rest = lower[idx..].trim_start();
            if let Some(after) = rest.strip_prefix(':').or_else(|| rest.strip_prefix('=')) {
                let token: String = after
                    .trim_start()
                    .chars()
                    .take_while(|c| !c.is_whitespace())
                    .collect();
                if token.chars().filter(|c| c.is_ascii_alphanumeric()).count() >= 3 {
                    return true;
                }
            }
            start = idx;
        }
    }
    false
}

fn max_contiguous_digits(text: &str) -> usize {
    let (mut max, mut cur) = (0usize, 0usize);
    for c in text.chars() {
        if c.is_ascii_digit() {
            cur += 1;
            max = max.max(cur);
        } else {
            cur = 0;
        }
    }
    max
}

/// Card-like: a 13–19 digit contiguous run, or 3+ consecutive 4-digit groups.
fn card_like(text: &str) -> bool {
    let run = max_contiguous_digits(text);
    if (13..=19).contains(&run) {
        return true;
    }
    let mut consec4 = 0;
    for tok in text.split(|c: char| c == ' ' || c == '-') {
        if tok.len() == 4 && tok.chars().all(|c| c.is_ascii_digit()) {
            consec4 += 1;
            if consec4 >= 3 {
                return true;
            }
        } else if !tok.is_empty() {
            consec4 = 0;
        }
    }
    false
}

fn ssn_like(text: &str) -> bool {
    text.split(|c: char| c.is_whitespace()).any(|w| {
        let parts: Vec<&str> = w.split('-').collect();
        parts.len() == 3
            && parts[0].len() == 3
            && parts[1].len() == 2
            && parts[2].len() == 4
            && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit()))
    })
}

fn has_isolated_digit_group(text: &str, n: usize) -> bool {
    text.split(|c: char| !c.is_ascii_digit())
        .any(|g| g.len() == n)
}

// -------------------------------------------------------- capture + local OCR

/// Capture the primary screen into memory and OCR it. The returned text is the
/// only thing that leaves this function — the pixels are dropped here.
#[cfg(windows)]
fn capture_and_ocr() -> Result<String, String> {
    let (bgra, width, height) = capture_primary_bgra()?;
    ocr_bgra(&bgra, width, height)
    // `bgra` is dropped here — the screenshot is never written anywhere.
}

/// Grab the primary screen as a BGRA buffer via GDI (entirely in memory).
#[cfg(windows)]
fn capture_primary_bgra() -> Result<(Vec<u8>, i32, i32), String> {
    use core::ffi::c_void;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        SRCCOPY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

    unsafe {
        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);
        if width <= 0 || height <= 0 {
            return Err("invalid screen dimensions".into());
        }

        let hscreen = GetDC(HWND::default());
        if hscreen.0.is_null() {
            return Err("GetDC failed".into());
        }
        let hdc_mem = CreateCompatibleDC(hscreen);
        let hbmp = CreateCompatibleBitmap(hscreen, width, height);
        let old = SelectObject(hdc_mem, hbmp);

        let blit = BitBlt(hdc_mem, 0, 0, width, height, hscreen, 0, 0, SRCCOPY);

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // negative => top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut buf = vec![0u8; (width as usize) * (height as usize) * 4];
        let scanlines = GetDIBits(
            hdc_mem,
            hbmp,
            0,
            height as u32,
            Some(buf.as_mut_ptr() as *mut c_void),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        // Always release GDI resources.
        SelectObject(hdc_mem, old);
        let _ = DeleteObject(hbmp);
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(HWND::default(), hscreen);

        if blit.is_err() || scanlines == 0 {
            return Err("screen capture failed".into());
        }

        // 32-bpp BI_RGB is BGRA; force opaque alpha for the OCR bitmap.
        for px in buf.chunks_exact_mut(4) {
            px[3] = 255;
        }
        Ok((buf, width, height))
    }
}

#[cfg(windows)]
fn ocr_bgra(bgra: &[u8], width: i32, height: i32) -> Result<String, String> {
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Security::Cryptography::CryptographicBuffer;

    let buffer = CryptographicBuffer::CreateFromByteArray(bgra).map_err(|e| e.to_string())?;
    let bitmap =
        SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, width, height)
            .map_err(|e| e.to_string())?;
    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(|e| format!("OCR unavailable: {e}"))?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| e.to_string())?
        .get()
        .map_err(|e| e.to_string())?;
    Ok(result.Text().map_err(|e| e.to_string())?.to_string())
}

#[cfg(windows)]
fn init_winrt() {
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
    // Best-effort; ignore "already initialized".
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
}

#[cfg(not(windows))]
fn capture_and_ocr() -> Result<String, String> {
    Err("Smart Tracking (screen OCR) is only available on Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discards_credit_card_numbers() {
        assert!(looks_sensitive("Card 4111 1111 1111 1111 exp 12/26"));
        assert!(looks_sensitive("acct 4111111111111111"));
    }

    #[test]
    fn discards_otp_and_ssn() {
        assert!(looks_sensitive("Your verification code is 482913"));
        assert!(looks_sensitive("SSN 123-45-6789 on file"));
        assert!(looks_sensitive("Enter the password: hunter2"));
    }

    #[test]
    fn keeps_normal_text() {
        assert!(!looks_sensitive(
            "Hungarian Algorithm assignment problem notes"
        ));
        assert!(!looks_sensitive(
            "Meeting at 10 with 3 people about Q2 plans"
        ));
    }

    #[test]
    fn keywords_skip_stopwords_and_digits() {
        let k = keywords("The refactor fixed the refactor bug in the parser parser parser");
        assert!(k.contains(&"parser".to_string()));
        assert!(k.contains(&"refactor".to_string()));
        assert!(!k.contains(&"the".to_string()));
    }
}
