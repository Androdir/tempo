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
are much faster. Closing the window sends Tempo to the system tray, where tracking continues.
Installed release builds enable **Settings → Tracking → Launch at Windows login** once by default;
they start hidden in the tray, and you can turn that off at any time.

If you run `npm run dev` instead, you get a browser preview with sample data. It
is useful for exploring the interface, but it cannot track your computer.

## Your first five minutes

### 1. Confirm tracking is running

Open **Today**. The tracking-health strip should show a recent desktop sample and a verified database. If it turns amber, follow the specific stale tracker, browser extension, sync queue, or database warning. Use a few normal apps for about a minute.

Then open **Activity → Timeline**. The default **Overview** shows meaningful runs
of work and absorbs only a brief (90 seconds or less) A → B → A switch when the
same activity immediately resumes. Choose **Exact** whenever you want every
captured switch, full classifier details, or inline corrections. Exact data is
always retained and the totals remain exact.

Tempo records the foreground app, window title, duration, and whether the
computer was idle. It does not record keystrokes, audio, clipboard contents, or
mouse coordinates.

### 2. Add one goal

Open **Plan → Daily Goals** and add the one outcome that would make today
successful. One to three goals is usually enough. A target is optional: choose **Time** for a timed block, or **Output / count** for a result such as `1 video`, `3 clips`, or `1 proposal`. If a matching watched folder sees likely output evidence, Tempo offers a **Confirm complete** action; it never auto-completes the mission because an exported file does not prove it was published.

Goals give the score and end-of-day review useful context. They are not required
for tracking.

**Priority** controls the order of missions and which unfinished mission Tempo treats as the main one: High is primary, Medium is normal, and Low is later. It does not multiply minutes or change Daily Score weights.

### 3. Classify the apps that matter

Open **Projects → Categories** after Tempo has observed some activity. The
default **All** view puts desktop apps and browser websites in one place; use the
Apps or Websites filters only when the list gets long. Classify the activity
that matters as productive, study, business, neutral, distracting, recovery,
or excluded.

Tempo also identifies common activity types before applying your policy. The built-in Game policy marks recognised games as distracting, including common Steam, Epic, Riot, Ubisoft, GOG, Xbox, and Battle.net installations. You can change the policy for all recognised games, chats, social feeds, creator tools, videos, or research without teaching every individual app. A specific app/site correction or strong project match still wins.

You normally do this once per app or website. Tempo saves the rule and applies
it to future activity. Websites appear after the browser extension has sent its
first samples. If something is wrong later, open **Activity → Classifications** and click the app or website. The side drawer explains the category source, confidence, project threshold, and exact match evidence before you correct it; the newest correction can be undone. For a false project association, use **Never match this app/website to this project**, or edit the project's exclusion list. **Projects → Projects** also includes a matcher tester so you can try a real app, title, and content example before saving.

### 4. Understand Quick check-ins

A Quick check-in means **“this happened today.”** Use one for things Tempo cannot
time reliably, such as reading a physical Bible, going to the gym, or publishing
a video. It does not add tracked minutes and it does not complete a Daily Goal.

After you tap one on **Plan → Daily Goals**, it appears under **Activity →
Timeline → Logged today**. It affects the Daily Score or a streak only if you
create a rule that uses that check-in; reviews and weekly summaries can also
include it.

**Insights → Daily Score** begins with only two Tempo starter rules: reward the main goal, and penalize leaving it unfinished. Open **Full breakdown → Edit rules** to change or remove them, or add rules for your check-ins, categories, apps/sites, and detected outputs. Tempo does not assume that studying, coding, gym, Instagram, or YouTube belongs in your score.

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
7. Browse normally for about 20 seconds, then check **Activity → Websites**.

Firefox temporary add-ons disappear after Firefox restarts. See
[`extension/README.md`](../extension/README.md) for permanent signed-XPI steps
and the exact permission explanation.

Page-text capture is separate and off by default. Review it under
**Settings → Browser & content** before enabling it. Keeping the raw text excerpt
is optional and normally unnecessary: it exists only so you can inspect the
exact visible text that caused a bad classification. Leave it off (or enable
“Delete raw text after classification”) when the summary and keywords are
enough.

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
and rule-based reviews work without it. Tempo validates AI output against deterministic evidence:
goals alone cannot assign a project, and an uncertain app/title-only guess cannot claim near-certainty.

The **Local AI check** checkbox on an app or website rule asks Ollama to double-check even when the deterministic rule is already confident. It is optional and usually should stay off; enable it only for an app or site whose context genuinely changes its meaning.

### Tempo Hub

Hub sync is for people who want one dashboard across multiple devices. It is
not required for one Windows computer. If you do want it, the
[Raspberry Pi setup](raspberry-pi-setup.md) creates the Hub, generates its
secret, and exposes it to your own Tailscale network through a private HTTPS
URL. Paste that exact URL into Tempo on each device; do not open router ports.

After pairing the Windows app, select **Sync projects & rules now**. This sends
the desktop's projects, categories, app and website rules, and classification
policies immediately. **Import history** also resets configuration sync, but is
intended for copying historical activity. Project configuration flows from the
Windows desktop to the Hub; edits made only in the mobile/Hub dashboard do not copy back.

## Pause and protect private context

Use **Settings → Tracking → Pause tracking** (or the tray menu) for private time. Add sensitive apps such as a password manager to the title-exclusion list when you still want accurate time totals but never want Tempo to store their window titles. Browser samples always discard query strings and fragments; configured sensitive domains retain only their origin. These controls are local to each device.

## Export for an accountability review

Open **Settings → Data & privacy → Accountability export** when you want a second opinion on where your time is going.

1. Pick **Last 7 days**, **14 days**, **30 days**, **90 days**, or enter exact dates.
2. Leave app and website names enabled if you want actionable feedback. Turn them off to replace them with labels such as `Desktop app #1` and `Website #1`.
3. Leave titles, daily notes, and raw captured text off unless their extra context is genuinely needed. These can contain private chats, client names, or personal information.
4. Choose **Copy for ChatGPT** for the fastest workflow, or save/download the `.md` file and inspect it before sharing.
5. Paste or attach the report to ChatGPT. The first block already asks for a brutally honest but practical assessment and warns the AI not to invent conclusions from incomplete tracking.

The report contains summaries rather than a dump of every ten-second sample: active/productive/neutral/distracting time, top leaks, productive sources, projects, daily trends, meaningful switches, goals, check-ins, outputs, and Tempo's own flags. URL paths, local file paths, pairing secrets, ingest tokens, and database identifiers are never exported.

If most time is neutral or the range contains very little tracked activity, correct your classifications or collect more data before treating the analysis as reliable.
## A low-friction daily routine

### Morning — 30 seconds

- Open **Plan → Daily Goals**.
- Set one main mission and, at most, two secondary goals.
- Start Focus Mode only if you need a timed block.

### During the day — automatic

- Leave Tempo running.
- Work normally.
- Log offline or untimed activities with Quick check-ins on **Plan → Daily Goals**; they record an occurrence, not minutes.
- Correct a new or misclassified app once; Tempo reuses the rule afterward.

### Evening — two minutes

- Open **Insights → Daily Review**.
- Check whether the time breakdown matches reality.
- Correct obvious mistakes in **Activity → Classifications**.
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

Open **Settings → Data & privacy** and choose **Back up now** before using reset or deletion controls. Tempo verifies the backup, and restoring one first preserves the current database as another manual backup.
