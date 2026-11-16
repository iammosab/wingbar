# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

**wingbar** is a general native macOS status bar in Rust (AppKit via `objc2`). It draws two "wings" at the top of each screen, runs built-in and script widgets, and lets people manage widgets from a right-click menu or `wingbar widget …`. It started as a replacement for Übersicht + simple-bar and still imports a `.simplebarrc` once. Efficiency is the main design goal. The 0.3 design is `docs/design/widgets-v2.md`; read it before changing settings, widgets, menus or the CLI.

## Commands

```bash
cargo build --release            # build the bar
cargo test                       # all tests; one test: cargo test <name substring>
cargo fmt --check && cargo clippy --all-targets -- -D warnings   # the CI gate (.github/workflows/ci.yml)
WINGBAR_CONFIG=$(mktemp -d)/config.json cargo run -- --no-server --quit-after 10   # run on throwaway settings
WINGBAR_CONFIG=... cargo run -- --no-server --snapshot <dir>   # save each bar as bar-N.png after 5 s, exit
WINGBAR_CONFIG=... cargo run -- widget list                     # the CLI (widget …, config …; --help on each)
scripts/measure.sh [--isolated] [secs]   # measure the bar's own CPU/memory apart from widget scripts
./install.sh [--download] / ./uninstall.sh   # build (or download the release) to ~/Applications/Wingbar.app + LaunchAgent / remove and restore Übersicht
git tag vX.Y.Z && git push --tags   # release: .github/workflows/release.yml builds the universal app (scripts/bundle.sh), publishes it, updates iammosab/homebrew-tap
```

**Settings safety:** the settings file is `$WINGBAR_CONFIG`, else `~/.config/wingbar/config.json`, and the bar *writes* it on first start (importing `~/.simplebarrc` if present). When developing on a machine with a real setup, always set `WINGBAR_CONFIG` to a temp path and pass `--no-server` (or a spare `--port`) so a running installed bar on 7776 is never touched. Never write `~/.simplebarrc`.

Tests are in-module `#[cfg(test)]` blocks (some modules keep them in a `tests.rs` child); they must not need a GUI, the user's home, AeroSpace or the network (fixtures in `tests/fixtures/`; catalog scripts are tested against fake commands via `WINGBAR_<TOOL>` overrides). Tests can't create AppKit menus (not on the main thread), so menu contents are pure models (`ui/bar_menu.rs` `model`, `ui/menu.rs` descriptions) and the AppKit layer stays thin; `WINGBAR_DEBUG_MENUS=manage|fire` with `--snapshot` prints/fires real menus. For UI changes compare `--snapshot` images before and after (an empty workspaces segment usually means the `aerospace` CLI was slow). For performance claims compare `scripts/measure.sh --bin <old>` against the new build using instructions/s (CPU % is noisy). Logs of an installed bar go to `/tmp/wingbar.log`. Keep `install.sh` and `uninstall.sh` symmetric.

## Architecture (wingbar)

Data flow: **signals (macOS events, scheduler timers, HTTP, settings watcher) → widget jobs → `state` (compare; only real changes) → one coalesced redraw on the main thread → only the wing that changed is redrawn.**

