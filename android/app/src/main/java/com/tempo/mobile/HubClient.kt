package com.tempo.mobile

import org.json.JSONArray
import org.json.JSONObject
import java.io.BufferedReader
import java.net.HttpURLConnection
import java.net.URI
import java.net.URL

/** Minimal hub client over HttpURLConnection (no third-party HTTP deps). */
object HubClient {

    data class Pairing(val deviceId: String, val token: String)

    fun normalizeHubUrl(input: String): String {
        val raw = input.trim()
        if (raw.isEmpty()) throw IllegalArgumentException("Hub URL is required")
        val candidate = if (raw.contains("://")) raw else "https://$raw"
        val uri = try {
            URI(candidate)
        } catch (_: Exception) {
            throw IllegalArgumentException("Enter a valid Hub URL")
        }
        val scheme = uri.scheme?.lowercase()
        if (scheme != "http" && scheme != "https") {
            throw IllegalArgumentException("Hub URL must use http:// or https://")
        }
        if (uri.host.isNullOrBlank()) throw IllegalArgumentException("Hub URL must include a host name or IP address")
        if (uri.rawUserInfo != null) throw IllegalArgumentException("Do not put a username or password in the Hub URL")
        if (uri.rawQuery != null || uri.rawFragment != null) {
            throw IllegalArgumentException("Hub URL cannot include a query string or fragment")
        }
        val path = (uri.rawPath ?: "").trimEnd('/')
        if (path.isNotEmpty()) throw IllegalArgumentException("Hub URL should not include /api or another path")
        return URI(scheme, null, uri.host, uri.port, "", null, null).toASCIIString().trimEnd('/')
    }

    /** POST /api/pair — exchange the pairing secret for a per-device token. */
    fun pair(hubUrl: String, secret: String, name: String): Pairing {
        val baseUrl = normalizeHubUrl(hubUrl)
        val body = JSONObject()
            .put("pairingSecret", secret)
            .put("name", name)
            .put("platform", "android")
        val resp = postJson("$baseUrl/api/pair", null, body.toString())
        val json = JSONObject(resp)
        val token = json.optString("token", "")
        if (token.isEmpty()) throw RuntimeException("hub did not return a token")
        return Pairing(json.optString("deviceId", ""), token)
    }

    /** POST /api/events — upload a batch of activity events (device-token auth). */
    fun postEvents(hubUrl: String, token: String, deviceId: String, events: List<Event>) {
        val arr = JSONArray()
        for (e in events) arr.put(e.toJson())
        val body = JSONObject()
            .put("deviceId", deviceId)
            .put("events", arr)
        postJson("${normalizeHubUrl(hubUrl)}/api/events", token, body.toString())
    }

    private fun postJson(urlStr: String, bearer: String?, body: String): String {
        val con = URL(urlStr).openConnection() as HttpURLConnection
        try {
            con.requestMethod = "POST"
            con.connectTimeout = 15000
            con.readTimeout = 15000
            con.doOutput = true
            con.setRequestProperty("Content-Type", "application/json")
            if (bearer != null) con.setRequestProperty("Authorization", "Bearer $bearer")
            con.outputStream.use { it.write(body.toByteArray(Charsets.UTF_8)) }

            val code = con.responseCode
            val stream = if (code in 200..299) con.inputStream else con.errorStream
            val text = stream?.bufferedReader()?.use(BufferedReader::readText) ?: ""
            if (code !in 200..299) throw RuntimeException("HTTP $code: $text")
            return text
        } finally {
            con.disconnect()
        }
    }
}
