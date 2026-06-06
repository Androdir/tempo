package com.tempo.mobile

import android.app.AppOpsManager
import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
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
     * Completed events in the window, plus the start time of any *still-open*
     * foreground app. The caller sets the watermark to that start and re-scans it
     * next run, so a session straddling two scans is neither lost nor double-counted
     * (event ids are deterministic, so the hub dedupes any re-emission).
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
        // Look back so an app already foreground at `since` is still discoverable.
        val lookback = (since - 60L * 60L * 1000L).coerceAtLeast(0L)
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
        // Whatever is still open at `now` is provisional — report its start, don't emit.
        val openStart = if (curPkg != null) curStart else null
        return Scan(out, openStart)
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

    /** Human label for a package (e.g. "YouTube"), falling back to the package name. */
    private fun label(ctx: Context, pkg: String): String = try {
        val pm = ctx.packageManager
        pm.getApplicationLabel(pm.getApplicationInfo(pkg, 0)).toString()
    } catch (e: PackageManager.NameNotFoundException) {
        pkg
    }
}
