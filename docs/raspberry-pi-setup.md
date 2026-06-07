# Tempo Hub on a Raspberry Pi — beginner, headless setup

A complete, copy-paste walkthrough to run the **Tempo Hub** on a Raspberry Pi **with no
monitor or keyboard attached** (fully headless), and connect your desktop + phone to it over a
private network. Written for first-timers — if you've never used SSH, you're in the right place.

Works on **Pi 4 / 400 / 5 / 500** (all arm64). The Pi never needs a screen — a hub is just a
background server.

**What you're building:** three pieces talking to one shared database.

```
 Desktop app (tracker) ─┐
 Phone app  (tracker) ──┼──►  Raspberry Pi  =  Tempo Hub (shared DB + dashboard)
 Any browser (viewer) ──┘        reached privately over Tailscale
```

**You'll need:** the Pi + its microSD card, a microSD→SD adapter (or a USB card reader), a
computer to flash the card, and ideally an ethernet cable.

---

## Phase 1 — Flash the SD card

1. Install **Raspberry Pi Imager**: <https://www.raspberrypi.com/software/>
2. Put the microSD in the adapter/reader and into your computer. If Windows says *"format this
   disk?"* → **Cancel** (you don't need to format — Imager does it).
3. In Imager:
   - **Choose Device** → your model (Pi 4 for a 400; Pi 5 for a 500).
   - **Choose OS** → *Raspberry Pi OS (other)* → **Raspberry Pi OS Lite (64-bit)** (no desktop —
     perfect for a server).
   - **Choose Storage** → your SD card.
4. **Next** → *"apply OS customisation?"* → **Edit Settings**. This is what makes headless work:
   - **Hostname:** `tempo`
   - **Username:** `pi` · **Password:** something you'll remember (write it down).
   - **Configure wireless LAN:** your WiFi name + password + country — *or skip it if you'll plug
     in an ethernet cable (recommended).*
   - **Locale:** your timezone.
   - **Services** tab → ✅ **Enable SSH** → "Use password authentication".
   - *(Optional)* Raspberry Pi Connect — harmless extra, gives browser-based access as a backup;
     not required since we use SSH + Tailscale.
   - **Save** → **Yes** → write (~5–10 min) → eject.

---

## Phase 2 — Boot it (no screen needed)

1. Put the microSD into the Pi (slot is on the back/underside edge).
2. **Plug an ethernet cable** from the Pi to your router (most reliable), or rely on the WiFi you
   configured.
3. Power on. Wait **~2 minutes** for it to boot and join the network.

---

## Phase 3 — SSH into the Pi

SSH = "control the Pi by typing commands on it from your own computer."

1. Open **Windows Terminal** / PowerShell (or Terminal on macOS).
2. Run:
   ```bash
   ssh pi@tempo.local
   ```
3. First time it asks to confirm the fingerprint → type **`yes`** → Enter.
4. Type your password. **Nothing appears on screen while you type a password — that's normal.**
   Press Enter.
5. You see `pi@tempo:~ $` → **you're in.** Every command below runs here, on the Pi.

> **If `tempo.local` doesn't resolve:** open your router admin page (often `http://192.168.1.1`),
> find the device named `tempo`, note its IP, and use `ssh pi@192.168.1.xx` instead.

---

## Phase 4 — Install the hub

**4a. Add swap** (do this on a 4 GB Pi like the 400 so the build doesn't get "Killed" — harmless
on 8 GB models too):
```bash
sudo dphys-swapfile swapoff
sudo sed -i 's/^CONF_SWAPSIZE=.*/CONF_SWAPSIZE=2048/' /etc/dphys-swapfile
sudo dphys-swapfile setup && sudo dphys-swapfile swapon
```

**4b. Install Docker:**
```bash
curl -fsSL https://get.docker.com | sh
sudo usermod -aG docker $USER
```
Now **log out and back in** so Docker works without `sudo`: type `exit`, then `ssh pi@tempo.local`
again.

