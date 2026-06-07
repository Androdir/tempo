// GUI-free logic now lives in the shared `tempo-core` crate. Re-export it at the
// crate root so existing `crate::db` / `crate::models` / … paths keep resolving.
pub use tempo_core::{
    aggregate, classify, db, llm, lockin, models, projects, rules, scoring, settings, streaks,
};

mod accountability;
mod commands;
mod ingest;
mod output;
mod platform;
mod server;
mod smart;
mod sync;
mod tracker;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // The SQLite file lives in the OS app-data dir — entirely local.
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir).ok();
            let db_path = dir.join("productivity.db");

            let database =
                db::init(&db_path).map_err(|e| format!("failed to open database: {e}"))?;

            // Seed defaults once and read the loopback endpoint config.
            let (token, port) = {
                let conn = database.lock().map_err(|e| e.to_string())?;
                settings::ensure_defaults(&conn)
                    .map_err(|e| format!("failed to seed settings: {e}"))?;
                // Enforce data retention once at startup.
                let retention =
                    settings::get_int(&conn, settings::RETENTION_DAYS, settings::DEFAULT_RETENTION_DAYS);
                let _ = db::prune(&conn, retention);
                (settings::ingest_token(&conn), settings::ingest_port(&conn))
            };

            app.manage(database.clone());

            // Desktop window tracker + loopback ingest endpoint for the extension.
            tracker::start(database.clone(), app.handle().clone());
            server::start(database.clone(), token, port);
            // Optional screen-OCR loop (gated by the smart_tracking_enabled setting).
            smart::start(database.clone());
            // Accountability watcher: distraction warnings, focus-mode nudges,
            // and the end-of-day review prompt (all local, soft enforcement).
            accountability::start(database.clone(), app.handle().clone());
            // Proof-of-output folder watcher (metadata only; gated by a setting).
            output::start(database.clone(), app.handle().clone());
            // Tempo Hub sync worker (idle unless app_mode = hub).
            sync::start(database.clone(), app.handle().clone());
            // Optional background local-LLM classifier (gated by llm_enabled).
            llm::start(database);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_today_summary,
            commands::get_tracked_apps,
            commands::get_category_rules,
            commands::set_category_rule,
            commands::delete_category_rule,
            commands::get_daily_review,
            commands::insert_sample_data,
            commands::get_browser_activity,
            commands::get_activity_details,
            commands::get_domain_rules,
            commands::get_tracked_domains,
            commands::set_domain_rule,
            commands::delete_domain_rule,
            commands::get_privacy_settings,
            commands::set_privacy_setting,
            commands::purge_raw_content,
            commands::delete_all_captured_content,
            commands::get_projects,
            commands::create_project,
            commands::update_project,
            commands::delete_project,
            commands::get_recent_activity,
            commands::correct_activity,
            commands::get_timeline_for_day,
            commands::get_output_events,
            commands::get_watched_folders,
            commands::add_watched_folder,
            commands::update_watched_folder,
            commands::remove_watched_folder,
            commands::link_output_events_to_blocks,
            commands::scan_outputs_now,
            commands::get_streaks,
            commands::get_streak_definitions,
            commands::update_streak_definition,
            commands::generate_lockin_plan,
            commands::get_lockin_plan,
            commands::save_lockin_plan,
            commands::copy_lockin_plan_to_goals,
            commands::get_lockin_auto,
            commands::set_lockin_auto,
            sync::get_sync_status,
            sync::set_app_mode,
            sync::pair_with_hub,
            sync::import_history_to_hub,
            commands::get_llm_settings,
            commands::set_llm_setting,
            commands::test_ollama_connection,
            commands::get_llm_errors,
            commands::get_daily_score,
            commands::set_checkin,
            commands::get_checkins,
            commands::get_goals,
            commands::add_goal,
            commands::update_goal,
            commands::toggle_goal,
            commands::delete_goal,
            commands::set_goal_recurring,
            commands::copy_previous_goals,
            commands::set_scoring_weight,
            commands::set_scoring_threshold,
            commands::reset_scoring_weights,
            commands::generate_daily_review,
            commands::set_daily_note,
            commands::start_focus_session,
            commands::get_focus_session,
            commands::end_focus_session,
            commands::get_focus_summary,
            commands::get_accountability_settings,
            commands::set_accountability_setting,
            commands::set_distraction_snooze,
            commands::set_distraction_intentional,
            commands::get_weekly_review,
            commands::prune_old_data,
            commands::reset_database,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
