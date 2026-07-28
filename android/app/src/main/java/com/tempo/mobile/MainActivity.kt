package com.tempo.mobile

import android.annotation.SuppressLint
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.view.ViewGroup
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity

/**
 * Two states in one Activity:
 *  - not paired  -> a small setup screen (grant Usage access, enter hub URL + secret).
 *  - paired      -> a WebView of the hub dashboard (the same web app the desktop serves),
 *                   with a one-time URL fragment that seeds `tempo_web_token` on the Hub origin.
 */
class MainActivity : AppCompatActivity() {

    private lateinit var prefs: Prefs
    private var web: WebView? = null
    private var statusView: TextView? = null
    private var statusBar: TextView? = null
    private val handler = Handler(Looper.getMainLooper())
    private var statusTick: Runnable? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        prefs = Prefs(this)
        if (prefs.isPaired()) showDashboard() else showSetup()
    }

    override fun onResume() {
        super.onResume()
        if (prefs.isPaired()) updateStatusBar() else refreshStatus()
    }

    // ----------------------------------------------------------------- setup

    private fun showSetup() {
        setContentView(R.layout.activity_setup)
        val hubUrl = findViewById<EditText>(R.id.hubUrl)
        val secret = findViewById<EditText>(R.id.secret)
        val grantBtn = findViewById<Button>(R.id.grantBtn)
        val pairBtn = findViewById<Button>(R.id.pairBtn)
        statusView = findViewById(R.id.status)

        if (prefs.hubUrl().isNotEmpty()) hubUrl.setText(prefs.hubUrl())

        grantBtn.setOnClickListener {
            startActivity(Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS))
        }

        pairBtn.setOnClickListener {
            val rawUrl = hubUrl.text.toString().trim()
            val sec = secret.text.toString().trim()
            if (rawUrl.isEmpty() || sec.isEmpty()) {
                setStatus("Enter the hub URL and pairing secret.")
                return@setOnClickListener
            }
            if (!UsageTracker.hasPermission(this)) {
                setStatus("Grant usage access first (step 1).")
                return@setOnClickListener
            }
            setStatus("Pairing…")
            Thread {
                try {
                    val url = HubClient.normalizeHubUrl(rawUrl)
                    val name = Build.MODEL ?: "Android phone"
                    val p = HubClient.pair(url, sec, name)
                    prefs.savePairing(url, p.token, p.deviceId, sec)
                    TempoApp.scheduleSync(this)
                    TempoApp.syncNow(this)
                    runOnUiThread { showDashboard() }
                } catch (e: Exception) {
                    runOnUiThread { setStatus("Pairing failed: ${e.message}") }
                }
            }.start()
        }

        refreshStatus()
    }

    private fun refreshStatus() {
        val ok = UsageTracker.hasPermission(this)
        setStatus(
            if (ok) "Usage access granted ✓  Enter your hub details and pair."
            else getString(R.string.status_idle),
        )
    }

    private fun setStatus(msg: String) {
        statusView?.text = msg
    }

    // ------------------------------------------------------------- dashboard

    @SuppressLint("SetJavaScriptEnabled")
    private fun showDashboard() {
        val root = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }

        // A thin status strip above the dashboard: tracking state + pickups + last sync.
        // Tap it to grant Usage access (if paused) or trigger a sync now.
        val bar = TextView(this).apply {
            textSize = 12f
            setPadding(dp(12), dp(8), dp(12), dp(8))
            setBackgroundColor(0xFF1A1730.toInt())
            setTextColor(0xFFEDEAF6.toInt())
            setOnClickListener {
                if (!UsageTracker.hasPermission(this@MainActivity)) {
                    startActivity(Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS))
                } else {
                    TempoApp.syncNow(this@MainActivity)
                }
            }
        }
        statusBar = bar
        root.addView(
            bar,
            LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT),
        )

        val w = WebView(this)
        web = w
        w.settings.javaScriptEnabled = true
        w.settings.domStorageEnabled = true
        w.settings.allowFileAccess = false
        w.settings.allowContentAccess = false
        w.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(view: WebView?, request: WebResourceRequest?): Boolean {
                val target = request?.url ?: return false
                if (sameHubOrigin(target)) return false
                runCatching { startActivity(Intent(Intent.ACTION_VIEW, target)) }
                return true
            }
        }
        // weight 1 = the WebView fills the rest below the status strip.
        root.addView(w, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))

        setContentView(root)
        val token = Uri.encode(prefs.secret())
        w.loadUrl("${prefs.hubUrl().trimEnd('/')}/#tempo_token=$token")
        startStatusTicker()
    }

    private fun sameHubOrigin(target: Uri): Boolean {
        val hub = Uri.parse(prefs.hubUrl())
        fun effectivePort(uri: Uri): Int = if (uri.port >= 0) uri.port else if (uri.scheme == "https") 443 else 80
        return target.scheme.equals(hub.scheme, ignoreCase = true) &&
            target.host.equals(hub.host, ignoreCase = true) &&
            effectivePort(target) == effectivePort(hub)
    }

    private fun startStatusTicker() {
        statusTick?.let { handler.removeCallbacks(it) }
        val tick = object : Runnable {
            override fun run() {
                updateStatusBar()
                handler.postDelayed(this, 30_000L)
            }
        }
        statusTick = tick
        handler.post(tick)
    }

    private fun updateStatusBar() {
        val bar = statusBar ?: return
        if (!UsageTracker.hasPermission(this)) {
            bar.text = "⚠ Tracking paused — tap to grant Usage access"
            return
        }
        val pickups = UsageTracker.pickupsToday(this)
        bar.text = "✅ Tracking on · $pickups app opens today · synced ${syncAgo(prefs.lastSync())}"
    }

    private fun syncAgo(ms: Long): String {
        if (ms <= 0L) return "not yet"
        val mins = (System.currentTimeMillis() - ms) / 60000L
        return when {
            mins < 1L -> "just now"
            mins < 60L -> "${mins}m ago"
            else -> "${mins / 60L}h ago"
        }
    }

    private fun dp(v: Int): Int = (v * resources.displayMetrics.density).toInt()

    override fun onDestroy() {
        statusTick?.let { handler.removeCallbacks(it) }
        super.onDestroy()
    }

    @Suppress("DEPRECATION")
    override fun onBackPressed() {
        val w = web
        if (w != null && w.canGoBack()) w.goBack() else super.onBackPressed()
    }
}
