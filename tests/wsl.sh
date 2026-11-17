#!/bin/bash
# Skiff for Windows, without Windows: copies target/skiff-windows-x86_64 (what
# `cargo xtask windows` makes) into an x86-64 Ubuntu container standing in
# for the WSL2 distro, runs its install.sh as a Windows user would, then
# drives the engine from a second container with only the docker CLI,
# standing in for Windows. The daemon serves only its unix socket (on
# Windows, skiff-win.exe reaches it through `skiff bridge`), so the second
# container shares the socket's directory. On Apple Silicon use aarch64:
# Rosetta runs the x86-64 build but has no clone3, which every container
# start needs (WSL2's kernel has it). What this cannot cover: WSL2's own
# kernel, and skiff-win.exe's relay. Usage: tests/wsl.sh [x86_64|aarch64]
set -u
D=${D:-docker}
ARCH=${1:-x86_64}
case $ARCH in x86_64) PLATFORM=linux/amd64 ;; aarch64) PLATFORM=linux/arm64 ;; *) echo "usage: $0 [x86_64|aarch64]"; exit 2 ;; esac
DIR=$(cd "$(dirname "$0")/.." && pwd)/target/skiff-windows-$ARCH
[ -x "$DIR/skiff" ] || { echo "build first: cargo xtask windows (--arch arm64 for aarch64)"; exit 1; }
SOCK=/var/lib/skiff-daemon/.skiff/run
# The containers' and volumes' name prefix: another runs a second copy beside this one.
W=${W:-skiff-wsl}
$D rm -f $W $W-cli >/dev/null 2>&1
$D volume rm $W-run $W-lib $W-store >/dev/null 2>&1
# WSL's disk is ext4; this container's own root is overlayfs, which can't be
# an overlay's upper layer, so Skiff's state goes on volumes, as in
# linux-host.sh.
$D run -d --name "$W" --platform $PLATFORM --privileged --tmpfs /run -v $W-run:$SOCK \
  -v $W-lib:/var/lib/skiff -v $W-store:/var/lib/skiff-daemon/.local/share/skiff ubuntu:24.04 sleep infinity >/dev/null
