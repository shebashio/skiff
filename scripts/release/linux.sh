#!/bin/bash
# Native desktop packages plus the Linux payload used by Windows packaging.
set -euo pipefail
arch=${1:?architecture required}
test "$(uname -m)" = "$arch"
payload="target/linux-$arch-musl"
bash scripts/windows-engine.sh "$PWD" "$PWD/$payload" "$arch"
bash scripts/skiff-linux.sh "$PWD" "$PWD/$payload" "$PWD/$payload/skiff-linux.tar.gz" "$arch"
CARGO_TARGET_DIR=target/linux-window cargo build --release --locked -p skiff --features window
bash packaging/linux/fetch-cli.sh "target/linux-cli-$arch" "$HOME/.cache/skiff-linux-cli" "$arch"
bash packaging/linux/build.sh "$payload" target/packages "$arch" "target/linux-cli-$arch" target/linux-window/release/skiff
mkdir -p target/payload
tar -czf "target/payload/windows-linux-$arch.tar.gz" -C "$payload" skiff skiff-init skiff-linux.tar.gz
# Inspect both package formats and require the desktop executable in each.
for package in target/packages/skiff_*.deb; do
  dpkg-deb --contents "$package" > target/deb-contents.txt
  grep -q 'usr/lib/skiff/skiff-window$' target/deb-contents.txt
done
for package in target/packages/skiff-[0-9]*.rpm; do
  rpm -qlp "$package" > target/rpm-contents.txt
  grep -q '^/usr/lib/skiff/skiff-window$' target/rpm-contents.txt
done
