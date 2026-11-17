# Skiff on a Linux host as an RPM: `skiff` (engine + docker CLI, Compose,
# buildx) or `skiff-lite` (engine only), chosen by packaging/linux/build.sh
# through skiff_name and with_cli. Static (musl) binaries: no runtime
# dependencies, no debuginfo. With with_window, `skiff` also carries the
# window (skiff built against GTK 3 and WebKitGTK 4.1), which needs those.
%global debug_package %{nil}
%global __strip /bin/true
%global __brp_mangle_shebangs %{nil}

Name:           %{skiff_name}
Version:        %{skiff_version}
Release:        1%{?dist}
Summary:        Tiny, fast container runtime (Docker API)%{!?with_cli:, engine only}
License:        Apache-2.0
URL:            https://github.com/shebashio/skiff
Source0:        skiff
Source1:        skiff-init
Source2:        skiff.service
Source3:        LICENSE
Source4:        NOTICE
Source18:       io.shebash.skiff.host-access.policy
%if 0%{?with_cli}
Source5:        docker
Source6:        docker-compose
Source7:        docker-buildx
Source8:        docker-wrapper
Conflicts:      skiff-lite
%if 0%{?with_window}
Source9:        skiff-window
Source10:       skiff.desktop
Source11:       skiff-app-icon.svg
Source12:       skiff-app-icon-16.png
Source13:       skiff-app-icon-32.png
Source14:       skiff-app-icon-64.png
Source15:       skiff-app-icon-128.png
Source16:       skiff-app-icon-256.png
Source17:       skiff-app-icon-512.png
Source19:       io.shebash.skiff.Skiff.metainfo.xml
Requires:       gtk3
Requires:       webkit2gtk4.1
# The window's setup checklist asks for the password through pkexec.
Recommends:     polkit
%endif
%else
Conflicts:      skiff
%endif
BuildArch:      %{skiff_arch}
# Static binaries: nothing to find by scanning them.
AutoReq:        no
Requires(pre):  shadow-utils
%if 0%{?with_cli}
Requires(post): %{_sbindir}/update-alternatives
Requires(preun): %{_sbindir}/update-alternatives
Requires(posttrans): %{_sbindir}/update-alternatives
%endif

%description
Skiff runs containers on this machine's kernel and speaks the Docker API on
/run/skiff/docker.sock for users in the skiff group (and at
/var/run/docker.sock while no Docker engine has it). No VM, no daemon other
than skiff itself. Members of the skiff group can run containers as root on
this machine; treat it like the docker group.
%if 0%{?with_cli}
This package brings the docker CLI, Compose and buildx with it, as Skiff.app
does on the Mac: `docker run` works right after installing. A docker CLI from
another package keeps precedence on PATH.
%if 0%{?with_window}
Skiff's window is in the app menu, or: skiff ui
%endif
%else
Engine only: point any docker CLI at it with
  docker context create skiff --docker host=unix:///run/skiff/docker.sock
%endif

%prep

%build

%install
install -D -m 755 %{SOURCE0} %{buildroot}%{_bindir}/skiff
install -D -m 755 %{SOURCE1} %{buildroot}%{_bindir}/skiff-init
install -D -m 644 %{SOURCE2} %{buildroot}%{_unitdir}/skiff.service
install -D -m 644 %{SOURCE3} %{buildroot}%{_licensedir}/%{name}/LICENSE
install -D -m 644 %{SOURCE4} %{buildroot}%{_licensedir}/%{name}/NOTICE
install -D -m 644 %{SOURCE18} %{buildroot}%{_datadir}/polkit-1/actions/io.shebash.skiff.host-access.policy
%if 0%{?with_cli}
install -D -m 755 %{SOURCE5} %{buildroot}/usr/libexec/skiff/bin/docker
install -D -m 755 %{SOURCE8} %{buildroot}/usr/libexec/skiff/docker
install -D -m 755 %{SOURCE6} %{buildroot}/usr/libexec/skiff/cli-plugins/docker-compose
install -D -m 755 %{SOURCE7} %{buildroot}/usr/libexec/skiff/cli-plugins/docker-buildx
%endif
%if 0%{?with_window}
install -D -m 755 %{SOURCE9} %{buildroot}/usr/lib/skiff/skiff-window
install -D -m 644 %{SOURCE10} %{buildroot}%{_datadir}/applications/skiff.desktop
install -D -m 644 %{SOURCE19} %{buildroot}%{_datadir}/metainfo/io.shebash.skiff.Skiff.metainfo.xml
install -D -m 644 %{SOURCE11} %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/skiff.svg
for s in 16 32 64 128 256 512; do
  install -D -m 644 %{_sourcedir}/skiff-app-icon-$s.png %{buildroot}%{_datadir}/icons/hicolor/${s}x${s}/apps/skiff.png