$D exec $W mkdir -p /tmp/skiff-windows
$D cp "$DIR/skiff" $W:/tmp/skiff-windows/skiff >/dev/null
$D cp "$DIR/skiff-init" $W:/tmp/skiff-windows/skiff-init >/dev/null
$D cp "$DIR/install.sh" $W:/tmp/skiff-windows/install.sh >/dev/null
$D exec $W sh -c 'cd /tmp/skiff-windows && sh ./install.sh'
$D run -d --name $W-cli --network container:$W -v $W-run:/skiff alpine sh -c 'apk add -q docker-cli curl >/dev/null 2>&1; sleep infinity' >/dev/null
for _ in $(seq 90); do $D exec $W-cli docker -H unix:///skiff/docker.sock version >/dev/null 2>&1 && break; sleep 1; done
PASS=0; FAIL=0
t() {
  local name=$1 want=$2; shift 2
  local got; got=$($D exec $W-cli sh -c "export DOCKER_HOST=unix:///skiff/docker.sock; $*" 2>&1 | tr -d '\r')
  if [[ "$got" == *"$want"* ]]; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); printf '  FAIL %s\n    want: %s\n    got:  %s\n' "$name" "$want" "$(echo "$got" | head -3)"; fi
}
# No systemd here, so install.sh must have added the WSL boot command.
if $D exec $W grep -q '/usr/local/bin/skiff daemon' /etc/wsl.conf; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); echo "  FAIL boot-entry: no skiff line in /etc/wsl.conf"; fi
t arch "$ARCH" "docker version --format '{{.Server.Arch}}' | sed 's/amd64/x86_64/; s/arm64/aarch64/'"
t hello "Hello from Docker!" "docker run --rm hello-world"
t run "3." "docker run --rm alpine cat /etc/alpine-release"
t internet "Example Domain" "docker run --rm alpine wget -qO- -T 10 http://example.com"
t port "Welcome to nginx" "docker run -d --name web -p 18082:80 nginx:alpine >/dev/null; for i in \$(seq 40); do curl -s localhost:18082 && break; sleep 0.5; done"
t exec "nginx version" "docker exec web nginx -v"
t port-inspect '{"80/tcp":[{"HostIp":"","HostPort":"18082"}]}' "docker inspect -f '{{json .HostConfig.PortBindings}}' web"
# What the registry serves now, for update checkers (#257).
t distribution '"mediaType":"application/vnd.oci.image.index.v1+json"' "curl -s --unix-socket /skiff/docker.sock http://d/v1.47/distribution/library/alpine:3.21/json"
t distribution-missing "404" "curl -s -o /dev/null -w '%{http_code}' --unix-socket /skiff/docker.sock http://d/v1.47/distribution/library/no-such-image-skiff-test/json"
# A copy into a created container, then its start, mounts the same upper
# dir twice in a row: kernels with overlayfs index on (Arch) refused the
# second with EBUSY (#227). A renamed directory must survive a commit.
t cp-then-start "failed 0" "f=0; for i in 1 2 3 4 5; do id=\$(docker create alpine sleep 30); docker cp /etc/hostname \$id:/f >/dev/null 2>&1 || f=\$((f+1)); docker start \$id >/dev/null 2>&1 || f=\$((f+1)); docker rm -f \$id >/dev/null; done; echo failed \$f"
t commit-renamed-dir "kept" "docker rm -f skiff-rename >/dev/null 2>&1; docker run --name skiff-rename alpine mv /etc/apk /etc/apk2; docker commit skiff-rename skiff-rename-test >/dev/null; docker rm -f skiff-rename >/dev/null; docker run --rm skiff-rename-test sh -c 'test -f /etc/apk2/repositories && echo kept'; docker rmi skiff-rename-test >/dev/null"
# A start that fails once its network is up leaves no name behind (#325).
t failed-start-no-dns "none" "docker network create skiff-dnsfail >/dev/null 2>&1; docker rm -f skiff-dnsbad >/dev/null 2>&1; docker run -d --name skiff-dnsbad --network skiff-dnsfail --sysctl net.ipv4.skiff_no_such_key=1 alpine sleep 60 >/dev/null 2>&1; docker run --rm --network skiff-dnsfail alpine sh -c 'nslookup skiff-dnsbad 2>&1 | grep -qE \"^Address: [0-9.]+\$\" && echo answered || echo none'; docker rm -f skiff-dnsbad >/dev/null 2>&1; docker network rm skiff-dnsfail >/dev/null"
t network-dns "PONG" "docker network create wn >/dev/null 2>&1; docker run -d --name db --network wn redis:7-alpine >/dev/null; sleep 2; docker run --rm --network wn redis:7-alpine redis-cli -h db ping"
t volume "kept" "docker run --rm -v wv:/d alpine sh -c 'echo kept > /d/f' && docker run --rm -v wv:/d alpine cat /d/f"
# A fresh named volume is filled from the image while the container starts.
# `docker run -d` must not return before the container's root is in place, or
# an immediate exec resolves users against the host's /etc/passwd (#311).
t volume-user-ready "passed passed passed" "docker run --name ready-seed alpine sh -c 'adduser -D dev && mkdir /payload && dd if=/dev/zero of=/payload/blob bs=1M count=64' >/dev/null && docker commit -c 'USER dev' ready-seed skiff-test/ready:1 >/dev/null && docker rm -f ready-seed >/dev/null; for i in 1 2 3; do v=ready-vol-\$i; c=\$(docker run -d --mount type=volume,source=\$v,target=/payload skiff-test/ready:1 sleep 60); if docker exec --user dev \$c id >/dev/null 2>&1; then printf 'passed '; else printf 'failed '; fi; docker rm -f \$c >/dev/null; docker volume rm \$v >/dev/null 2>&1; done; docker rmi skiff-test/ready:1 >/dev/null; echo"
t seccomp "Seccomp:	2" "docker run --rm alpine grep Seccomp: /proc/self/status"
t cleanup "web" "docker rm -f web db"
# The agent (skiff-init under the daemon) dies with nobody asking (#50):
# `always` and `unless-stopped` come back by themselves; no policy, or a
# `docker stop`, stays down.
t crash-setup "ok" "for p in always unless-stopped no; do docker run -d --name cr-\$p --restart \$p alpine sleep 1d >/dev/null; done; docker run -d --name cr-user --restart always alpine sleep 1d >/dev/null; docker stop -t 0 cr-user >/dev/null && echo ok"
if $D exec $W sh -c 'kill -9 $(pgrep -x -P "$(pgrep -xo skiff)" skiff-init)'; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); echo "  FAIL crash-kill: no agent to kill"; fi
t crash-recovers "cr-always=true cr-unless-stopped=true cr-no=false cr-user=false" "sleep 4; for c in cr-always cr-unless-stopped cr-no cr-user; do printf '%s=%s ' \$c \$(docker inspect -f '{{.State.Running}}' \$c); done"
# Removing a container the crash stopped, or one never started, deletes its
# writable layer from the engine's disk.
for c in crash created; do
  if [ $c = crash ]; then id=$($D exec $W-cli sh -c 'export DOCKER_HOST=unix:///skiff/docker.sock; id=$(docker inspect -f "{{.Id}}" cr-no); docker rm cr-no >/dev/null; echo $id')
  else id=$($D exec $W-cli sh -c 'export DOCKER_HOST=unix:///skiff/docker.sock; id=$(docker create alpine true); docker cp /etc/hostname $id:/f >/dev/null; docker rm $id >/dev/null; echo $id'); fi
  if [ -n "$id" ] && $D exec $W sh -c "for i in \$(seq 20); do test -e /var/lib/skiff/containers/$id || exit 0; sleep 0.1; done; exit 1"; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); echo "  FAIL $c-rm-frees-layer: ${id:-no id} still on disk"; fi
