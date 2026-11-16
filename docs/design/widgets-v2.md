# wingbar 0.3: general widgets you can manage

Status: agreed design (2026-11-12). The source of truth for the 0.3 work.

## Goals

1. **General.** Nothing in the code is specific to one person's setup. No built-in
   default mentions ClickUp, Claude, prayer times, ADB or BarPopover. Those keep
   working for their owner through his imported settings.
2. **Easy to manage.** Show, hide, move, add and remove widgets from a right-click
   menu on the bar or with `wingbar widget …` commands. Changes apply at once,
   without restarting the bar.
3. **Own settings file.** `~/.config/wingbar/config.json` (format version 2) holds every
   widget and option. A simple-bar user's `.simplebarrc` is imported once; after
   that wingbar never reads it again.
4. **Built-in menus.** A widget script can print menu lines; wingbar shows them as a
   native menu. BarPopover stays supported, but is optional.
5. **No regression.** After importing the current owner's `.simplebarrc`, the bar looks
   and behaves exactly as 0.2.0 does with the same file (snapshot-identical).

Non-goals for 0.3: a settings window, themes other than the wings, web widgets.

## Settings file (version 2)

Path: `$WINGBAR_CONFIG` if set, else `~/.config/wingbar/config.json`.
JSON, read leniently (one bad value warns and falls back, never drops the file).
Keys starting with `//` are comments. Unknown keys warn. Writes keep key order and
unknown keys (serde_json `preserve_order`), and are atomic (temp file + rename).

```jsonc
{
  "version": 2,
  "windowManager": "auto",            // auto | aerospace | yabai | none
  "aerospacePath": "", "yabaiPath": "", // optional; found on PATH otherwise
  "bar": {
    "fontSize": null,                 // null = follow the screen
    "titleMaxWidth": 420,
    "windowLevel": "above",           // above | desktop
    "volumeStep": 5,
    "port": 7776, "server": true,
    "hideEmptySpaces": true, "spacesExclusions": [],
    "popoverBinary": "", "popoverDaemon": "127.0.0.1:7779"  // BarPopover, optional
  },
  "theme": {
    "wing": "#000000", "wingHover": "#161618", "text": "#f2f2f2",
    "dim": "#8e8e93", "faint": "#7c7c82", "warn": "#ff9f45", "alert": "#ff6b7a",
    "colors": { "green": "#9ed072", "blue": "#76cce0", "red": "#fc5d7c" }
  },
  "left":  ["spaces", "title"],
  "right": ["cpu", "memory", "battery", "sound", "date", "time"],
  "widgets": {
    "spaces":  { "type": "spaces" },
    "title":   { "type": "title" },
    "cpu":     { "type": "cpu", "every": "2s", "click": "open -a 'Activity Monitor'" },
    "memory":  { "type": "memory", "every": "4s", "click": "open -a 'Activity Monitor'" },
    "battery": { "type": "battery", "click": "caffeinate" },
    "sound":   { "type": "sound" },
    "date":    { "type": "date", "click": "open -a Calendar" },
    "time":    { "type": "time", "hour12": false, "seconds": false },
    "network": {
      "type": "script",
      "command": "bash ~/.config/wingbar/widgets/network.sh",
      "every": "10s",
      "icon": "network",
      "levels": { "ok": "green", "eth": { "color": "blue", "icon": "cable.connector" } },
      "click": "open 'x-apple.systempreferences:com.apple.Network-Settings.extension'"
    }
  }
}
```

### Placement and definitions

- `widgets` defines widgets by **key** (unique, `[a-z0-9_-]+`). `type` picks the
  implementation. The same type may appear under several keys (two clocks).
- `left` / `right` place widgets, in order. A defined widget that is in neither list
  is **hidden** (defined, not running, keeps its settings). A key in a list that has
  no definition warns and is skipped. A key in both lists warns; the first wins.
- Built-in types: `spaces`, `title`, `cpu`, `memory`, `battery`, `sound`, `date`,
  `time`, `spotify`, `music`, `script`. Their options are today's options, renamed to
  camelCase where needed (`every`, `click`, `hour12`, `seconds`, `caffeinateOption`, …).
