package com.tempo.mobile

import org.json.JSONArray
import org.json.JSONObject
import java.io.BufferedReader
import java.net.HttpURLConnection
import java.net.URL

/** Minimal hub client over HttpURLConnection (no third-party HTTP deps). */
object HubClient {

    data class Pairing(val deviceId: String, val token: String)

    /** POST /api/pair — exchange the pairing secret for a per-device token. */
    fun pair(hubUrl: String, secret: String, name: String): Pairing {
        val body = JSONObject()
            .put("pairingSecret", secret)
            .put("name", name)
            .put("platform", "android")
        val resp = postJson("$hubUrl/api/pair", null, body.toString())
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
        postJson("$hubUrl/api/events", token, body.toString())
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
