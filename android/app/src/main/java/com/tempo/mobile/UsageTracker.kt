package com.tempo.mobile

import android.app.AppOpsManager
import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import java.util.Locale
import android.os.Process

/**
 * Reads per-app foreground time from [UsageStatsManager] and turns each completed
 * foreground interval into an [Event].
 *
 * Android only exposes *which app* was foreground and for how long — not in-app
 * detail. So "YouTube for 20 min" is captured; the video title is not. A phone
 * browser shows up as the browser app, not the visited domain. That is an OS
 * sandbox limit.
 */
object UsageTracker {

    /**
     * Completed events in the window, including the elapsed portion of any app
     * that is still open. The next scan starts at that boundary, so long sessions
     * become visible without waiting for an app switch while deterministic ids
     * still make retries safe.
     */
    data class Scan(val events: List<Event>, val openStart: Long?)

    /** Whether the user has granted "Usage access" in Settings (a special permission). */
    fun hasPermission(ctx: Context): Boolean {
        val appOps = ctx.getSystemService(Context.APP_OPS_SERVICE) as AppOpsManager
        val mode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            appOps.unsafeCheckOpNoThrow(
                AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), ctx.packageName,
            )
        } else {
            @Suppress("DEPRECATION")
            appOps.checkOpNoThrow(
                AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), ctx.packageName,
            )
        }
        return mode == AppOpsManager.MODE_ALLOWED
    }

    fun collect(ctx: Context, since: Long, now: Long): Scan {
        val usm = ctx.getSystemService(Context.USAGE_STATS_SERVICE) as UsageStatsManager
        // Look back far enough to discover an app that has stayed foreground for
        // several hours while Android delayed background work.
        val lookback = (since - 24L * 60L * 60L * 1000L).coerceAtLeast(0L)
        val events = usm.queryEvents(lookback, now)

        val out = ArrayList<Event>()
        val ev = UsageEvents.Event()
        var curPkg: String? = null
        var curStart = 0L

        while (events.getNextEvent(ev)) {
            when (ev.eventType) {
                UsageEvents.Event.MOVE_TO_FOREGROUND -> {
                    val p = curPkg
                    if (p != null && p != ev.packageName) emit(ctx, out, p, curStart, ev.timeStamp, since)
                    curPkg = ev.packageName
                    curStart = ev.timeStamp
                }
                UsageEvents.Event.MOVE_TO_BACKGROUND -> {
                    val p = curPkg
                    if (p != null && p == ev.packageName) {
                        emit(ctx, out, p, curStart, ev.timeStamp, since)
                        curPkg = null
                    }
                }
            }
        }
        // Upload the completed portion of a still-open session. `emit` clips its
        // start to the previous watermark, so later scans append rather than overlap.
        val openPackage = curPkg
        if (openPackage != null) emit(ctx, out, openPackage, curStart, now, since)
        return Scan(out, null)
    }

    private fun emit(
        ctx: Context,
        out: MutableList<Event>,
        pkg: String,
        start: Long,
        end: Long,
        since: Long,
    ) {
        if (end <= start) return
        if (end <= since) return                 // already covered by an earlier scan
        val s = start.coerceAtLeast(since)
        val durSec = (end - s) / 1000L
        if (durSec < 5L) return                  // ignore blips / quick switches
        if (pkg == ctx.packageName) return       // don't track ourselves
        out.add(Event.appSample(label(ctx, pkg), pkg, s, durSec))
    }

    /** How many times an app was opened (foreground) so far today — a "pickups" /
     *  doomscroll proxy for the status bar. Counts only package *changes*. */
    fun pickupsToday(ctx: Context): Int {
        val usm = ctx.getSystemService(Context.USAGE_STATS_SERVICE) as UsageStatsManager
        val cal = java.util.Calendar.getInstance()
        cal.set(java.util.Calendar.HOUR_OF_DAY, 0)
        cal.set(java.util.Calendar.MINUTE, 0)
        cal.set(java.util.Calendar.SECOND, 0)
        cal.set(java.util.Calendar.MILLISECOND, 0)
        val events = usm.queryEvents(cal.timeInMillis, System.currentTimeMillis())
        val ev = UsageEvents.Event()
        var count = 0
        var lastPkg: String? = null
        while (events.getNextEvent(ev)) {
            if (ev.eventType == UsageEvents.Event.MOVE_TO_FOREGROUND && ev.packageName != lastPkg) {
                if (ev.packageName != ctx.packageName) count++
                lastPkg = ev.packageName
            }
        }
        return count
    }

    /** Human label for a package (e.g. "YouTube"). Android 11+ package
     * visibility is declared in the manifest so this does not degrade to ids. */
    private fun label(ctx: Context, pkg: String): String {
        val pm = ctx.packageManager
        val installedLabel = try {
            pm.getApplicationLabel(pm.getApplicationInfo(pkg, 0)).toString().trim()
        } catch (_: PackageManager.NameNotFoundException) {
            ""
        } catch (_: SecurityException) {
            ""
        }
        if (installedLabel.isNotEmpty() && installedLabel != pkg) return installedLabel
        KNOWN_LABELS[pkg]?.let { return it }

        // Last-resort readability for an unusual package that disappeared
        // between UsageStats collection and label resolution.
        return pkg.substringAfterLast('.')
            .replace('_', ' ')
            .replaceFirstChar { char ->
                if (char.isLowerCase()) char.titlecase(Locale.getDefault()) else char.toString()
            }
    }

    private val KNOWN_LABELS = mapOf(
        "com.google.android.youtube" to "YouTube",
        "com.google.android.apps.youtube.music" to "YouTube Music",
        "com.android.chrome" to "Google Chrome",
        "com.google.android.gm" to "Gmail",
        "com.instagram.android" to "Instagram",
        "com.facebook.katana" to "Facebook",
        "com.facebook.orca" to "Messenger",
        "com.whatsapp" to "WhatsApp",
        "org.telegram.messenger" to "Telegram",
        "com.zhiliaoapp.musically" to "TikTok",
        "com.discord" to "Discord",
        "com.reddit.frontpage" to "Reddit",
        "com.twitter.android" to "X",
        "com.spotify.music" to "Spotify",
        "com.netflix.mediaclient" to "Netflix",
        "com.snapchat.android" to "Snapchat",
    )
}
