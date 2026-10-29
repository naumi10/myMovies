use std::fs;
use std::path::{Path, PathBuf};

use crate::model::Collection;

const APP_DIR: &str = "myMovies";
const DATA_FILE: &str = "movies.json";

/// Location of the collection database.
///
/// `MYMOVIES_DATA` overrides the path (used by the command line import and by
/// tests). Otherwise the usual XDG data directory is used.
pub fn data_path() -> PathBuf {
    if let Some(path) = std::env::var_os("MYMOVIES_DATA") {
        return PathBuf::from(path);
    }

    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share"))
        })
        .unwrap_or_else(|| PathBuf::from("."));

    base.join(APP_DIR).join(DATA_FILE)
}

pub fn load(path: &Path) -> Result<Collection, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Collection::default());
        }
        Err(error) => {
            return Err(format!("Could not read {}: {error}", path.display()));
        }
    };

    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))
}

/// Saves through a temporary file so an interrupted write cannot leave a
/// truncated collection behind.
pub fn save(path: &Path, collection: &Collection) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }

    let json = serde_json::to_vec_pretty(collection)
        .map_err(|error| format!("Could not serialize the collection: {error}"))?;

    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, json)
        .map_err(|error| format!("Could not write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("Could not replace {}: {error}", path.display()))?;
    Ok(())
}

/// A short, human friendly version of a path (uses `~` for the home folder).
pub fn display_path(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if let Ok(relative) = path.strip_prefix(&home) {
            return format!("~/{}", relative.display());
        }
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Movie;

    #[test]
    fn saves_and_loads_a_collection() {
        let directory = std::env::temp_dir().join(format!("mymovies-test-{}", std::process::id()));
        let path = directory.join("movies.json");

        let collection = Collection {
            version: 1,
            movies: vec![Movie {
                id: "test:1".to_string(),
                title: "Alien".to_string(),
                ..Default::default()
            }],
        };

        save(&path, &collection).expect("save");
        let loaded = load(&path).expect("load");
        assert_eq!(loaded.movies.len(), 1);
        assert_eq!(loaded.movies[0].title, "Alien");

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn missing_file_is_an_empty_collection() {
        let path = std::env::temp_dir().join("mymovies-does-not-exist.json");
        let collection = load(&path).expect("load");
        assert!(collection.movies.is_empty());
    }
}
