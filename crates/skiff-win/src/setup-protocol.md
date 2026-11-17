# skiff-win setup: page ⇄ host protocol

The first-run setup page (`setup.html`, Windows only) talks to skiff-win the
way the main page does: it sends JSON strings with
`window.webkit.messageHandlers.skiff.postMessage(JSON.stringify({cmd, ...}))`
(skiff-win's shim forwards that to WebView2) and receives state with
`window.skiff.on(event)`.

## Page → host

| cmd | fields | when |
|---|---|---|
| `setup-ready` | | the page has loaded; the host answers with the current `setup` state |
| `setup-install-wsl` | | Welcome → "Install WSL" |
| `setup-restart` | | Restart → "Restart now" |
| `setup-later` | | Restart → "Restart later" (hides the window to the tray) |
| `setup-choose` | `value`: `"skiff"` or `"ubuntu"` | Choose a Linux → the primary button |
| `setup-switch` | `value`: `"skiff"` | while Ubuntu downloads → "Use Skiff Linux instead" |
| `setup-cancel` | | any progress screen → "Cancel" (back to the step's start) |
| `setup-retry` | | error → "Try again" |
| `setup-default` | `value`: bool | Docker tools → "Make Skiff the default for docker" checkbox |
| `setup-build` | `value`: `"docker"` or `"skiff"` | Docker tools → Builds (shown only with another engine and Skiff not made docker's default): "Keep building with Docker" / "Build with Skiff" |
| `setup-autostart` | `value`: bool | Ready → "Start Skiff when I sign in" |
| `setup-open` | | Ready → "Open Skiff" (the main window replaces setup) |
| `setup-quit` | | Welcome → "Quit" |
| `copy` | `value`: text | Ready → "Copy" |
| `open` | `name`: https URL | "What is WSL?" |

## Host → page

One event type, sent whenever anything changes (at most ~10 a second):

```json
{
  "type": "setup",
  "step": "welcome" | "wsl" | "restart" | "choose" | "linux" | "engine" | "tools" | "ready",
  "linux": "skiff" | "ubuntu" | null,
  "sizes": {"wsl": 367575040, "ubuntu": 418495746, "skiff": 9000000},
  "existingUbuntu": false,
  "otherDocker": false,
  "otherEngine": null | "Docker Desktop",
  "makeDefault": true,
  "buildSkiff": true,
  "autostart": true,
  "progress": {"pct": 58, "done": 224395264, "total": 367575040, "rate": 19084083, "eta": 9, "label": "Downloading WSL 3.0.3 from Microsoft"} | null,
  "tasks": [{"label": "Download WSL", "state": "done" | "active" | "todo" | "error", "detail": "214 / 368 MB"}],
  "log": ["skiff daemon: ready in 33 ms"],
  "error": null | "text"
}
```

- `step` picks the screen (variant A of the design canvas): welcome = A1,
  wsl = A2, restart = A3, choose = A4, linux = A5 (skiff) / A5b (ubuntu),
  engine = A6, tools = A7, ready = A8.
- `progress.done`/`total` are bytes when downloading (show "214 of 368 MB",
  rate as MB/s, eta in seconds); `total` null means an unsized step (show
  the percentage only, or an indeterminate bar when `pct` is null).
- `error`: shown on the current step's screen with "Try again" (`setup-retry`).
- The left rail derives from `step`: steps before it done, it current, after it to do.
