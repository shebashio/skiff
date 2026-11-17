#!/bin/sh
# Skiff on Windows: run inside a WSL2 distro, as root, from the directory that
# holds the Linux build (cargo xtask windows; x86-64, or arm64 with --arch arm64):
#   wsl -d Ubuntu -u root sh ./install.sh
# Containers run on WSL2's Linux kernel. The daemon serves its unix socket
# only; skiff-win.exe carries the Windows docker CLI's connections to it over
# `skiff bridge` (see ../windows/skiff.ps1). It is not served on TCP: WSL's
# localhost relay drops half-closes, which `docker run` needs.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
install -m 755 "$here/skiff" "$here/skiff-init" /usr/local/bin/
if [ -d /run/systemd/system ]; then
  cat > /etc/systemd/system/skiff.service <<UNIT
[Unit]
Description=Skiff container engine
After=network-online.target
[Service]
ExecStart=/usr/local/bin/skiff daemon
Environment=HOME=/var/lib/skiff-daemon
Delegate=yes
KillMode=mixed
Restart=on-failure
UMask=0027
[Install]
WantedBy=multi-user.target
UNIT
  systemctl daemon-reload
  systemctl enable skiff
  systemctl restart skiff
else
  # No systemd in this distro: start Skiff when WSL boots it.
  if ! grep -q 'skiff daemon' /etc/wsl.conf 2>/dev/null; then
    printf '\n[boot]\ncommand = HOME=/var/lib/skiff-daemon nohup /usr/local/bin/skiff daemon >/var/log/skiff.log 2>&1 &\n' >> /etc/wsl.conf
  fi
  pkill -f '/usr/local/bin/skiff daemon' 2>/dev/null || true
  HOME=/var/lib/skiff-daemon nohup /usr/local/bin/skiff daemon >/var/log/skiff.log 2>&1 &
fi
echo "Skiff runs in WSL on /var/lib/skiff-daemon/.skiff/run/docker.sock"