**4c. Get the code and start the hub:**
```bash
git clone https://github.com/Androdir/tempo.git
cd tempo
echo "TEMPO_PAIRING_SECRET=$(openssl rand -hex 24)" > .env
docker compose up -d --build
```
⏳ The first build compiles Rust on the Pi — **expect 20–40 min on a Pi 4, ~10–15 on a Pi 5.**

> If `git clone` asks for a username/password, your GitHub repo is **private**. Easiest fix: on
> GitHub → repo → **Settings → General → Change visibility → Public** (safe — no secrets are
> committed). Or use a personal access token in place of the password.

**4d. When it finishes — grab the secret and confirm it's running:**
```bash
cat .env                                  # copy the TEMPO_PAIRING_SECRET value
docker compose logs --tail 20 tempo-hub   # should say: listening on http://0.0.0.0:7700
```

---

## Phase 5 — Tailscale (private access from anywhere)

This puts the Pi + your devices on one private network — reachable from anywhere, invisible to the
public internet. Free for personal use.

**On the Pi:**
```bash
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up
```
It prints a **URL** → open it on your computer → log in. The Pi joins your network.

Then install the **Tailscale app** on your **desktop** and **phone**, signed into the **same
account**. Your hub's address is now:
```
http://tempo:7700
```

> **Check it works:** open `http://tempo:7700` in a browser on a device that has Tailscale on. It
> should prompt for the secret — paste it, and you'll see the dashboard.

---

## Phase 6 — Connect your desktop (the tracker)

In the **Tempo desktop app** → **Privacy & Settings → Sync**:
- Mode → **Connect to Tempo Hub**
- Hub URL: `http://tempo:7700`
- Pairing secret: *(from `cat .env`)*
- **Pair**, then **Import history** to backfill your existing data.

It keeps tracking locally and uploads in the background, buffering through any hub downtime.

---

## Phase 7 — Connect your phone (Android)

1. Build/install the Android app — open the `android/` folder in **Android Studio** and **Run ▶**
   on your phone (see [`../android/README.md`](../android/README.md)).
2. In the app: **Grant usage access** → enter `http://tempo:7700` + the secret → **Pair & start
   tracking**.

Now the hub dashboard (browser or phone) shows desktop + phone combined; the desktop app keeps its
own local view too.

---

## (Optional) LLM-written reviews on the dashboard

The Pi doesn't run an AI model. If you want the hub's Daily Review / Lock-In Plan to be
LLM-written (instead of the deterministic fallback), point the hub at an **Ollama you run on your
desktop**. On the desktop set `OLLAMA_HOST=0.0.0.0` and keep it awake, then add to the Pi's `.env`:
```bash
TEMPO_LLM_ENABLED=1
TEMPO_OLLAMA_URL=http://<desktop-tailscale-or-LAN-ip>:11434
TEMPO_OLLAMA_MODEL=llama3.1:8b
```
and `docker compose up -d` again. Only LAN/Tailscale addresses are accepted (cloud is refused).

---

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| `ssh: Could not resolve hostname tempo.local` | Use the Pi's IP from your router page instead. |
| SSH password "isn't working" | It is — the screen just shows nothing as you type. Type it blind, press Enter. |
| `docker: permission denied` | You skipped the log-out/in after `usermod -aG docker`. Run `exit`, SSH back in. |
| `docker compose` build ends with **Killed** | Out of memory — apply the swap step (4a), then rebuild. |
| `git clone` asks for a password | Repo is private — make it public, or use a token (see 4c). |
| Can't reach `tempo:7700` from a device | Make sure Tailscale is **on** and signed into the same account on that device. |

## Day-to-day

```bash
docker compose logs -f tempo-hub   # watch logs
docker compose restart tempo-hub   # restart
docker compose pull && docker compose up -d --build   # update after a git pull
```
Your data lives on the `tempo-data` Docker volume and survives rebuilds. Back it up by copying
`/var/lib/docker/volumes/` periodically, or `docker compose cp tempo-hub:/data ./backup`.
</content>