- Durations: `"500ms"`, `"15s"`, `"2m"`, or a number of milliseconds. Minimum 500 ms.
- Colours anywhere: `#rgb`, `#rrggbb`, `#rrggbbaa`, a `theme` key (`warn`, `dim`, …)
  or a `theme.colors` name, with or without a leading `--`.
- Icons are SF Symbol names, or simple-bar icon names (capitalised: `ClickUp`,
  `MicOff`), which wingbar bundles (`src/icons/`, drawn from their SVG). Since
  0.3.1 the import keeps simple-bar's icon names and sets `theme.icons:
  "simple-bar"` (built-ins and the prayer rule use simple-bar's and its wings
  theme's icons; workspaces show app icons); `"sf"` is the default.

### Script widgets

Options: `command` (run with `/bin/bash -c`, fixed PATH, own process group),
`every`, `timeout` (default 30s), `icon`, `levels`, `click`, `rightClick`,
`middleClick`, `menuOn`, `popover`, `popoverTab`, `hideWhenEmpty` (default true), `iconOnly`,
`greyWhenOk`, `iconFromLabel`, `prelude`, `cacheFile`, `simplebarIds`, `raw`.

`raw: true` reads the output as it is, not with the protocol below: the label is
the first line of the trimmed output (a `|` in it is text), the level is "ok", and
no `icon:` or `menu:` lines are read (so there is no menu). The import sets it on
simple-bar's plain user widgets, which is how 0.2 showed them.

Output protocol (backward compatible with today's `level|label`):

```
warn|3 tasks            line 1: level|label, or just a label (level "ok")
icon: timer             optional: icon for this run (SF Symbol or bundled name)
menu: Stop timer | bash ~/scripts/clickup_toggle.sh
menu: Today: 2h 10m     a line with no command is shown, not clickable
menu: ---               separator
```

Other lines are ignored. At most 64 lines / 16 KB are read. An empty line 1 hides
the widget (unless `hideWhenEmpty: false`).

In a `menu:` line the first `|` ends the title; everything after it is the
command. A title therefore never contains a `|`, and a script that puts text it
doesn't control into a title (a device name, an SSID, a branch or folder name)
must remove `|`, tabs and other control characters from it first, or that text
could end the title early and become a command. The catalog scripts do.

Colour rules (exactly today's): levels `warn`, `alert`, `dim` colour icon and text
with `theme.warn` / `theme.alert` / `theme.faint`. Any other level colours the icon
with `levels[level]` (a colour string, or `{ "color", "icon" }`), falling back to
the first `levels` entry, then `theme.text`; text is `theme.text`. With
`greyWhenOk`, those other levels use `theme.dim` for the icon. Icon precedence:
`icon:` line, then the `iconFromLabel` rule when it matches the label, then
`levels[level].icon`, then `icon`.
`iconOnly` hides the label. `iconFromLabel: "prayer"` keeps the prayer-name→icon rule.
`prelude` is a file sourced before the command (the import uses it for `_env.sh`).
`cacheFile` writes line 1 to that file after each run (other tools read it).
`simplebarIds` keeps `/widget/user-widget/<action>/<n>` working for old hooks.

### Menus

- `menuOn`: `"click"` (default when the widget prints menu lines and has no `click`
  command), `"hover"` (opens after the pointer rests 300 ms), or `"none"`.
- `popover: true` sends hover to BarPopover (`bar.popoverBinary` / `popoverDaemon`)
  as 0.2 did; it wins over `menuOn: "hover"`.
- `popoverTab: true` (one widget): the bar reports that widget's place to BarPopover
  (`GET /tab?x&w&h&sw` on the daemon, from the main screen's bar, when it changes), so
  BarPopover's tab hangs under it. The import sets it on `salat` when it has a popover.
- A menu item runs its command detached (no shell for built-in actions; the item's
  command string goes through `/bin/bash -c` like click actions), then refreshes the
  widget after 500 ms.

## Managing widgets

### Right-click menu on the bar

Right-click anywhere on a wing opens the wingbar menu, except on a widget that has
its own `rightClick` command (that runs instead). ⌥-right-click or ⌃-click always
opens the wingbar menu.

```
CPU                                (header, when over a widget)
Move Left
Move Right
Move to Other Wing
Hide CPU
Remove CPU…                        (asks to confirm; script files are never deleted)
───────────
Widgets              ▸  ✓ Workspaces   (every defined widget; ✓ = shown; click toggles)
Add Widget           ▸  Built-in ▸ / Catalog ▸ / Custom Script…
───────────
Open Settings File                 (opens config.json in the default text editor)
Reload Settings
About wingbar 0.3.0                (disabled)
Quit wingbar
```

"Custom Script…" asks for a name and a command in a small native dialog.
Adding places the widget at the end of the wing the menu was opened on.

### Command line

```
wingbar widget list [--json]       key, type, wing, position, shown/hidden, last error
wingbar widget catalog             built-in types and catalog scripts that can be added
wingbar widget add <type|catalog-name> [--as KEY] [--left|--right]
                   [--before K|--after K|--at N] [--every DUR] [--command CMD]
