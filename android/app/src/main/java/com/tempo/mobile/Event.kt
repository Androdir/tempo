package com.tempo.mobile

import org.json.JSONObject
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone

/**
 * One activity event in the hub's wire format (mirrors `tempo_core::events::SyncEvent`,
 * camelCase). A phone foreground-app interval maps to an `app_sample`, exactly like a
 * desktop window sample, so it flows through the same ingest → activity_log → dashboard.
 *
 * `eventId` is deterministic (`android:<package>:<startMillis>`) so re-uploads dedupe on
 * the hub's `(device_id, event_id)` and never double-count.
 */
data class Event(
    val eventId: String,
    val appName: String,
    val pkg: String,
    val startMillis: Long,
    val durationSeconds: Long,
) {
    fun toJson(): JSONObject {
        val meta = JSONObject()
            .put("isIdle", false)
            .put("package", pkg)
        return JSONObject()
            .put("eventId", eventId)
            .put("eventType", "app_sample")
            .put("source", "android")
            .put("timestamp", utc(startMillis))
            .put("day", localDay(startMillis))
            .put("appName", appName)
            .put("durationSeconds", durationSeconds)
            .put("metadata", meta)
    }

    companion object {
        fun appSample(appName: String, pkg: String, startMillis: Long, durationSeconds: Long): Event =
            Event("android:$pkg:$startMillis", appName, pkg, startMillis, durationSeconds)

        /** RFC3339 / ISO-8601 in UTC (e.g. 2026-06-04T12:00:00Z) — the hub parses this. */
        private fun utc(ms: Long): String {
            val f = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss'Z'", Locale.US)
            f.timeZone = TimeZone.getTimeZone("UTC")
            return f.format(Date(ms))
        }

        /** The *local* calendar day, so phone activity lands on the same day the
         *  desktop would record (the hub aggregates by this `day`). */
        private fun localDay(ms: Long): String =
            SimpleDateFormat("yyyy-MM-dd", Locale.US).format(Date(ms))
    }
}
