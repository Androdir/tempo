package com.tempo.mobile

import android.content.Context
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters

/**
 * Periodic background sync: read new foreground usage since the watermark, upload it
 * to the hub, and advance the watermark only on success. Because event ids are
 * deterministic, a failed upload simply retries next tick with no risk of
 * double-counting.
 */
class SyncWorker(ctx: Context, params: WorkerParameters) : CoroutineWorker(ctx, params) {

    override suspend fun doWork(): Result {
        val prefs = Prefs(applicationContext)
        if (!prefs.isPaired()) return Result.success()
        if (!UsageTracker.hasPermission(applicationContext)) {
            prefs.setSyncError("Usage access is not granted")
            return Result.success()
        }

        val now = System.currentTimeMillis()
        val wm = prefs.watermark()
        // First run: backfill only the last 24h. Otherwise never reach back past ~7d
        // (UsageStats retention is limited, and we don't want a huge first batch).
        val since = if (wm == 0L) now - DAY_MS else wm.coerceAtLeast(now - 7L * DAY_MS)

        val scan = UsageTracker.collect(applicationContext, since, now)
        return try {
            if (scan.events.isNotEmpty()) {
                HubClient.postEvents(
                    prefs.hubUrl(), prefs.token(), prefs.deviceId(), scan.events,
                ) { uploaded ->
                    // Persist progress after every acknowledged batch. If a later
                    // batch times out, retry resumes here instead of sending the
                    // whole day again.
                    val last = uploaded.last()
                    prefs.setWatermark(last.startMillis + last.durationSeconds * 1000L)
                }
            }
            // The collector includes the completed portion of the currently open
            // app, so the next scan can always continue from `now`.
            prefs.setWatermark(scan.openStart ?: now)
            prefs.setSyncSuccess(now, scan.events.size)
            Result.success()
        } catch (e: Exception) {
            // Leave the watermark untouched; deterministic ids make the retry safe.
            prefs.setSyncError(e.message ?: "${e.javaClass.simpleName} while contacting the Hub")
            Result.retry()
        }
    }

    companion object {
        private const val DAY_MS = 24L * 60L * 60L * 1000L
    }
}
