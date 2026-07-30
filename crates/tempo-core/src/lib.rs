//! Tempo core: GUI-free logic shared by the desktop app and the Tempo Hub
//! server. Contains the database schema/access, serde models, settings, the
//! classifier + scoring + streak + lock-in engines, and (added incrementally)
//! the aggregation cores and the generic synced-event model.
//!
//! Nothing here depends on `tauri`, `active-win-pos-rs`, or any windowing stack,
//! so it cross-compiles cleanly for a headless Raspberry Pi / Docker server.

pub mod accountability_export;
pub mod aggregate;
pub mod classify;
pub mod db;
pub mod events;
pub mod llm;
pub mod lockin;
pub mod models;
pub mod projects;
pub mod rules;
pub mod scoring;
pub mod semantic;
pub mod settings;
pub mod streaks;
