# myMovies

A small desktop app for keeping track of a DVD / Blu-ray / 4K Ultra HD collection.
Titles are shown in a list on the left, with the selected movie's details on the
right. Collections exported from [DVD Profiler](https://www.invelos.com/) can be
imported directly from `Collection.xml`.

## Features

- Manual add and edit (title is the only required field).
- DVD, Blu-ray and 4K Ultra HD formats.
- Import DVD Profiler XML exports (Windows-1252 or UTF-8 encoded).
- Re-importing updates existing titles and keeps your personal notes.
- Search across title, cast, director, rating, year and UPC.
- Format filters, sortable title list, delete with confirmation.
- Drag an XML file onto the window to import it.
- Everything is stored in one plain JSON file.

## Build and run

Rust 1.85 or newer is required (the project uses edition 2024). With the
[rustup](https://rustup.rs/) toolchain installed:

```sh
cargo run
```

To run without a terminal attached:

```sh
cargo build --release
./target/release/myMovies
```

The GUI uses `eframe`/`egui` with the OpenGL (glow) renderer, so no GTK
development packages are needed.

## Importing a collection

From the app, click **Import XML…** and choose the `Collection.xml` file, or drop
the file onto the window. For scripted or headless imports there is a CLI mode:

```sh
myMovies --import /path/to/Collection.xml
```

This merges the export into the current collection and exits:

```
Imported 1547 titles (1547 added, 0 updated, 0 skipped) into ~/.local/share/myMovies/movies.json
```

Imports are matched by a stable id: the DVD Profiler profile id when available,
otherwise the UPC, otherwise title and year. Personal notes on an existing entry
are kept when the same profile is imported again.

## Where the data lives

```
~/.local/share/myMovies/movies.json
```

Set the `MYMOVIES_DATA` environment variable or pass `--data <file.json>` to use
a different file:

```sh
MYMOVIES_DATA=/tmp/test.json cargo run
myMovies --import Collection.xml --data /tmp/test.json
```

The JSON file is written atomically. If it cannot be decoded at startup, it is
renamed to `movies.json.invalid-<timestamp>` instead of being overwritten, and
the app starts with an empty collection.

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| `Ctrl+N` | Add a movie |
| `Ctrl+F` | Focus the search field |

## Project layout

| File | Purpose |
| --- | --- |
| `src/main.rs` | Entry point, CLI arguments, startup and corrupt-store recovery |
| `src/app.rs` | egui interface, dialogs and app state |
| `src/model.rs` | `Movie`, `Collection`, filtering, sorting, import merge |
| `src/import.rs` | DVD Profiler XML decoding and parsing |
| `src/storage.rs` | JSON load/save and data path resolution |

## Tests

```sh
cargo test
cargo clippy --all-targets
```
