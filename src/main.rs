mod app;
mod import;
mod model;
mod storage;

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;
use model::Collection;

const HELP: &str = "\
myMovies — keep track of a DVD and Blu-ray collection

Usage:
  myMovies                         Start the desktop app
  myMovies --import <file.xml>     Import a DVD Profiler export and exit
  myMovies --data <file.json>      Use a different collection file
  myMovies --help                  Show this help

Environment:
  MYMOVIES_DATA                    Overrides the collection file location

The GUI can import XML with the Import button; --import is a convenience for
scripted or headless imports.
";

fn main() {
    if let Err(error) = run() {
        eprintln!("myMovies: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut import_path: Option<PathBuf> = None;
    let mut data_override: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--import" | "-i" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--import requires a path to an XML file".to_string())?;
                import_path = Some(PathBuf::from(value));
            }
            "--data" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--data requires a path to a JSON file".to_string())?;
                data_override = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                print!("{HELP}");
                return Ok(());
            }
            other => {
                return Err(format!("unknown argument “{other}”\n\n{HELP}"));
            }
        }
    }

    let data_path = data_override.unwrap_or_else(storage::data_path);

    if let Some(xml_path) = import_path {
        return run_import(&xml_path, &data_path);
    }

    let (collection, load_error) = load_or_recover(&data_path);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("myMovies")
            .with_inner_size([1240.0, 820.0])
            .with_min_inner_size([880.0, 560.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "myMovies",
        options,
        Box::new(move |cc| {
            Ok(Box::new(app::MyMoviesApp::new(
                cc, collection, data_path, load_error,
            )))
        }),
    )
    .map_err(|error| format!("could not start the window: {error}"))
}

fn run_import(xml_path: &Path, data_path: &Path) -> Result<(), String> {
    let bytes = std::fs::read(xml_path)
        .map_err(|error| format!("could not read {}: {error}", xml_path.display()))?;
    let parsed = import::parse_collection(&bytes)?;

    let mut collection = storage::load(data_path)?;
    let total = parsed.movies.len();
    let stats = collection.merge_imported(parsed.movies);
    storage::save(data_path, &collection)?;

    println!(
        "Imported {total} titles ({} added, {} updated, {} skipped) into {}",
        stats.added,
        stats.updated,
        parsed.skipped,
        storage::display_path(data_path)
    );
    Ok(())
}

/// Loads the collection. If the JSON file is unreadable it is moved aside so
/// the next save cannot silently overwrite a file the user may still want.
fn load_or_recover(path: &std::path::Path) -> (Collection, Option<String>) {
    match storage::load(path) {
        Ok(collection) => (collection, None),
        Err(error) => {
            if !path.exists() {
                return (Collection::default(), Some(error));
            }

            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let backup = path.with_extension(format!("json.invalid-{stamp}"));

            match std::fs::rename(path, &backup) {
                Ok(()) => (
                    Collection::default(),
                    Some(format!(
                        "{error} The unreadable file was kept as {}.",
                        storage::display_path(&backup)
                    )),
                ),
                Err(rename_error) => (
                    Collection::default(),
                    Some(format!(
                        "{error} It could not be moved aside either: {rename_error}"
                    )),
                ),
            }
        }
    }
}
