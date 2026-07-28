#!/usr/bin/env bash
set -Eeuo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing required command: $1" >&2
    exit 1
  }
}

for command in docker tailscale curl openssl sudo; do
  need "$command"
done

architecture="$(uname -m)"
case "$architecture" in
  aarch64|arm64) ;;
  *)
    echo "This helper is for a 64-bit Raspberry Pi; detected: $architecture" >&2
    echo "Install Raspberry Pi OS Lite (64-bit), then run it again." >&2
    exit 1
    ;;
esac

docker info >/dev/null
docker compose version >/dev/null
tailscale status >/dev/null || {
  echo "Tailscale is installed but not connected. Run: sudo tailscale up --hostname=tempo-hub" >&2
  exit 1
}

if [[ ! -f .env ]]; then
  {
    printf 'TEMPO_PAIRING_SECRET=%s\n' "$(openssl rand -hex 24)"
    printf 'TEMPO_HOST_BIND=127.0.0.1\n'
  } > .env
  chmod 600 .env
  echo "Created .env with a new pairing secret."
fi

pairing_secret="$(sed -n 's/^TEMPO_PAIRING_SECRET=//p' .env | head -n 1)"
if [[ ${#pairing_secret} -lt 24 ]] || [[ "$pairing_secret" == "replace-with-a-long-random-secret" ]]; then
  echo "TEMPO_PAIRING_SECRET in .env is missing or too short." >&2
  echo "Generate one with: openssl rand -hex 24" >&2
  exit 1
fi

docker compose config --quiet
docker compose up -d --build

echo "Waiting for Tempo Hub health check..."
healthy=0
for _ in $(seq 1 60); do
  if curl --fail --silent http://127.0.0.1:7700/api/health >/dev/null; then
    healthy=1
    break
  fi
  sleep 2
done

if [[ "$healthy" -ne 1 ]]; then
  docker compose logs --tail 80 tempo-hub
  echo "Tempo Hub did not become healthy." >&2
  exit 1
fi

sudo tailscale serve --bg http://127.0.0.1:7700

echo
echo "Tempo Hub is healthy and available only inside your tailnet:"
sudo tailscale serve status
echo
echo "Use the HTTPS URL printed above on both the desktop and Android app."
echo "To view the pairing secret locally: grep '^TEMPO_PAIRING_SECRET=' .env"
