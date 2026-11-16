# Changelog

## Unreleased

### New
- Widget menus (a script's `menu:` lines) are drawn by wingbar in the bar's
  colours: a panel hanging from the wing under the widget, joined to it (a
  square wing corner where they're flush, curved fillets elsewhere), with
  readings dimmed and the item under the pointer highlighted. Opened by a
  hover, it closes when the pointer leaves; opened by a click, it stays until a
  click elsewhere (or on the widget again) or a choice. `theme.menus: "native"` keeps
  macOS menus. The right-click wingbar menu is still a macOS menu.

## 0.3.2 — 2026-11-16

### Fixed
- A BarPopover panel opened under the first widget of the right wing (or the
  last of the left wing) met the wing's rounded inner corner, leaving a notch
  between the two. That corner now goes square while the panel is open, so
  wing and panel join as one shape, and rounds again as it closes.

## 0.3.1 — 2026-11-13

### Fixed
- Adding a widget that has nothing to show yet (Spotify while nothing plays,
  AeroSpace's mode while in main, a script that prints nothing or fails) looked
  like nothing happened, because an empty widget takes no room. After its first
  run, the menu now says so in an alert and the CLI in a `note:` line: that it
  is on the bar, and when it appears (or why its script failed).
- Short window titles that fit (`cmux`) are shown; only a title squeezed under
  40 pt is dropped.

### New
- simple-bar's icons are bundled (259 icons, MIT, © Jean Tinland): any icon
  option can name one (`ClickUp`, `MicOff`, `GitHub`), next to SF Symbols.
- The line-art icons of the simple-bar "wings" theme are bundled too: `Memory`,
  `Storage`, `Ethernet`, `Speaker1`–`Speaker3`, `SpeakerMuted`, and one per
  prayer (`Fajr`, `Chorouq`, `Dohr`, `Asr`, `Maghrib`, `Ishae`).
- `theme.icons: "simple-bar"` draws the bar as that theme did: CPU with
  simple-bar's icon, sound with the theme's speakers (and "Muted" in red),
  Spotify/Music with simple-bar's, up to three app icons after each workspace
  (none for apps without one), prayer times with each prayer's icon in its
  colour. The import from `.simplebarrc` sets it, keeps every widget's own
  simple-bar icon, and gives `memory`, `storage` and Ethernet the theme's icons.

- `popoverTab: true` on a widget: wingbar tells BarPopover where that widget
  is (as simple-bar's hover script did), so BarPopover's prayer tab hangs under
  it, also after BarPopover restarts. The import sets it on the prayer widget.

### Install
- Ready-built releases: each `v*` tag publishes a universal (Apple Silicon +
  Intel) `Wingbar.app`, so installing no longer needs Rust.
- One-line install and update:
  `curl -fsSL https://raw.githubusercontent.com/iammosab/wingbar/main/install.sh | bash`.
  In a clone `install.sh` still builds from source (`--download` for the release).
- Homebrew: `brew install iammosab/tap/wingbar`, then `brew services start wingbar`.
- `install.sh` stops a running bar before replacing it, so macOS no longer kills
  it for an invalid code signature mid-update.

### Changed
- `/widgets` reports `showing` for each running widget: whether it takes room
  on the bar now.

## 0.3.0 — 2026-11-12

wingbar is now a general status bar: nothing in it is specific to one setup, and
widgets are managed from the bar or the command line.

### New
- Right-click menu on the bar: move, hide, remove, tick widgets on and off, add a
  built-in, catalog or custom script widget, open the settings, reload, quit.
- `wingbar widget list | catalog | add | remove | show | hide | move | set` and
  `wingbar config path | check | edit | import-simplebar`.
- Changes apply at once without restarting the bar: from the menu, the commands,
  `wingbar reload`, or saving `config.json` by hand (the folder is watched).
- Widget scripts can print `icon:` and `menu:` lines; menus open natively on click
  or hover. BarPopover still works but is optional.
- A catalog of ready-made widgets: storage, network, mic, adb, aerospace-mode,
  git-branch.
- Any number of widgets of a type (two clocks), and any level names with their own
  colour and icon.
- `wingbar --version`; `wingbar widget list` shows each widget's last error.
- Script option `"raw": true`: the first line is shown as it is, with no `level|label`,
  `icon:` or `menu:` parsing. The import sets it on simple-bar's plain user widgets,
  so a label containing `|` keeps showing in full.

### Changed
- Settings live in wingbar's own `~/.config/wingbar/config.json` (version 2). On
  first start `~/.simplebarrc` is imported once (never modified); a 0.2
  `config.json` is merged and kept as `config.v1.json`. After that wingbar no
  longer reads `.simplebarrc`.
- Defaults that were specific to one setup (ClickUp, Claude, prayer times, ADB,
  BarPopover, special icons) are gone from the code; the import writes them as
  explicit options for whoever had them.
- Widgets that are switched on but not placed in a wing no longer run.
- Script output is capped at 64 lines / 16 KB, and a non-zero exit shows as an
  error in `wingbar status`.
- `wingbar reload` reloads in place instead of restarting the process.
- `--help` prints a usage text; unknown flags are errors.
- `config import-simplebar --force` keeps numbered backups (`config.json.bak`, `.bak-2`, …)
  and never overwrites one. `widget list` and `widget catalog` never write settings.
  No command converts the settings while an older wingbar is running on them.
- A symlinked `config.json` is written through to its target and keeps its permissions.
- Widget names in routes and `wingbar refresh NAME` are case-insensitive.

## 0.2.0 — 2026-11-12

An engineering pass: same bar, same look, measured lighter and much easier to change.

### Faster and lighter
- The bar draws only its two wings instead of a screen-wide surface. Measured with
  `scripts/measure.sh` (120 s runs): the bar's own work fell from ~16–19M to ~4.2–4.5M
  instructions/s with built-in widgets, and from ~19–25M to ~8–10M with a full config;
  peak memory footprint fell from ~145 MB to ~15–17 MB.
- Drawing no longer holds the shared state lock while measuring text and icons.
- Fewer stray threads: popover requests share one thread, volume scrolling no longer
  starts a thread per step.

### Fixed
- An invalid colour in `config.json` (e.g. `"wing": "black"`) crashed the bar at startup;
  it now warns and uses the default. Same for an out-of-range port.
- Any web page open in a browser could quit, reload or toggle the bar through
  `127.0.0.1:7776`. Browser requests are now refused (403) and the CORS header is gone.
- A settings file with a JSON syntax error was ignored silently; it is now reported.
- Battery and audio listeners leaked a run-loop source and accumulated listeners across
  output-device changes (e.g. Bluetooth reconnects).
- A refresh that arrived while a widget was running was dropped, so the bar could show a
  stale value until the next poll (e.g. after a workspace switch).
- Toggling the `process` widget over HTTP did nothing.
- Scroll events with no movement counted as "scroll up".
- `--snapshot` reported success even when nothing was saved.
- Popover arguments from the config were passed through a shell unquoted.

### Changed
- `wingbar status` shows each widget's last error (script failed, timed out, AeroSpace
  CLI or now-playing failure).
- `config.json`: wrongly typed values and unknown top-level keys are reported on stderr.
- An invalid `--port` exits with a message instead of being ignored.
- A user script named like a built-in widget (e.g. `memory`) now replaces it cleanly.

### Under the hood
- 108 tests and CI (fmt, clippy `-D warnings`, tests, release build) on macOS.
- Widgets live in `src/widgets/` behind one `Widget` trait and a registry; the UI,
  scheduler and server no longer know widgets by name.
- Config is read through typed, lenient serde structs.
- All C bindings live in `src/ffi/` behind safe wrappers; every `unsafe` block is documented.
- `src/ui/` is split into a pure, tested layout plus drawing, view, input and observers.
- `scripts/measure.sh` measures the bar's own cost apart from the widget scripts.

## 0.1.0

First version.