wingbar widget remove <key>
wingbar widget show <key> [--left|--right] [--before K|--after K|--at N]
wingbar widget hide <key>
wingbar widget move <key> [--left|--right] [--before K|--after K|--at N]
wingbar widget set <key> <option> <value>      value: JSON, or a plain string
wingbar config path | check | edit
wingbar config import-simplebar [--from PATH] [--dry-run] [--force]
```

Commands edit the file whether or not the bar is running, then ask a running bar to
reload (`GET /reload`). Errors name the problem and the fix, and exit non-zero.

Both the menu and the CLI use one pure module of edit operations on the parsed
document (add, remove, show, hide, move, set), with tests.

### Applying changes

- `GET /reload`, `wingbar reload`, the menu, the CLI, and saving `config.json` by hand
  all hot-reload: re-read the file, rebuild the widget registry, replace the
  scheduler's tasks, clear state for widgets that went away, re-lay out the bars.
  No process restart, no blank bar.
- The settings directory is watched (kqueue on the directory, so editors that save by
  rename are seen), debounced by 200 ms.
- A job from before a reload never writes into the state after it (generation check).
- Changing `bar.port` or `bar.server` needs a restart; the reload says so.
- A file that fails to parse keeps the running settings and reports the error
  (stderr and `wingbar status`).

## Import from simple-bar

`wingbar config import-simplebar`, and automatically at startup when the settings
file is missing and `~/.simplebarrc` exists. It never modifies `.simplebarrc`.

- Built-ins on/off and options → built-in widget definitions.
- `themes.colorX` → `theme.colors`.
- `sbw.sh <name> <level> [ttl]` user widgets → one `script` widget per name:
  `command: bash <dir>/<name>.sh`, `prelude: <dir>/_env.sh` (if present),
  `every` = smallest interval, `levels` = each variant's level → its colour and icon,
  clicks from the first variant, `simplebarIds` = all variant ids,
  `cacheFile` = today's `simple-bar-widgets/<name>.out`.
- Other user widgets → `script` widgets keyed `user-<n>`, with `raw: true` (shown as 0.2 did).
- 0.2's per-name quirks become explicit options on the imported widgets:
  `iconFromLabel: "prayer"` for `salat`, `iconOnly: true` for `mic`, icon
  `memorychip` for `memory`, `greyWhenOk` for 0.2's grey-when-ok list, `popover: true`
  for 0.2's popover list, plus `bar.popoverBinary` when that binary exists.
- A 0.2 `config.json` (no `version`) is merged in (wing order, colours, sizes) and
  kept as `config.v1.json`.
- Prints what it wrote and any widget it could not convert.

Fresh install with neither file: a default config with spaces (if a window manager
is found), title, cpu, memory, battery, sound, date, time.

## Catalog

Scripts shipped inside the binary (`include_str!`), written to
`~/.config/wingbar/widgets/<name>.sh` when added (never overwriting an existing file).
Each is plain bash, needs no secrets, follows the output protocol and has a header
comment saying what it shows and needs. First set: `storage`, `network`, `mic`,
`adb`, `aerospace-mode`, `git-branch`. They are tested with fixtures.

## Testing and safety rules for this work

- Never write to the real `~/.config/wingbar/` or `~/.simplebarrc` during development:
  run the binary with `WINGBAR_CONFIG=<temp dir>/config.json`.
- Acceptance: import the owner's real `~/.simplebarrc` (read-only) into a temp
  config; a `--snapshot` of the bar with it must match 0.2.0's snapshot (only live
  values may differ).
