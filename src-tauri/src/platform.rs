//! OS-specific helpers for reading the active window and the user's idle time.
//!
//! Privacy note: we only ever read
//!   (a) the *title* of the foreground window and the *name* of its process,
//!   (b) *how long* it has been since the last input event, and
//!   (c) whether media is *playing* — a play/pause flag from the OS media
//!       controls, never the audio itself, the track/title, or any content.
//! We never read key codes, typed text, mouse coordinates, the clipboard,
//! audio samples, or screen contents. The idle check below uses the OS-provided
//! "time of last input" value only — not the input itself.

use std::path::Path;

/// `(app_name, window_title)` for screen OCR and other callers that do not need
/// executable identity.
pub fn active_window() -> (String, String) {
    let (app, title, _) = active_window_detailed();
    (app, title)
}

/// Foreground app plus its executable path. The path is never shown publicly or
/// sent to the browser extension; it lets the local classifier recognise Steam
/// libraries and stable executables even when a window title is unhelpful.
pub fn active_window_detailed() -> (String, String, Option<String>) {
    match active_win_pos_rs::get_active_window() {
        Ok(win) => {
            let app = if win.app_name.trim().is_empty() {
                fallback_from_path(&win.process_path)
            } else {
                win.app_name
            };
            let path = (!win.process_path.as_os_str().is_empty())
                .then(|| win.process_path.to_string_lossy().to_string());
            (app, win.title, path)
        }
        // No focused window (locked screen, desktop, transitions, etc.).
        Err(_) => ("Unknown".to_string(), String::new(), None),
    }
}

fn fallback_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

/// Seconds since the last keyboard/mouse activity.
#[cfg(windows)]
pub fn idle_seconds() -> u64 {
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };

    // SAFETY: `info` is a valid, fully-initialized LASTINPUTINFO with cbSize set.
    unsafe {
        if GetLastInputInfo(&mut info).as_bool() {
            let now = GetTickCount();
            // dwTime is a tick count; subtraction is wrapping by design.
            let idle_ms = now.wrapping_sub(info.dwTime);
            (idle_ms / 1000) as u64
        } else {
            0
        }
    }
}

/// Non-Windows fallback: treat the user as always active (idle detection is a
/// Windows feature for this MVP).
#[cfg(not(windows))]
pub fn idle_seconds() -> u64 {
    0
}

/// Initialize WinRT for the calling thread, required before `media_playing()`.
/// Call once per thread; best-effort (ignores "already initialized").
#[cfg(windows)]
pub fn init_media_detection() {
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
    // SAFETY: idempotent thread-init; re-init just returns S_FALSE.
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
}

#[cfg(not(windows))]
pub fn init_media_detection() {}

/// Whether any media session is currently *playing* (a video, music, a lecture…),
/// per the Windows Global System Media Transport Controls. Lets "I'm watching
/// something" avoid being marked idle even with no keyboard/mouse input. Reads
/// only the playback *status* — never audio, titles, or content. Returns false if
/// nothing is playing, no session exists, or on any error.
#[cfg(windows)]
pub fn media_playing() -> bool {
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSessionManager as MediaManager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
    };
    let probe = || -> windows::core::Result<bool> {
        // RequestAsync returns an IAsyncOperation; `.get()` blocks for the result.
        let manager = MediaManager::RequestAsync()?.get()?;
        let session = manager.GetCurrentSession()?;
        let status = session.GetPlaybackInfo()?.PlaybackStatus()?;
        Ok(status == PlaybackStatus::Playing)
    };
    probe().unwrap_or(false)
}

#[cfg(not(windows))]
pub fn media_playing() -> bool {
    false
}
