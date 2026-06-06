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
    }
}
