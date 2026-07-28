# Tempo setup and everyday-use guide

This guide gets Tempo from a fresh checkout to useful daily data with the least
possible setup. Start with the essentials and add optional tracking only when it
answers a question you care about.

The same checklist is built into the app under **Settings → Setup Guide**. The
in-app version checks your actual setup and links directly to the right screen.

## The fastest useful setup

You need:

- Windows 10 or 11
- Node.js 18 or newer
- Rust stable, installed with [rustup](https://rustup.rs/)
- Visual Studio Build Tools with **Desktop development with C++**

From the project folder:

```powershell
npm install
npm run app
```

The first start compiles the Rust app and can take a few minutes. Later starts
are much faster. Keep the desktop window open or minimized while you work.

If you run `npm run dev` instead, you get a browser preview with sample data. It
is useful for exploring the interface, but it cannot track your computer.

## Your first five minutes

### 1. Confirm tracking is running

Open **Today**. The status at the top should say that Tempo is tracking every
10 seconds. Use a few normal apps for about a minute.

Then open **Activity → Timeline**. You should see blocks for the apps you used.
Tempo records the foreground app, window title, duration, and whether the
computer was idle. It does not record keystrokes, audio, clipboard contents, or
mouse coordinates.

### 2. Add one goal

Open **Plan → Daily Goals** and add the one outcome that would make today
successful. One to three goals is usually enough.

Goals give the score and end-of-day review useful context. They are not required
for tracking.

### 3. Classify the apps that matter

Open **Projects → Categories** after Tempo has observed some activity. Classify
your main apps as productive, study, business, neutral, distracting, recovery,
or excluded.

You normally do this once per app. Tempo saves the rule and applies it to future
activity. If something is wrong later, correct it from **Activity → Activity
Log**.

At this point, the core setup is complete. Browser tracking, screen context,
output folders, local AI, and Hub sync are optional.

## Optional: add browser tracking

The shared extension supports Firefox 142+, Chrome, Edge, and current Chromium
browsers. It adds the focused tab's domain and page title while the browser is
the active desktop window.

1. Start the Tempo desktop app.
2. Open **Settings → Setup Guide** and expand **Add browser tracking**.
3. Copy the endpoint and token shown there.
4. Install the extension:
   - **Firefox:** open `about:debugging#/runtime/this-firefox`, choose
     **Load Temporary Add-on**, and select `extension/manifest.json`.
   - **Chrome/Edge:** open the browser's extensions page, enable Developer mode,
     choose **Load unpacked**, and select the `extension` folder.
5. Open Tempo's extension Options page.
6. Paste the endpoint and token, save, and choose **Test connection**.
7. Browse normally for about 20 seconds, then check **Activity → Browser**.

Firefox temporary add-ons disappear after Firefox restarts. See
[`extension/README.md`](../extension/README.md) for permanent signed-XPI steps
and the exact permission explanation.

Page-text capture is separate and off by default. Review it under
**Settings → Browser & content** before enabling it.

## Optional: count saved work

Open **Activity → Outputs** and add a folder where you regularly save completed
work, such as exports, documents, or builds.

Tempo records file metadata as evidence that something was produced. It does
not read the file contents. Choose specific output folders instead of broad
locations such as your whole user folder.

## Optional: improve classification

### Screen context

Open **Settings → Tracking** to review local OCR. It can help when app and
window titles are ambiguous, but it is off by default and is not needed for
basic tracking.

### Local AI with Ollama

Open **Settings → Connections** to connect an Ollama server. Local AI can add
richer classification and reviews, but all core tracking, categories, scores,
and rule-based reviews work without it.

### Tempo Hub

Hub sync is for people who want one dashboard across multiple devices. It is
not required for one Windows computer. If you do want it, the
[Raspberry Pi setup](raspberry-pi-setup.md) creates the Hub, generates its
secret, and exposes it to your own Tailscale network through a private HTTPS
URL. Paste that exact URL into Tempo on each device; do not open router ports.

## A low-friction daily routine

### Morning — 30 seconds

- Open **Plan → Daily Goals**.
- Set one main mission and, at most, two secondary goals.
- Start Focus Mode only if you need a timed block.

### During the day — automatic

- Leave Tempo running.
- Work normally.
- Log offline activities with the quick check-ins on **Today**.
- Correct a new or misclassified app once; Tempo reuses the rule afterward.

### Evening — two minutes

- Open **Insights → Daily Review**.
- Check whether the time breakdown matches reality.
- Correct obvious mistakes in **Activity → Activity Log**.
- Choose one adjustment for tomorrow.

### Weekly — five minutes

Use **Insights → Weekly Review** to look for repeated patterns. Treat the score
as a prompt for reflection, not as an objective measure of a good day.

## What Tempo can and cannot know

Tempo can automatically track digital foreground time, idle time, optional
browser context, and optional saved-file events.

It cannot reliably know:

- whether a work-looking app was used productively;
- what you did away from the computer;
- why you switched tasks;
- whether time spent was valuable to you.

Goals, categories, corrections, and check-ins provide that missing context.
That is why a small amount of deliberate input is more useful than enabling
every optional feature.

## Troubleshooting

### No activity appears

- Make sure you launched `npm run app`, not `npm run dev`.
- Leave the app running and use another window for at least 20 seconds.
- Return to **Today** or **Activity → Timeline**.
- Check **Settings → Tracking** for the idle threshold.

### Browser extension cannot connect

- Keep the Tempo desktop app running.
- Copy the current endpoint and token again from **Settings → Setup Guide**.
- Confirm the endpoint uses `127.0.0.1`, not another computer's address.
- Save the extension options before testing.
- Reload the unpacked extension after changing its source files.

### Everything looks neutral or incorrect

Open **Projects → Categories** and classify the apps you use most. Rule-based
classification becomes useful quickly once those few rules exist.

### The interface shows sample data

You are in browser preview mode. Close it and run:

```powershell
npm run app
```

### You want to start over

Open **Settings → Data & privacy**. Export anything you want to keep before
using reset or deletion controls.
