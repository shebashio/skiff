# Security

Skiff runs a virtual machine, containers that can be privileged, and a local
certificate authority on your computer (macOS, Windows or Linux), so security
reports matter a lot.

## Reporting a vulnerability

Please do not open a public issue. Use GitHub's private vulnerability
reporting on this repository (Security → Report a vulnerability).

Include what you found, how to reproduce it, and the Skiff version
(Skiff → About in the app). The maintainers will investigate the report and respond through GitHub. Reporters are credited in
the release notes unless they prefer not to be.

## Supported versions

Security fixes currently target the latest main branch. Update your checkout
and rebuild before reporting; Skiff has no default automatic-update feed.

## In scope

- Escaping a container or the VM to the host (the Mac, Windows or the Linux
  machine).
- The daemon socket, the HTTP/HTTPS router, DNS, and the certificate
  authority (it is limited to `skiff.local` and `skiff.localhost` names).
- The installer, `skiff setup` and auto-update.
