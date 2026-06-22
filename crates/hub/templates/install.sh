#!/bin/sh
# last-monitor agent installer (native binary + systemd).
#   curl -fsSL <HUB>/install.sh | HUB_URL=<HUB> API_KEY=<key> sh
#
# Requires root (writes /usr/local/bin and a systemd unit). Linux x86_64/arm64.
set -eu

: "${HUB_URL:?set HUB_URL, e.g. HUB_URL=https://monitor.senprints.net}"
: "${API_KEY:?set API_KEY (from Add System in the UI)}"

REPO="The-Last-Devops/last-monitor"
SUDO=""
[ "$(id -u)" -ne 0 ] && SUDO="sudo"

case "$(uname -m)" in
  x86_64|amd64)  ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *) echo "unsupported arch $(uname -m); use the Docker method instead" >&2; exit 1 ;;
esac

echo "→ finding latest last-agent release for linux-$ARCH…"
URL=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
  | grep -oE "https://[^\"]*linux-$ARCH[^\"]*\.tar\.gz" | head -1)
if [ -z "$URL" ]; then
  echo "no linux-$ARCH binary in the latest release; use the Docker method instead" >&2
  exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
echo "→ downloading $URL"
curl -fsSL "$URL" | tar -xz -C "$TMP"
$SUDO install -m 0755 "$(find "$TMP" -name last-agent -type f | head -1)" /usr/local/bin/last-agent

echo "→ writing /etc/last-monitor/agent.env"
$SUDO mkdir -p /etc/last-monitor
printf 'HUB_URL=%s\nAPI_KEY=%s\nDISK_PATH=/\n' "$HUB_URL" "$API_KEY" | $SUDO tee /etc/last-monitor/agent.env >/dev/null
$SUDO chmod 600 /etc/last-monitor/agent.env

echo "→ installing systemd service"
$SUDO tee /etc/systemd/system/last-agent.service >/dev/null <<'UNIT'
[Unit]
Description=last-monitor agent
After=network-online.target
Wants=network-online.target
[Service]
EnvironmentFile=/etc/last-monitor/agent.env
ExecStart=/usr/local/bin/last-agent
Restart=always
RestartSec=5
[Install]
WantedBy=multi-user.target
UNIT

$SUDO systemctl daemon-reload
$SUDO systemctl enable --now last-agent
echo "✓ last-agent installed and started. Check: systemctl status last-agent"
