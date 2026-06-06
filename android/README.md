# Tempo Mobile (Android)

A **lightweight phone companion** for [Tempo](../README.md). It does two things:

1. **Tracks which apps you use** (via Android's `UsageStatsManager`) and pushes that time
   to your **Tempo Hub**, so phone usage shows up on the same shared dashboard as your
   desktop — "watched YouTube on my phone for 20 min" becomes a block in Today / Timeline /
   Daily Score, and counts against an active focus session.
2. **Shows the hub dashboard** in a WebView (the exact same web app the hub already serves),
   so you get check-ins, goals, review, streaks, and the timeline on your phone — no separate
   mobile UI to maintain.

It is intentionally minimal: a setup screen + a WebView + a background sync job. All the
analytics live on the hub.

---

## What it can and can't see

Android only exposes **which app was in the foreground and for how long** — not what's on
screen inside it. So:

- ✅ Per-app time: YouTube, Instagram, TikTok, Chrome, your editor, etc. — attributed to the
  right day and classified by the hub's existing rules (the app's display name, e.g.
  "YouTube", is sent, so it classifies just like the desktop).
- ❌ In-app detail: which video, which Reel, which website inside the browser. A phone browser
  is recorded as the **browser app** ("Chrome"), not the domain.

This is an OS sandbox limit, identical for every usage-tracking app on the Play Store.

---

## Build it

You have the Android SDK and JDK 17 already, so this builds as-is. **Easiest path: open it
in Android Studio.**

1. **Android Studio** → *Open* → select this `android/` folder. Studio will create the
   Gradle wrapper, write `local.properties` (pointing at your SDK), sync, and let you
   **Run ▶** on a connected phone or emulator.

2. **Command line** (if you prefer): from `android/`, with `ANDROID_HOME` set (it is) and
   the Gradle wrapper present:
   ```bash
   ./gradlew assembleDebug        # gradlew.bat on Windows
   # APK -> app/build/outputs/apk/debug/app-debug.apk
   ```
   > The binary `gradle/wrapper/gradle-wrapper.jar` is **not** checked in (it can't be
   > generated as text). Opening once in Android Studio creates it, or run `gradle wrapper`
   > if you have a system Gradle. After that `./gradlew` works.

Toolchain pinned in the build files: AGP 8.5.2 · Gradle 8.7 · Kotlin 1.9.24 ·
compileSdk/targetSdk 34 · minSdk 26 (Android 8.0+).

---

## Set it up on your phone

First make sure the **hub is reachable from the phone**: run it with `TEMPO_BIND=0.0.0.0`
on your LAN (it prints a warning — that's expected), or put both devices on **Tailscale**.
The hub URL is then `http://<pi-lan-ip>:7700` (or your Tailscale name).

In the app:

1. **Grant usage access** (button 1) → flips on "Usage access" for Tempo in Android Settings.
   This is the special permission Android requires; it can't be granted from a normal dialog.
2. Enter the **hub URL** and the **pairing secret** (`TEMPO_PAIRING_SECRET` on the hub).
3. **Pair & start tracking** (button 2). The app pairs as its own device, kicks off an
   immediate sync, then drops you into the dashboard.

From then on a background job uploads new usage every ~15 minutes (WorkManager's minimum),
buffering through any hub downtime. To re-pair against a different hub, clear the app's data.

---

## How it works (for the next dev)

- `UsageTracker` reconstructs foreground intervals from `UsageStatsManager.queryEvents` and
  emits one `app_sample` event per completed interval. Event ids are deterministic
  (`android:<package>:<startMillis>`), so retries/overlapping scans **dedupe on the hub** and
  never double-count. A still-open app isn't emitted until it backgrounds; its start becomes
  the next watermark.
- `HubClient` pairs (`POST /api/pair`) and uploads (`POST /api/events`) with the device token,
  using the **same wire format** as the desktop sync (`tempo_core::events::SyncEvent`). No new
  hub code was needed — phone events flow through the existing ingest → `activity_log` →
  aggregation, including the cross-device focus summary.
- `SyncWorker` (WorkManager, every 15 min, network-constrained) scans since the watermark,
  uploads, and advances the watermark only on success.
- `MainActivity` shows setup when unpaired, else a WebView of the hub with the pairing secret
  injected as `tempo_web_token` so the dashboard authenticates silently. A thin status strip on
  top shows "✅ Tracking on · N app opens today · synced Nm ago" (tap to sync now, or to grant
  Usage access if it's paused).
- The pairing token + secret are stored with **EncryptedSharedPreferences** (AES-256, key in the
  Android Keystore), not in plaintext prefs.

### Not yet (future)
- Mirroring an *in-progress* desktop focus session live to the phone (the cross-device
  **summary** already lands once a session ends).
- A native "today on this phone" mini-view (right now the phone view is the hub's web app).
- iOS — Apple does not expose per-app usage to third-party apps, so an iPhone stays
  companion-only (the web dashboard works in Safari, but it can't track).
