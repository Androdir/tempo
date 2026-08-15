package com.tempo.mobile

import android.content.Context
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

/** Tiny SharedPreferences wrapper for pairing + the upload watermark. */
class Prefs(ctx: Context) {
    // The pairing token + secret are sensitive, so persist them encrypted at rest
    // (AES-256, key held in the Android Keystore). Fall back to plain prefs only if
    // the keystore is somehow unavailable, so the app still functions.
    private val sp = run {
        val app = ctx.applicationContext
        try {
            val masterKey = MasterKey.Builder(app)
                .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                .build()
            EncryptedSharedPreferences.create(
                app,
                "tempo_secure",
                masterKey,
                EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
                EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
            )
        } catch (e: Exception) {
            app.getSharedPreferences("tempo", Context.MODE_PRIVATE)
        }
    }

    fun hubUrl(): String = (sp.getString(HUB_URL, "") ?: "").trimEnd('/')
    fun token(): String = sp.getString(TOKEN, "") ?: ""
    fun deviceId(): String = sp.getString(DEVICE_ID, "") ?: ""
    fun secret(): String = sp.getString(SECRET, "") ?: ""
    fun isPaired(): Boolean = hubUrl().isNotEmpty() && token().isNotEmpty()

    fun watermark(): Long = sp.getLong(WATERMARK, 0L)
    fun setWatermark(v: Long) = sp.edit().putLong(WATERMARK, v).apply()

    fun lastSync(): Long = sp.getLong(LAST_SYNC, 0L)
    fun setLastSync(v: Long) = sp.edit().putLong(LAST_SYNC, v).apply()
    fun lastSyncError(): String = sp.getString(LAST_SYNC_ERROR, "") ?: ""
    fun lastSyncEventCount(): Int = sp.getInt(LAST_SYNC_EVENT_COUNT, 0)
    fun syncInProgress(): Boolean = sp.getBoolean(SYNC_IN_PROGRESS, false)

    fun setSyncStarted() {
        sp.edit()
            .putBoolean(SYNC_IN_PROGRESS, true)
            .remove(LAST_SYNC_ERROR)
            .apply()
    }

    fun setSyncSuccess(at: Long, eventCount: Int) {
        sp.edit()
            .putLong(LAST_SYNC, at)
            .putInt(LAST_SYNC_EVENT_COUNT, eventCount)
            .putBoolean(SYNC_IN_PROGRESS, false)
            .remove(LAST_SYNC_ERROR)
            .apply()
    }

    fun setSyncError(message: String) {
        sp.edit()
            .putString(LAST_SYNC_ERROR, message.trim().take(180))
            .putBoolean(SYNC_IN_PROGRESS, false)
            .apply()
    }

    /** Persist pairing. The secret doubles as the web-dashboard token (`tempo_web_token`). */
    fun savePairing(url: String, token: String, deviceId: String, secret: String) {
        sp.edit()
            .putString(HUB_URL, url.trimEnd('/'))
            .putString(TOKEN, token)
            .putString(DEVICE_ID, deviceId)
            .putString(SECRET, secret)
            .apply()
    }

    companion object {
        private const val HUB_URL = "hub_url"
        private const val TOKEN = "hub_token"
        private const val DEVICE_ID = "device_id"
        private const val SECRET = "web_secret"
        private const val WATERMARK = "usage_watermark"
        private const val LAST_SYNC = "last_sync"
        private const val LAST_SYNC_ERROR = "last_sync_error"
        private const val LAST_SYNC_EVENT_COUNT = "last_sync_event_count"
        private const val SYNC_IN_PROGRESS = "sync_in_progress"
    }
}
