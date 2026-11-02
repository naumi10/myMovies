# myMovies

A small desktop app for keeping track of a DVD / Blu-ray / 4K Ultra HD collection.
Titles are shown in a list on the left, with the selected movie's details on the
right. Collections exported from [DVD Profiler](https://www.invelos.com/) can be
imported directly from `Collection.xml`.

## Features

- Manual add and edit (title is the only required field).
- Add by barcode: scan a UPC in the Add dialog to fill details from Invelos.
  If several countries share the barcode, choose the right locality before the
  profile is loaded.
- Locality (the release country in DVD Profiler) is imported from
  `Collection.xml` and filled by barcode lookups. It is shown in the details
  pane and can be edited with the other movie fields.
- The original title is imported from `Collection.xml` and filled by barcode
  lookups. It is shown as a details fact and can be edited in the form.
- Letterboxd ratings can be imported from a Letterboxd `.zip` export. The
  rating (0.5–5) is matched by title and year and shown as gold stars beside
  the movie title.
- **Update from OMDB** in the details pane fetches genres, credits, plot,
  country and the IMDb ID from OMDb, then lets you choose which values to save.
  If the movie has no year, the year from OMDb can be saved too. A stored IMDb
  ID is shown as a clickable IMDb link.
- DVD, Blu-ray, 3D Blu-ray and 4K Ultra HD logos in the collection list.
- 3D discs use the 3D logo in the list and are labelled in the details pane;
  use the **3D Blu-ray** filter to hide everything else. The editor has a 3D
  checkbox for manual and barcode-added titles.
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

## Importing Letterboxd ratings

Click **Import Letterboxd…** and choose a Letterboxd `.zip` export. The app reads
`ratings.csv` from the archive and matches each rating to a movie by title and
year; original titles are considered too. Matches are shown as gold stars
beside the movie title, including half stars. A rating that matches several
titles without a disambiguating year is skipped rather than guessed.

For scripted imports, the same operation is available from the command line:

```sh
myMovies --import-letterboxd /path/to/letterboxd-export.zip
```

## Updating from OMDb

Select a movie and click **Update from OMDB** in the details pane. The app looks
the title and year up in the [Open Movie Database](https://www.omdbapi.com/).

OMDb requires a free API key. The first update asks for it, stores it beside
the collection as `omdb_api_key` (for example
`~/.local/share/myMovies/omdb_api_key`) with owner-only permissions, and reuses
it on later updates. If the key is rejected, the dialog offers **Change API
key**.

When the response arrives, a comparison window lists the mapped fields:

| OMDb field | Movie field |
| --- | --- |
| `Year` | Year (only when the movie has no usable year) |
| `Genre` | Genres |
| `Director` | Directors |
| `Writer` | Writers |
| `Actors` | Actors |
| `Plot` | Synopsis |
| `Country` | Country |
| `imdbID` | IMDb ID |

Each field is shown as a clickable row with the OMDb value on the left and the
current detail beside it. Changed values are selected by default; fields whose
current detail is empty are selected when OMDb has a value, and empty OMDb
values are left unselected. Clicking a row toggles the green frame around the
OMDb side; the current detail never gets a green frame. **Select all** /
**Select none** adjust the whole list. **Update movie** saves only the selected
fields. Existing cast roles are kept when an actor appears in both lists.
Re-importing a DVD Profiler XML export also keeps the stored IMDb ID, which
appears in the details pane as a link such as
`https://www.imdb.com/title/tt0064116/`.

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

The app looks the profile up in Invelos in the background and fills the title,
format, year, runtime, rating, director, cast, synopsis and UPC from the
returned profile. Everything is still editable before saving. Any
high-quality front/back covers the server returns are written to the cover
folder and shown for the new movie immediately.

The lookup is built into the app; Python and a separate helper script are not
needed. A UPC can exist in several Invelos localities (for example the same
Nordic Blu-ray may have Swedish, Norwegian, Danish and Finnish profiles). In
that case the Add dialog lists each matching profile with its country, title and
edition, preselects the preferred locality, and waits for you to load one.

The preferred locality is configurable:

| Variable | Default | Purpose |
| --- | --- | --- |
| `MYMOVIES_LOCALITY` | `16` | Preferred Invelos locality (16 is Finland) |

The preference only chooses the initial selection in the locality list; you can
always pick another country for an individual barcode. Barcodes with a single
profile are loaded automatically.

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
| `src/letterboxd.rs` | Letterboxd ZIP/CSV import and rating matching |
| `src/lookup.rs` | Native Invelos barcode lookup |
| `src/omdb.rs` | OMDb lookup and response-to-movie field mapping |
| `src/storage.rs` | JSON load/save, data path and OMDb key storage |

## Tests

```sh
cargo test
cargo clippy --all-targets
```

Two ignored tests exercise the live Invelos service (network required):

```sh
cargo test -- --ignored
```

The OMDb test also needs a key:

```sh
OMDB_API_KEY=your-key cargo test -- --ignored omdb
```