done
%endif

%pre
getent group skiff >/dev/null || groupadd --system skiff
exit 0

%post
if [ -d /run/systemd/system ]; then
  systemctl daemon-reload >/dev/null 2>&1 || :
  # Not started or enabled: it starts when someone sets Skiff up (the window's
  # Allow access, or `skiff host-access`). An upgrade ($1 >= 2) restarts a
  # running engine on the new binary, as the Mac and Windows apps do after
  # an update; its containers stop, and those with a restart policy start again.
  if [ "$1" -ge 2 ] && systemctl is-active -q skiff.service; then
    echo "Skiff is upgraded: restarting the engine on the new version."
    systemctl try-restart skiff.service || echo "  The engine didn't restart: sudo systemctl restart skiff"
  fi
fi
# A `skiff` before /usr/bin on PATH runs instead of this package's.
for d in /usr/local/sbin /usr/local/bin; do
  if [ -e "$d/skiff" ] && [ "$(readlink -f "$d/skiff")" != /usr/bin/skiff ]; then
    echo "Warning: $d/skiff comes before /usr/bin/skiff on PATH, so it runs instead of this Skiff."
    echo "  If it's an older Skiff, remove it:  sudo rm $d/skiff"
  fi
done
%if 0%{?with_cli}
# Only a /usr/bin/docker that is absent or already an alternatives link; a
# real file, or a symlink another package or the administrator put there, is
# left alone: Skiff's CLI must not take over someone else's docker.
free=1
if [ -e /usr/bin/docker ] || [ -L /usr/bin/docker ]; then
  case "$(readlink /usr/bin/docker 2>/dev/null)" in /etc/alternatives/*) ;; *) free=0 ;; esac
fi
if [ "$free" = 1 ]; then
  %{_sbindir}/update-alternatives --install /usr/bin/docker docker /usr/libexec/skiff/docker 10 || :
  echo "Skiff is installed with its own docker CLI (docker, docker compose, docker buildx)."
else
  echo "Skiff is installed; /usr/bin/docker belongs to something else and stays. Skiff's CLI: /usr/libexec/skiff/docker"
fi
%else
echo "Skiff is installed (engine only). Point a docker CLI at it:"
echo "  docker context create skiff --docker host=unix:///run/skiff/docker.sock && docker context use skiff"
%endif
echo "To use it without sudo:  sudo skiff host-access \$USER   (or open Skiff, which sets it up)"
echo "Members of the skiff group can run containers as root on this machine; treat it like the docker group."
echo "While no Docker engine has /var/run/docker.sock, it leads to Skiff too (for Testcontainers, docker-py and other tools that ignore docker contexts)."
exit 0

%preun
if [ "$1" -eq 0 ]; then
  if [ -d /run/systemd/system ]; then
    systemctl disable --now skiff.service >/dev/null 2>&1 || :
  fi
  # Everything else it runs: a window or skiff command still open.
  for sig in TERM KILL; do
    pkill -$sig -f '^/usr/lib/skiff/skiff-window( |$)' >/dev/null 2>&1 || :
    pkill -$sig -f '^(/usr)?/bin/skiff( |$)' >/dev/null 2>&1 || :
    [ "$sig" = KILL ] || sleep 1
  done
%if 0%{?with_cli}
  %{_sbindir}/update-alternatives --remove docker /usr/libexec/skiff/docker || :
  for p in docker-compose docker-buildx; do
    %{_sbindir}/update-alternatives --remove $p /usr/libexec/skiff/cli-plugins/$p || :
  done
%endif
fi
exit 0

%if 0%{?with_cli}
%posttrans
# Compose and buildx into /usr/lib/docker/cli-plugins where the slot is free
# and Docker's own copy isn't in another of the CLI's plugin folders; else
# Skiff's link goes and Docker's copy is used. In %posttrans because an
# upgrade from a Skiff that put real files there erases them only after %post;
# again whenever another package changes Docker's plugin folders.
mkdir -p /usr/lib/docker/cli-plugins
for p in docker-compose docker-buildx; do
  l=/usr/lib/docker/cli-plugins/$p
  theirs=
  for d in /usr/local/lib/docker/cli-plugins /usr/local/libexec/docker/cli-plugins /usr/libexec/docker/cli-plugins; do
    [ -e $d/$p ] && theirs=1
  done
  if [ -z "$theirs" ] && { [ ! -e $l ] && [ ! -L $l ] || readlink $l 2>/dev/null | grep -q '^/etc/alternatives/'; }; then
    %{_sbindir}/update-alternatives --install $l $p /usr/libexec/skiff/cli-plugins/$p 10 || :
  elif %{_sbindir}/update-alternatives --display $p 2>/dev/null | grep -q "^/usr/libexec/skiff/cli-plugins/$p "; then
    %{_sbindir}/update-alternatives --remove $p /usr/libexec/skiff/cli-plugins/$p || :
  fi
done
exit 0

%filetriggerin -- /usr/lib/docker/cli-plugins /usr/libexec/docker/cli-plugins /usr/local/lib/docker/cli-plugins /usr/local/libexec/docker/cli-plugins
mkdir -p /usr/lib/docker/cli-plugins
for p in docker-compose docker-buildx; do
  l=/usr/lib/docker/cli-plugins/$p
  theirs=
  for d in /usr/local/lib/docker/cli-plugins /usr/local/libexec/docker/cli-plugins /usr/libexec/docker/cli-plugins; do
    [ -e $d/$p ] && theirs=1
  done
  if [ -z "$theirs" ] && { [ ! -e $l ] && [ ! -L $l ] || readlink $l 2>/dev/null | grep -q '^/etc/alternatives/'; }; then
    %{_sbindir}/update-alternatives --install $l $p /usr/libexec/skiff/cli-plugins/$p 10 || :
  elif %{_sbindir}/update-alternatives --display $p 2>/dev/null | grep -q "^/usr/libexec/skiff/cli-plugins/$p "; then
    %{_sbindir}/update-alternatives --remove $p /usr/libexec/skiff/cli-plugins/$p || :
  fi
done
exit 0

%filetriggerpostun -- /usr/lib/docker/cli-plugins /usr/libexec/docker/cli-plugins /usr/local/lib/docker/cli-plugins /usr/local/libexec/docker/cli-plugins
mkdir -p /usr/lib/docker/cli-plugins
for p in docker-compose docker-buildx; do
  l=/usr/lib/docker/cli-plugins/$p
  theirs=
  for d in /usr/local/lib/docker/cli-plugins /usr/local/libexec/docker/cli-plugins /usr/libexec/docker/cli-plugins; do
    [ -e $d/$p ] && theirs=1
  done
  if [ -z "$theirs" ] && { [ ! -e $l ] && [ ! -L $l ] || readlink $l 2>/dev/null | grep -q '^/etc/alternatives/'; }; then
    %{_sbindir}/update-alternatives --install $l $p /usr/libexec/skiff/cli-plugins/$p 10 || :
  elif %{_sbindir}/update-alternatives --display $p 2>/dev/null | grep -q "^/usr/libexec/skiff/cli-plugins/$p "; then
    %{_sbindir}/update-alternatives --remove $p /usr/libexec/skiff/cli-plugins/$p || :
  fi
done
exit 0
%endif

%postun
if [ -d /run/systemd/system ]; then
  systemctl daemon-reload >/dev/null 2>&1 || :
fi
if [ "$1" -eq 0 ]; then
  echo "Skiff is removed; its images, containers and volumes stay in /var/lib/skiff"
fi
exit 0

%files
%license %{_licensedir}/%{name}/LICENSE
%license %{_licensedir}/%{name}/NOTICE
%{_bindir}/skiff
%{_bindir}/skiff-init
%{_unitdir}/skiff.service
%{_datadir}/polkit-1/actions/io.shebash.skiff.host-access.policy
%if 0%{?with_cli}
%dir /usr/libexec/skiff
%dir /usr/libexec/skiff/bin
/usr/libexec/skiff/bin/docker
/usr/libexec/skiff/docker
%dir /usr/libexec/skiff/cli-plugins
/usr/libexec/skiff/cli-plugins/docker-compose
/usr/libexec/skiff/cli-plugins/docker-buildx
%endif
%if 0%{?with_window}
%dir /usr/lib/skiff
/usr/lib/skiff/skiff-window
%{_datadir}/applications/skiff.desktop
%{_datadir}/icons/hicolor/*/apps/skiff.*
%{_datadir}/metainfo/io.shebash.skiff.Skiff.metainfo.xml
%endif

%changelog
* Sun Oct 25 2026 SHE BASH - %{skiff_version}-1
- Packaged from the static Linux build