done
t crash-cleanup "cr-always" "docker rm -f cr-always cr-unless-stopped cr-user"
# A containers.json cut short (a crash, a full disk) is kept aside as
# containers.json.corrupt-*, the records still readable are loaded, and
# nothing is deleted because of the damaged list: no log folder and no
# writable layer, also once another container has started and after the
# next start, which finds a readable but shorter list (#438). The engine is
# restarted as install.sh starts it without systemd.
ST=/var/lib/skiff-daemon/.local/share/skiff
stop_engine() { $D exec "$W" sh -c 'pkill -TERM -x skiff; for i in $(seq 100); do pgrep -x skiff >/dev/null || break; sleep 0.2; done'; }
start_engine() {
  $D exec -d "$W" sh -c 'HOME=/var/lib/skiff-daemon nohup /usr/local/bin/skiff daemon >>/var/log/skiff.log 2>&1'
  for _ in $(seq 60); do $D exec "$W-cli" docker -H unix:///skiff/docker.sock version >/dev/null 2>&1 && break; sleep 1; done
}
in_wsl() { $D exec "$W" sh -c "$1" 2>&1; }
t cj-setup "ok" "docker run --name cj1 -v cjv1:/v alpine sh -c 'echo line-one; echo precious > /data' >/dev/null && docker run --name cj2 -v cjv2:/v alpine sh -c 'echo line-two; echo precious > /data' >/dev/null && echo ok"
CJ_IDS=$($D exec "$W-cli" sh -c "docker -H unix:///skiff/docker.sock inspect -f '{{.Id}}' cj1 cj2" | tr '\n' ' ')
stop_engine
# Cut inside the record of whichever of the two comes later in the file, so
# the other one can still be read; print that one's id.
CJ_READ=$($D exec -i -e "IDS=$CJ_IDS" -e "f=$ST/containers.json" "$W" sh <<'SH'
set -- $IDS
a=$(awk -v id="$1" '{print index($0, "{\"id\":\"" id "\"")}' $f)
b=$(awk -v id="$2" '{print index($0, "{\"id\":\"" id "\"")}' $f)
[ "$a" -gt 0 ] && [ "$b" -gt 0 ] || exit
if [ "$a" -lt "$b" ]; then cut=$b read=$1; else cut=$a read=$2; fi
truncate -s $((cut + 20)) $f && echo "$read"
SH
)
start_engine
CJ_KEPT="n=0; for id in $CJ_IDS; do test -d $ST/containers/\$id && test -d /var/lib/skiff/containers/\$id && n=\$((n+1)); done; echo kept \$n"
cj() { local name=$1 want=$2 got; got=$(in_wsl "$3"); if [[ "$got" == *"$want"* ]]; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); printf '  FAIL %s\n    want: %s\n    got:  %s\n' "$name" "$want" "$(echo "$got" | head -3)"; fi; }
cj cj-copy-kept "containers.json.corrupt-" "ls $ST"
t cj-info-warns "containers.json.corrupt-" "docker info --format '{{json .Warnings}}'"
t cj-readable-listed "${CJ_READ:-no id read}" "docker ps -a -q --no-trunc"
cj cj-nothing-deleted "kept 2" "$CJ_KEPT"
t cj-run "ran" "docker run --rm alpine true && echo ran"
cj cj-nothing-deleted-after-run "kept 2" "$CJ_KEPT"
t cj-readable-logs "line-" "docker logs ${CJ_READ:-none}"
# The cut container's volume is used by no listed container: neither a
# prune nor a removal without --force takes it while the list is damaged.
t cj-prune-refused "were not pruned" "docker volume prune -af 2>&1"
t cj-prune-kept-volumes "volumes 2" "echo volumes \$(docker volume ls -q | grep -c '^cjv')"
t cj-volume-rm-refused "was not removed" "docker volume rm cjv1 cjv2 2>&1"
t cj-volume-rm-kept-volumes "volumes 2" "echo volumes \$(docker volume ls -q | grep -c '^cjv')"
stop_engine; start_engine
t cj-run-after-restart "ran" "docker run --rm alpine true && echo ran"
cj cj-next-start-deletes-nothing "kept 2" "$CJ_KEPT"
t cj-cleanup "done" "docker rm -f cj1 cj2 >/dev/null 2>&1; docker volume rm -f cjv1 cjv2 >/dev/null 2>&1; echo done"
in_wsl "for id in $CJ_IDS; do rm -rf $ST/containers/\$id /var/lib/skiff/containers/\$id; done; rm -f $ST/containers.json.corrupt-*" >/dev/null
$D rm -f $W $W-cli >/dev/null
$D volume rm $W-run $W-lib $W-store >/dev/null 2>&1
echo "$PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
