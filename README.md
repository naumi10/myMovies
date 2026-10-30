# myMovies

A small desktop app for keeping track of a DVD / Blu-ray / 4K Ultra HD collection.
Titles are shown in a list on the left, with the selected movie's details on the
right. Collections exported from [DVD Profiler](https://www.invelos.com/) can be
imported directly from `Collection.xml`.

## Features

- Manual add and edit (title is the only required field).
- Add by barcode: scan a UPC in the Add dialog to fill details from Invelos.
- DVD, Blu-ray and 4K Ultra HD formats, including 3D editions.
- 3D discs show **BD 3D** (or the matching DVD/4K label) in the list and are
  labelled in the details pane; the editor has a 3D checkbox for manual and
  barcode-added titles.
- Front and back cover images in the details pane, click to enlarge.
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

## Adding a movie with a barcode scanner

Open **Add movie** (`Ctrl+N`). The barcode field is focused automatically, so a
USB barcode scanner can scan a disc and send Enter without any clicking. The
same field also accepts a UPC typed by hand.

The app runs the `invelos_lookup.py` helper in the background and fills the
title, format, year, runtime, rating, director, cast, synopsis and UPC from the
returned Invelos profile. Everything is still editable before saving. Any
high-quality front/back covers the helper can download are written to the cover
folder and shown for the new movie immediately.

The lookup script is looked for at `~/invelos_lookup/invelos_lookup.py`. These
environment variables override the defaults:

| Variable | Default | Purpose |
| --- | --- | --- |
| `MYMOVIES_LOOKUP_SCRIPT` | `~/invelos_lookup/invelos_lookup.py` | Lookup helper path |
| `MYMOVIES_PYTHON` | `python3` | Python interpreter |
| `MYMOVIES_LOCALITY` | `16` | Invelos locality (16 is Finland) |

If a barcode is not found, or the network is unavailable, the dialog shows the
error and leaves the form untouched so the movie can still be entered manually.

## Cover images

Cover images are looked up by UPC in `~/covers` (override with the
`MYMOVIES_COVERS` environment variable). The folder is expected to contain a
front and a back scan per disc:

```
~/covers/012236047605f.jpg   front
~/covers/012236047605b.jpg   back
```

Multi-disc releases may add a disc suffix before the `f`/`b`, for example
`044007867020.11f.jpg`. Covers downloaded by the barcode lookup are named
`044007867020.16_front.jpg` / `_back.jpg` and are matched the same way. Hyphens
and spaces in the UPC stored in the collection are ignored when matching, and
`.jpeg` is accepted as well as `.jpg`. When a UPC has no matching images the
covers section is simply omitted. Clicking a thumbnail opens a larger view; `Esc`
or **Close** dismisses it.

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
| `src/covers.rs` | Cover-image directory index and UPC matching |
| `src/import.rs` | DVD Profiler XML decoding and parsing |
| `src/lookup.rs` | Barcode lookup through `invelos_lookup.py` |
| `src/storage.rs` | JSON load/save and data path resolution |

## Tests

```sh
cargo test
cargo clippy --all-targets
```
