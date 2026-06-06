# Tempo — Local Browser Tracker (Chrome extension)

Tracks the **active browser tab** while Chrome is the focused desktop window and
sends each sample to the local Tempo desktop app over a **loopback-only** HTTP
endpoint. Optionally captures *readable* page text — never passwords, form
fields, or hidden content, and never to any cloud.

## What it sends (per ~10s sample)

- domain, full URL, page title, timestamp, duration, idle/active
- detected content type (article / video / chat / docs-editor / social feed / search / other)
- **only when you turn capture on, and only on allowed domains:**
  a local `content_summary`, `detected_keywords`, and (if you opt in) a
  `raw_text_excerpt`

## Install (unpacked)

1. Start the desktop app (`npm run app` in the project root). It runs the local
   endpoint at `http://127.0.0.1:48710`.
2. In the app, open **Privacy & Settings → Browser extension** and copy the
   **endpoint** and **token**.
3. In Chrome: `chrome://extensions` → enable **Developer mode** → **Load
   unpacked** → select this `extension/` folder.
4. Click the Tempo toolbar icon → **Open settings**, paste the endpoint + token,
   tick **Tracking enabled**, then **Save** and **Test connection** (expect
   "Connected ✓").

The toolbar badge shows status: green = sending, `!` = app unreachable or no
token, `॥` = tracking paused.

## How capture is decided

```
capture text  ⇐  app "Capture page content" is ON
              AND domain rule = "Readable text"
              AND domain is not built-in blocked (bank/pay/email/health/pw-managers)
              AND you are not idle
```

Otherwise only domain/URL/title/duration are sent (for blocked domains the URL
is reduced to its origin). All capture policy lives in the **app** — the
extension just pulls it from `GET /config` and enforces it.

## Privacy notes

- Requests go **only** to the loopback endpoint you configure. No other network
  access; the extension has no remote host permissions.
- The content script is passive: it extracts text **only** when the background
  asks, and reads **visible** headings/paragraphs/lists/labels via a text-node
  walk. It never reads `<input>`/`<textarea>`/`contenteditable` values, skips
  hidden/nav/ad nodes and `<form>`s, dedupes repeated menus, and redacts
  card/SSN-like numbers as a safety net.
- The `tabs` + all-sites content script permissions are required to read the
  active tab's URL/title and (when allowed) its text. Tracking can be paused any
  time from the popup.

## Files

| File | Role |
| --- | --- |
| `manifest.json` | MV3 manifest (loopback host permission only) |
| `src/background.js` | Service worker: ticks → policy → POST `/ingest` |
| `src/content.js` | Focus-gated 10s tick + privacy-safe text extraction |
| `src/summarize.js` | Local summary/keyword heuristics + content-type detection |
| `options.html` / `src/options.js` | Endpoint + token setup, connection test |
| `popup.html` / `src/popup.js` | Status + pause toggle |
