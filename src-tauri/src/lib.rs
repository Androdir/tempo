// GUI-free logic now lives in the shared `tempo-core` crate. Re-export it at the
// crate root so existing `crate::db` / `crate::models` / … paths keep resolving.
pub use tempo_core::{
    accountability_export, aggregate, classify, db, llm, lockin, models, projects, rules, scoring,
    semantic, settings, streaks,
};

mod accountability;
mod commands;
mod ingest;
mod output;
mod platform;
mod server;
mod secrets;
mod smart;
mod sync;
mod tracker;

#[cfg(desktop)]
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;
#[cfg(desktop)]
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, WindowEvent,
};

#[cfg(desktop)]
static TRAY_HINT_SHOWN: AtomicBool = AtomicBool::new(false);

#[cfg(desktop)]
fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(desktop)]
fn set_tracking_pause(app: &AppHandle, until: Option<String>, label: &str) {
    let database = app.state::<db::Db>();
    if let Ok(conn) = database.lock() {
        let _ = settings::set_tracking_paused_until(&conn, until.as_deref());
    }
    let _ = accountability::send_native_notification(app, "Tempo tracking", label);
    let _ = app.emit("tracking-updated", ());
}

#[cfg(desktop)]
fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Tempo", true, None::<&str>)?;
    let pause15 = MenuItem::with_id(
        app,
        "pause15",
        "Pause tracking for 15 minutes",
        true,
        None::<&str>,
    )?;
    let pause60 = MenuItem::with_id(
        app,
        "pause60",
        "Pause tracking for 1 hour",
        true,
        None::<&str>,
    )?;
    let pause_tomorrow = MenuItem::with_id(
        app,
        "pause_tomorrow",
        "Pause until tomorrow",
        true,
        None::<&str>,
    )?;
    let resume = MenuItem::with_id(app, "resume", "Resume tracking", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Tempo", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open, &pause15, &pause60, &pause_tomorrow, &resume, &quit],
    )?;

    let mut tray = TrayIconBuilder::with_id("tempo-tray")
        .tooltip("Tempo — tracking in the background")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "pause15" => set_tracking_pause(
                app,
                Some((chrono::Utc::now() + chrono::Duration::minutes(15)).to_rfc3339()),
                "Tracking paused for 15 minutes. Resume any time from the tray.",
            ),
            "pause60" => set_tracking_pause(
                app,
                Some((chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339()),
                "Tracking paused for 1 hour. Resume any time from the tray.",
            ),
            "pause_tomorrow" => {
                let tomorrow = chrono::Local::now().date_naive() + chrono::Duration::days(1);
                let until = tomorrow
                    .and_hms_opt(0, 0, 0)
                    .and_then(|naive| naive.and_local_timezone(chrono::Local).single())
                    .map(|local| local.with_timezone(&chrono::Utc).to_rfc3339());
                set_tracking_pause(app, until, "Tracking paused until tomorrow.");
            }
            "resume" => set_tracking_pause(app, None, "Tracking resumed."),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // Must be registered first: reopening Tempo focuses the existing tray
    // process instead of starting a second tracker and duplicating samples.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        show_main_window(app);
    }));
    let builder = builder.plugin(tauri_plugin_notification::init());
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        Some(vec!["--background"]),
    ));
    #[cfg(desktop)]
    let builder = builder.on_window_event(|window, event| {
        if window.label() != "main" {
            return;
        }
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = window.hide();
            if !TRAY_HINT_SHOWN.swap(true, Ordering::Relaxed) {
                let _ = accountability::send_native_notification(
                    window.app_handle(),
                    "Tempo is still running",
                    "Tracking continues in the system tray. Use the tray icon to reopen or quit Tempo.",
                );
            }
        }
    });

    builder
        .setup(|app| {
            #[cfg(desktop)]
            setup_tray(app)?;

            // The SQLite file lives in the OS app-data dir — entirely local.
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir).ok();
            let db_path = dir.join("productivity.db");
            let database_existed = db_path.exists()
                && std::fs::metadata(&db_path)
                    .map(|metadata| metadata.len() > 0)
                    .unwrap_or(false);

            let database =
                db::init(&db_path).map_err(|e| format!("failed to open database: {e}"))?;

            // Seed defaults once and read the loopback endpoint config.
            let (token, port, retention) = {
                let conn = database.lock().map_err(|e| e.to_string())?;
                settings::ensure_defaults(&conn)
                    .map_err(|e| format!("failed to seed settings: {e}"))?;
                models::ensure_category_defaults(&conn)
                    .map_err(|e| format!("failed to seed categories: {e}"))?;
                models::ensure_checkin_defaults(&conn)
                    .map_err(|e| format!("failed to seed check-ins: {e}"))?;
                scoring::ensure_rule_defaults(&conn)
                    .map_err(|e| format!("failed to seed score rules: {e}"))?;
                // The installed release opts in once, as requested. Later user changes
                // are read from the OS and are never overwritten on restart.
                #[cfg(all(desktop, not(debug_assertions)))]
                if settings::get_setting(&conn, settings::LAUNCH_AT_LOGIN_INITIALIZED).is_none() {
                    use tauri_plugin_autostart::ManagerExt;
                    let _ = app.autolaunch().enable();
                    let _ =
                        settings::set_setting(&conn, settings::LAUNCH_AT_LOGIN_INITIALIZED, "1");
                }
                // Maintenance uses a separate connection after the first paint.
                let retention = settings::get_int(
                    &conn,
                    settings::RETENTION_DAYS,
                    settings::DEFAULT_RETENTION_DAYS,
                );
                (
                    settings::ingest_token(&conn),
                    settings::ingest_port(&conn),
                    retention,
                )
            };

            app.manage(database.clone());
            db::start_startup_maintenance(db_path, database_existed, retention);

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
            // Load the cloud key from the OS credential vault before the AI worker starts.
            secrets::load_openai_key_into_environment();
            // Optional background AI classifier (gated by llm_enabled).
            llm::start(database);

            // Windows-login launches stay out of the way while tracking begins.
            #[cfg(desktop)]
            if std::env::args_os().any(|arg| arg.to_string_lossy() == "--background") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_today_summary,
            commands::get_time_breakdown,
            commands::show_native_notification,
            commands::get_launch_at_login,
            commands::set_launch_at_login,
            commands::get_tracked_apps,
            commands::get_category_rules,
            commands::get_category_definitions,
            commands::get_classification_policies,
            commands::set_classification_policies,
            commands::upsert_category_definition,
            commands::delete_category_definition,
            commands::set_category_rule,
            commands::delete_category_rule,
            commands::get_daily_review,
            commands::get_browser_activity,
            commands::get_activity_details,
            commands::get_domain_rules,
            commands::get_tracked_domains,
            commands::set_domain_rule,
            commands::delete_domain_rule,
            commands::get_privacy_settings,
            commands::set_tracking_pause,
            commands::set_privacy_setting,
            commands::purge_raw_content,
            commands::delete_all_captured_content,
            commands::get_projects,
            commands::create_project,
            commands::update_project,
            commands::delete_project,
            commands::test_project_match,
            commands::exclude_activity_from_project,
            commands::get_recent_activity,
            commands::correct_activity,
            commands::get_correction_history,
            commands::undo_correction,
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
            commands::add_streak_definition,
            sync::read_from_hub,
            commands::delete_streak_definition,
            commands::seed_default_streaks,
            commands::generate_lockin_plan,
            commands::get_lockin_plan,
            commands::save_lockin_plan,
            commands::copy_lockin_plan_to_goals,
            commands::get_lockin_auto,
            commands::set_lockin_auto,
            sync::get_sync_status,
            sync::set_app_mode,
            sync::pair_with_hub,
            sync::get_device_breakdown,
            sync::import_history_to_hub,
            sync::sync_configuration_now,
            commands::get_llm_settings,
            commands::set_llm_setting,
            commands::test_ollama_connection,
            commands::set_openai_api_key,
            commands::clear_openai_api_key,
            commands::test_openai_connection,
            commands::get_llm_errors,
            commands::get_daily_score,
            commands::set_checkin,
            commands::clear_checkin,
            commands::get_checkins,
            commands::get_checkin_definitions,
            commands::upsert_checkin_definition,
            commands::delete_checkin_definition,
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
            commands::get_score_rules,
            commands::upsert_score_rule,
            commands::delete_score_rule,
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
            commands::list_database_backups,
            commands::create_database_backup,
            commands::restore_database_backup,
            commands::generate_accountability_export,
            commands::save_accountability_export,
            commands::get_tracking_health,
            commands::reset_database,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
