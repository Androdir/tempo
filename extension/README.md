# Tempo local browser tracker (Firefox, Chrome, and Edge)

Tempo's WebExtension sends the active tab to the Tempo desktop app through its
loopback-only endpoint. It works in Firefox 142+, Chrome, Edge, and other current
Chromium browsers from the same source folder.

It records the domain, URL, page title, timestamp, duration, idle state, and a
local content-type guess. Readable page text is sent only when page capture is
enabled in Tempo and the current domain's policy allows it. Form fields,
password inputs, hidden text, and blocked sensitive domains are never read.

## Before installing

1. Start Tempo with `npm run app` and leave it running.
2. In Tempo, open **Settings → Setup Guide → Add browser tracking**.
3. Keep the endpoint and token ready. The default endpoint is
   `http://127.0.0.1:48710` and is reachable only from this computer.

## Install temporarily in Firefox

Use this path while developing or using the extension locally:

1. Update to Firefox 142 or newer.
2. Open `about:debugging#/runtime/this-firefox`.
3. Select **Load Temporary Add-on**.
4. Choose `extension/manifest.json` from this repository.
5. Open the Tempo toolbar button, choose **Open settings**, paste the endpoint
   and token, leave tracking enabled, then select **Save** and
   **Test connection**.

A temporary Firefox add-on is removed when Firefox restarts. Its extension
storage can persist, so reload it from the same manifest and verify its status
after restarting.

## Install permanently in Firefox

Firefox Stable permanently installs only Mozilla-signed extensions. To create a
signed XPI for personal self-distribution:

1. Validate and package the source with Mozilla's `web-ext` tool:

   ```bash
   npx web-ext lint --source-dir extension
   npx web-ext build --source-dir extension --artifacts-dir extension/web-ext-artifacts --overwrite-dest
   ```

2. Sign in to the [Firefox Add-on Developer Hub](https://addons.mozilla.org/developers/).
3. Submit a new add-on and choose **On your own** for self-distribution.
4. Upload the generated ZIP. After Mozilla signs it, download the XPI.
5. Open the signed XPI in Firefox and approve the requested permissions.

The manifest already includes the stable Firefox extension ID and Firefox's
required data-collection declaration. Tempo declares browsing activity, search
terms, page content, and the local authentication token because Firefox counts
sending them to the desktop app as transmission outside the extension—even
though the destination is your own computer, not a cloud service.

## Install in Chrome or Edge

1. Open `chrome://extensions` in Chrome or `edge://extensions` in Edge.
2. Turn on **Developer mode**.
3. Select **Load unpacked** and choose this `extension` folder.
4. Open the Tempo toolbar button → **Open settings**.
5. Paste the endpoint and token, select **Save**, then **Test connection**.

## Status badge

- No badge: samples are reaching Tempo.
- `!`: Tempo is not reachable or the token is missing/incorrect.
- `॥`: tracking is paused from the popup.

Samples are sent about every 10 seconds while the tab is visible and the browser
window has operating-system focus.

## How page capture is decided

```text
capture text  =  Tempo's "Capture page content" setting is on
              + domain policy is "Readable text"
              + domain is not blocked as sensitive
              + the computer is not idle
```

Otherwise, Tempo receives metadata only. For a blocked domain, the URL is
reduced to its origin.

## Why the permissions are needed

- `tabs`: read the active tab's URL and title.
- `idle`: exclude time while the computer is idle.
- `storage`: remember the local endpoint, token, and pause state.
- Access to HTTP/HTTPS pages: run the passive focus timer and, only when Tempo's
  policy requests it, extract allowed readable text.
- Access to `127.0.0.1` and `localhost`: talk to the Tempo desktop app.

The extension does not request browsing history, cookies, downloads, clipboard,
camera, microphone, or remote internet host access.

## Troubleshooting

### Test connection fails

- Keep the Tempo desktop app running.
- Re-copy both values from **Settings → Setup Guide**.
- Keep the endpoint on `127.0.0.1` or `localhost`; the manifest does not allow a
  remote endpoint.
- In Firefox, select **Reload** for Tempo on `about:debugging`, then reopen its
  options page.
- In Chrome/Edge, select the extension's reload button on the extensions page.

### No sites appear

- Confirm **Tracking enabled** is checked in the popup.
- Keep a normal HTTP/HTTPS page focused for at least 20 seconds.
- Browser-internal pages such as `about:`, `chrome:`, and `edge:` cannot be
  tracked by extensions.
- Check **Activity → Browser** in Tempo.

## Developer validation

Run the repository's dependency-free cross-browser checks:

```bash
npm run extension:check
```

For Firefox's full manifest and policy lint, also run:

```bash
npx web-ext lint --source-dir extension
```

## Files

| File | Role |
| --- | --- |
| `manifest.json` | Shared Manifest V3 metadata and Firefox consent declaration |
| `src/background.js` | Chromium service worker / Firefox event-page module |
| `src/content.js` | Focus-gated timer and privacy-safe text extraction |
| `src/summarize.js` | Local summary, keywords, and content-type heuristics |
| `options.html` / `src/options.js` | Endpoint and token setup |
| `popup.html` / `src/popup.js` | Connection status and pause control |
