package com.tempo.mobile

import android.annotation.SuppressLint
import android.content.Intent
import android.graphics.Bitmap
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.view.ViewGroup
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
 *                   with the pairing secret injected as `tempo_web_token` so it just works.
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
            val url = hubUrl.text.toString().trim().trimEnd('/')
            val sec = secret.text.toString().trim()
            if (url.isEmpty() || sec.isEmpty()) {
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
        val secret = prefs.secret()
        w.webViewClient = object : WebViewClient() {
            override fun onPageStarted(view: WebView?, url: String?, favicon: Bitmap?) {
                injectToken(view, secret)
            }
            override fun onPageFinished(view: WebView?, url: String?) {
                injectToken(view, secret)
            }
        }
        // weight 1 = the WebView fills the rest below the status strip.
        root.addView(w, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))

        setContentView(root)
        w.loadUrl(prefs.hubUrl())
        startStatusTicker()
    }

    /** Seed the dashboard's web token so it authenticates without a prompt. */
    private fun injectToken(view: WebView?, secret: String) {
        if (view == null || secret.isEmpty()) return
        val esc = secret.replace("\\", "\\\\").replace("'", "\\'")
        view.evaluateJavascript(
            "try{localStorage.setItem('tempo_web_token','$esc');}catch(e){}",
            null,
        )
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
