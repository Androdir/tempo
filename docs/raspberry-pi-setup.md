# Run Tempo Hub on a Raspberry Pi over Tailscale

This is the recommended private setup for a Raspberry Pi 4, 400, 5, or 500
running a 64-bit Raspberry Pi OS. The Hub stays bound to the Pi itself; Tailscale
Serve gives it one private HTTPS address for your computer and Android phone.

```text
Windows Tempo tracker ─┐
Android Tempo tracker ─┼─ Tailscale ─ https://tempo-hub.<tailnet>.ts.net
Browser dashboard ─────┘                         │
                                      Raspberry Pi + Docker
                                      Tempo Hub + SQLite
```

Nothing needs to be port-forwarded on your router and this guide does not use
Tailscale Funnel. The Hub is available only to devices permitted by your
tailnet.

## What you need

- Raspberry Pi 4/400/5/500 with a 64-bit CPU and power supply
- Raspberry Pi OS Lite **64-bit** on a microSD card or SSD
- Ethernet if possible, or reliable Wi-Fi
- A Tailscale account
- The Tempo source repository on the Pi

Docker's current Raspberry Pi guidance recommends the Debian `arm64` packages
for a 64-bit OS. Do not use a 32-bit image for this setup.

## 1. Prepare the Pi

Use [Raspberry Pi Imager](https://www.raspberrypi.com/software/) to install
**Raspberry Pi OS Lite (64-bit)**. In the Imager customisation screen:

- set the hostname to `tempo-hub`;
- create your username and password;
- configure Wi-Fi if you are not using Ethernet;
- enable SSH;
- set your locale and timezone.

Boot the Pi, wait about two minutes, then connect from your computer:

```bash
ssh <your-user>@tempo-hub.local
```

If the `.local` name does not resolve, use the Pi's LAN IP from your router.

## 2. Install Docker and Tailscale

Install Docker Engine and the Compose plugin using Docker's instructions for
[Debian arm64](https://docs.docker.com/engine/install/debian/). The convenience
installer also works for a personal Pi:

```bash
curl -fsSL https://get.docker.com | sh
sudo usermod -aG docker "$USER"
```

Log out and reconnect after changing the Docker group:

```bash
exit
ssh <your-user>@tempo-hub.local
```

Install and connect Tailscale:

```bash
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up --hostname=tempo-hub
```

Open the authentication link it prints. Install Tailscale on the Windows
computer and Android phone as well, using the same tailnet.

Verify both services before continuing:

```bash
docker version
docker compose version
tailscale status
```

## 3. Start Tempo Hub

Get the code and run the Pi helper:

```bash
git clone https://github.com/Androdir/tempo.git
cd tempo
bash scripts/setup-pi-hub.sh
```

The helper:

1. verifies that the Pi is running a 64-bit ARM OS;
2. creates `.env` with a strong pairing secret if needed;
3. validates the Compose configuration;
4. builds the React dashboard and ARM64 Rust Hub image;
5. waits for `/api/health` to pass;
6. publishes the loopback-only Hub through persistent Tailscale Serve HTTPS.

The first native build can take 10–40 minutes depending on the Pi. Later builds
reuse Docker layers.

At the end, Tailscale prints an address similar to:

```text
https://tempo-hub.example-tailnet.ts.net
```

Copy the **exact HTTPS URL it prints**. Do not add `:7700`; Tailscale terminates
HTTPS on the private URL and proxies it to the loopback Hub.

View the pairing secret only when you are ready to pair a device:

```bash
grep '^TEMPO_PAIRING_SECRET=' .env
```

Keep `.env` private. The helper gives it owner-only file permissions and Docker
never copies it into the image.

## 4. Verify the Pi before pairing

Run all four checks on the Pi:

```bash
docker compose ps
curl --fail http://127.0.0.1:7700/api/health
tailscale serve status
curl --fail https://tempo-hub.example-tailnet.ts.net/api/health
```

The health response should contain:

```json
{"app":"tempo-hub","ok":true}
```

From the Windows computer and phone, with Tailscale connected, open the same
HTTPS URL in a browser. Tempo should load and ask for the Hub secret.

## 5. Connect the Windows desktop tracker

In Tempo:

1. Open **Settings → Connections**.
2. Under **Tempo Hub sync**, paste the exact Tailscale HTTPS URL.
3. Paste the pairing secret from the Pi.
4. Select **Pair this device**.
5. Select **Import history** if you want to copy existing local events to the Hub.
6. Select **Sync projects & rules now** to immediately replace old Hub examples or
   stale matching configuration with the projects, categories, and rules from this desktop.

The desktop still records locally. If the Pi or Tailscale is temporarily
offline, events remain queued and upload later.

A successful setup shows **Connected**, a recent sync time, and zero queued
events after the next sync cycle.

Project configuration currently has one source of truth: the Windows desktop.
Changes made in the Hub dashboard do not copy back to Windows and may be replaced
the next time the desktop sends its configuration.

## 6. Connect the Android tracker

Build and install the app from the `android` folder as described in
[`android/README.md`](../android/README.md). On the phone:

1. Install Tailscale, sign into the same tailnet, and make sure it is connected.
2. Open Tempo and grant Android **Usage access**.
3. Paste the same Tailscale HTTPS URL and pairing secret.
4. Select **Pair & start tracking**.

Tempo normalises and validates the URL before saving it. The dashboard receives
the secret through a one-time URL fragment, removes it immediately, and never
injects it into pages outside the Hub origin.

Android uploads on its WorkManager schedule and also triggers a sync directly
after pairing. The top status strip shows tracking permission, app pickups, and
last sync time.

## 7. Confirm combined data

Use the computer and phone for a few minutes, then open the Hub dashboard. Check:

- **Today** shows activity;
- the device breakdown includes both Windows and Android;
- the desktop connection reports no growing queue;
- the Android status strip reports a recent sync.

The Hub deduplicates repeated uploads by device and event ID, so reconnecting or
importing history does not double-count the same event from one device.

## Updates and day-to-day commands

```bash
cd tempo
docker compose logs -f tempo-hub
docker compose restart tempo-hub
tailscale serve status
```

Update without deleting the database:

```bash
cd tempo
git pull
bash scripts/setup-pi-hub.sh
```

The SQLite database lives in the named `tempo-data` Docker volume and survives
container rebuilds.

## Back up the Hub

Stop the Hub briefly so SQLite and its WAL are consistent, copy the data folder,
then restart:

```bash
cd tempo
docker compose stop tempo-hub
mkdir -p "$HOME/tempo-backups"
docker compose cp tempo-hub:/data "$HOME/tempo-backups/data-$(date +%F-%H%M)"
docker compose start tempo-hub
```

Copy that backup off the Pi periodically.

## Direct LAN access instead of Tailscale Serve

The default Compose configuration publishes port 7700 only on `127.0.0.1`, so
other LAN devices cannot bypass Tailscale. If you deliberately want LAN access,
set this in `.env` and recreate the container:

```bash
TEMPO_HOST_BIND=0.0.0.0
```

```bash
docker compose up -d
```

Then use `http://<pi-lan-ip>:7700`. This exposes the port on every Pi network
interface, and Docker-published ports can bypass simple UFW rules. Tailscale
Serve is the recommended default.

## Troubleshooting

| Symptom | What to check |
| --- | --- |
| Helper says the Pi is not ARM64 | Reinstall Raspberry Pi OS Lite **64-bit**. |
| Docker permission denied | Log out and reconnect after `usermod -aG docker`. |
| Build is killed | Add swap on a low-memory Pi, then rerun the helper. |
| Local health check fails | Run `docker compose logs --tail 100 tempo-hub`. |
| Tailscale URL does not open | Confirm Tailscale is connected on both devices and run `tailscale serve status`. |
| Desktop pairing fails | Paste the exact HTTPS URL without a trailing `/api` path and re-copy the secret. |
| Phone pairing fails | Open the HTTPS URL in the phone browser first to confirm Tailscale reachability. |
| Hub works locally but not remotely | Confirm `.env` keeps `TEMPO_HOST_BIND=127.0.0.1` and that Tailscale Serve targets `http://127.0.0.1:7700`. |
| Temporary loss of connection | Leave both clients running; their local queues retry automatically. |

For a low-memory Pi, create a 2 GB swap file before building:

```bash
sudo dphys-swapfile swapoff
sudo sed -i 's/^CONF_SWAPSIZE=.*/CONF_SWAPSIZE=2048/' /etc/dphys-swapfile
sudo dphys-swapfile setup
sudo dphys-swapfile swapon
```