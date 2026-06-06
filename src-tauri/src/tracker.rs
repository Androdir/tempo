use crate::db::Db;
use crate::platform;
use crate::settings;
use chrono::{Local, Utc};
use rusqlite::params;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// How often we sample the foreground window.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(10);

/// Spawn the background sampling loop. Runs for the lifetime of the app.
pub fn start(db: Db, app: AppHandle) {
    std::thread::spawn(move || {
        let mut last = Instant::now();
        // WinRT init so media-playback detection works on this thread (no-op off Windows).
        platform::init_media_detection();

        loop {
            std::thread::sleep(SAMPLE_INTERVAL);

            // Measure real elapsed time so durations stay correct even if the
            // OS suspends/throttles the thread.
            let elapsed = last.elapsed().as_secs().max(1) as i64;
            last = Instant::now();

            let idle_secs = platform::idle_seconds();
            let (app_name, title) = platform::active_window();

            let timestamp = Utc::now().to_rfc3339();
            let day = Local::now().format("%Y-%m-%d").to_string();

            if let Ok(conn) = db.lock() {
                // Idle threshold is user-configurable (Privacy settings) — re-read each
                // tick so changes take effect without a restart.
                let threshold =
                    settings::get_int(&conn, settings::IDLE_THRESHOLD, settings::IDLE_SECONDS)
                        .clamp(20, 1800) as u64;
                let idle_by_input = idle_secs >= threshold;
                // Passively watching/listening (media playing) shouldn't count as idle
                // when the user has opted in — keeps a video/lecture from being dropped.
                let is_idle = idle_by_input
                    && !(settings::get_bool(&conn, settings::COUNT_MEDIA_ACTIVE, true)
                        && platform::media_playing());
                let _ = conn.execute(
                    "INSERT INTO activity_log
                       (timestamp, day, app_name, window_title, duration_seconds, is_idle)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![timestamp, day, app_name, title, elapsed, is_idle as i64],
                );
            }

            // Best-effort: let the dashboard refresh live. Ignored if no listener.
            let _ = app.emit("tracking-updated", ());
        }
    });
}