- `main.rs` / `cli.rs`: argument parsing (hand-written, pure, tested). Subcommands either talk to a running bar over HTTP (`status`, `reload`, `refresh`, `quit`) or edit settings (`widget …`, `config …`). After an edit the CLI reloads a running bar only if its `/widgets` says it uses the same settings file. Without a subcommand: NSApplication setup, settings load, bars, observers, scheduler, server, watcher, run loop.
- `config/`: the version 2 settings. `model.rs` (types: `Config`, `WidgetDef`, `Options` per `WidgetType`, `ScriptOptions`, `Theme`, `Bar`), `parse.rs` (lenient: one bad value warns and falls back, unknown keys warn), `doc.rs` (the file as an editable JSON document; atomic, order-preserving writes), `edit.rs` (pure add/remove/show/hide/move/set on the document), `import.rs` (`.simplebarrc` → v2; sbw.sh variants become one script widget with per-level colour/icon; 0.2's personal quirks become explicit options; the *only* place person-specific names may appear), `defaults.rs` (fresh install), `mod.rs` (`settings_path()`, `current()`/`set_current()` swappable `Arc`, first-run import/write, `ensure_settings` for the CLI). `left`/`right` place widgets; defined but unplaced = hidden (doesn't run).
- `widgets/`: the `Widget` trait (`key`, `schedule`, `poll -> Result<Show, Failure>`, `kind`, `aliases`, `user_widgets`, `clickable`, `click`, `scroll`, menus) and the `Registry`, built from the placed definitions in wing order and swapped on reload (`widgets::current()`). Built-in types: `system.rs` (cpu, memory, battery, sound), `clock.rs`, `media.rs` (Spotify/Music, pushed), `window.rs` (title), `spaces.rs`. `script.rs` is the script widget and its output protocol (`level|label`, `icon:`, `menu:` lines; 64 lines / 16 KB cap). The task wrapper (not the widget) records errors and applies results, checked against the state's generation.
- `reload.rs`: the one hot reload used by `/reload`, the menu, the CLI and the kqueue watcher on the settings folder (200 ms debounce): parse → new registry (unchanged widgets kept) → bump generation and clear state of removed keys → `set_current` → `sched::replace` → `ui::apply_settings` on the main thread. A file that can't be used keeps the running settings and reports why. `bar.port`/`bar.server` need a restart.
- `manage.rs`: the high-level operations behind both the right-click menu and `wingbar widget …` (load document → pure edit → atomic save → human message), so the two never disagree.
- `catalog.rs` + `catalog/*.sh`: ready-made script widgets embedded with `include_str!`, installed to `<settings dir>/widgets/` without ever overwriting.
- `icons/`: simple-bar's icon library and app → icon map, generated into `simple_bar.rs` by `scripts/simplebar-icons.py` (MIT, `SIMPLE-BAR-LICENSE`), plus `extra.rs`, the "wings" theme's line-art SVGs (whole documents drawn in `black`, tinted by replacing it). A capitalised icon name is bundled (drawn from SVG by `ui/draw.rs`), any other is an SF Symbol. `theme.icons` (`sf` | `simple-bar`) picks the built-ins' icons and the prayer rule's, and turns on app icons in the workspaces (`wm.rs` reads each workspace's apps; at most 3, none for apps without an icon).
- `sched.rs`: one scheduler thread sleeps until the next task is due or a refresh arrives; each run gets a short-lived thread (measured cheaper and safer than a pool), never overlapping itself; a refresh mid-run re-runs once after. `run_program*` use a fixed `PATH`, a process group, a timeout that kills the group, and an optional output cap. `replace` swaps the task set on reload (a kept slot that is running re-runs once with its new task).
- `state.rs`: global `Mutex<State>` (items, spaces, title, disabled, errors, and a generation per key: a reload invalidates only removed, added or remade keys, so kept widgets' in-flight results still apply). Setters compare and only request a redraw on a real change, coalesced onto the main queue via `dispatch2`. Errors never cause redraws.
- `ui/`: one full-width borderless panel per screen whose content view draws nothing; two child `WingView`s, each only as wide as its wing, draw the segments (a screen-wide drawn surface made Core Animation upload the whole backing store on every change: keep drawn surfaces wing-sized). `layout.rs` is pure and tested; `draw.rs` copies state, releases the lock, lays out once, redraws only changed wings; `input.rs` routes clicks/scrolls/hover to the registry, script menus (click or 300 ms hover) and BarPopover (and, while a panel it asked for is open, reads BarPopover's `barpopover.pid` every 100 ms so the wing corner above a flush panel squares: `state.panel` → `layout::square_corners`); `menu.rs` builds NSMenus from plain descriptions with Rust closures (`pop_up` only from event handlers); `dropdown.rs` draws a widget's menu (the same `Menu` description) as a themed panel joined to the wing (`theme.menus: "wings"`, default; pure model + thin AppKit, a 100 ms timer and a click monitor only while open); `bar_menu.rs` is the right-click management menu; `observers.rs` installs NSWorkspace/screen/wake/now-playing observers. The UI knows no widget by name.
- `sys.rs` / `ffi/`: in-process readings (Mach CPU/memory, IOKit battery, CoreAudio volume, Accessibility title, libproc, strftime) on safe wrappers over every C binding in `ffi/` (CF via `objc2-core-foundation`, kqueue, etc.). Every `unsafe` block needs a `// SAFETY:` comment (`clippy::undocumented_unsafe_blocks`, crate-wide).
- `wm.rs`: workspaces from the `aerospace`/`yabai` CLIs, plus focus/cycle.
- `server.rs`: HTTP on 127.0.0.1 only, simple-bar-server-compatible routes (`/aerospace/spaces/refresh`, `/yabai/...`, `/widget/<name>/(refresh|toggle|enable|disable)`, `/widget/user-widget/<action>/<n>` via `simplebarIds`) plus `/status`, `/widgets` (JSON), `/reload`, `/quit`. Requests with `Origin`/`Sec-Fetch-*` headers are refused (403) and stalled headers get 408, so web pages can't drive the bar; keep it that way. Existing AeroSpace hooks depend on these routes.

### Conventions that matter

- Prefer events pushed by macOS over polling, and in-process APIs over shelling out. Most of the code exists to avoid both, and the README's "Why it's light" section lists the guarantees.
- Only touch AppKit on the main thread. Workers write to `state` and never call `ui` directly.
- Keep new threads small (the existing ones use 64–256 KB stacks).
- Nothing person-specific in code outside `config/import.rs`: personal behaviour is expressed as settings options (`iconFromLabel`, `iconOnly`, `greyWhenOk`, `popover`, …).
- The local server rejects browser requests (any `Origin`/`Sec-Fetch-*` header) so web pages can't drive the bar; keep it that way.
- Rust edition 2024, MSRV 1.88 (let-chains). Dependencies are deliberately minimal: `objc2*` (including `objc2-core-foundation`), `dispatch2`, `block2`, `libc`, `serde`, `serde_json` (with `preserve_order`).
