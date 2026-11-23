# MacCleaner

A native macOS utility written in Rust that frees disk space, manages startup items and
background processes, and shows where your memory goes.

## Features

### Storage
- Scans caches, logs, Xcode build data and simulators, developer package caches (npm, pnpm,
  Bun, Cargo, Gradle, Maven, Dart, CocoaPods, `~/.cache`), the Trash, Downloads, iPhone/iPad
  backups, iOS firmware files, Mail attachments and saved application state.
- **Add Folder…** reviews the contents of any folder, such as a projects folder or an external drive.
- **Folder view before deletion**: expand any item to browse its subfolders and files, with
  sizes, before you decide. Double-click a name or use the open icon to reveal it in Finder.
- Select whole categories, single folders, or individual files deep inside a folder. Partially
  selected folders show a dash.
- Shows the size of each folder and category, plus a running **To be freed** total for the selection.
- **Select Safe** picks only data that apps re-create automatically. **Select All** and
  **Deselect All** do what they say.
- Deletes by **moving to the Trash** (recoverable) or **permanently**. A confirmation lists every
  item and size first, and flags items marked *Review* (personal files, backups).
- Sizes count real disk usage: allocated blocks, hard links counted once, without crossing
  onto other volumes.

### Startup Items
- Lists login items, your launch agents (`~/Library/LaunchAgents`), launch agents for all users
  (`/Library/LaunchAgents`) and system launch daemons (`/Library/LaunchDaemons`).
- Shows whether each item is running (with its PID), runs at login, or is kept alive by launchd.
- Turn items off or on with a switch. Launch agents use `launchctl disable/enable` and also
  stop or start right away. Daemons ask for an administrator password. Login items you remove
  can be added back during the same session.

### Background Processes
- Lists background helpers, agents and menu bar apps, grouped by the app they belong to, with
  memory and CPU.
- **Quit** asks an app to quit, like the Dock does. **Force Quit** kills it immediately.
- Processes started by launchd are marked with a restart icon, since they may start again. Turn them off in
  Startup Items to keep them off.
- macOS components and other users' processes are protected and can't be quit.

### Memory
- Total RAM, memory used, cached files, swap, and memory pressure, computed the way Activity
  Monitor does.
- A breakdown into app, wired, compressed, cached and free memory, plus a 5-minute usage chart.
- **Memory that can be freed**: cached/purgeable memory plus what quittable background
  processes use.
- **Free Up RAM…** runs macOS's `purge` (asks for an administrator password) and reports how
  much was freed.
- Per-app memory usage (physical footprint), with helpers grouped under their app. Expand an
  app to see each of its processes.

## Requirements

- macOS 11 or later (Apple Silicon or Intel)
- Rust 1.88 or later (edition 2024)

## Build and run

```sh
cargo run --release            # run directly
./scripts/bundle.sh            # build target/release/bundle/MacCleaner.app
open target/release/bundle/MacCleaner.app
```

Copy `MacCleaner.app` to `/Applications` if you want to keep it.

### App icon

The icon's source is `assets/icon.svg`. After editing it, regenerate the bundle icon and the
Dock icon embedded in the binary:

```sh
./scripts/make_icon.sh   # writes assets/AppIcon.icns and assets/icon-256.rgba
```

This builds the small renderer in `tools/icon-render` (based on `resvg`), so the app itself
doesn't depend on an SVG library.

## Permissions

| Permission | Why | Where |
|---|---|---|
| Full Disk Access | Measure the Trash, Mail attachments and device backups | System Settings › Privacy & Security › Full Disk Access |
| Automation → System Events | List, remove and restore login items | Asked automatically the first time |
| Administrator password | Change system launch daemons, run `purge` | Asked each time |

Without Full Disk Access the scan still works. Unreadable folders are reported, and the app
links to the right settings pane.

## Safety

- Nothing is deleted without a confirmation that lists every item.
- System locations (`/System`, `/usr`, `/Library`, …), your home folder and its top-level
  folders, folders directly inside `~/Library`, Keychains, Mail, Messages and iCloud Drive are
  never deleted. You can still open a top-level folder and pick items inside it.
- Right before deleting, every path is checked again: it must be inside one of the scanned
  folders.
- Symbolic links are removed, never followed.

## Project layout

```
src/
  main.rs              window setup
  app.rs               sidebar navigation and tabs
  util.rs              formatting, shell/AppleScript helpers
  storage/             scan locations, sizing, selection tree, deletion, safety rules
  startup/             launch agents/daemons, login items, launchctl parsing
  system/              memory statistics, processes, background monitor thread
  ui/                  one view per tab, shared widgets
assets/icon.svg        app icon source (AppIcon.icns and icon-256.rgba are generated)
tools/icon-render/     SVG → .iconset renderer used by scripts/make_icon.sh
scripts/bundle.sh      .app packaging
scripts/make_icon.sh   icon regeneration
```

## Credits

Icons are [Material Symbols](https://fonts.google.com/icons) by Google (Apache License 2.0),
bundled through the [`egui_material_icons`](https://crates.io/crates/egui_material_icons) crate.

## Tests

```sh
cargo test
```

The tests cover size formatting, the safety rules, selection logic, scanning (including hard
links), deletion (including read-only folders and paths outside the scanned folders),
launchctl/plist/login item parsing, the memory statistics FFI, and process grouping.
