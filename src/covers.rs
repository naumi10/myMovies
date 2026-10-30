use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Front and back cover images for one movie.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Covers {
    pub front: Option<PathBuf>,
    pub back: Option<PathBuf>,
}

impl Covers {
    pub fn is_empty(&self) -> bool {
        self.front.is_none() && self.back.is_none()
    }
}

/// Maps UPCs to cover images by scanning a directory.
///
/// Files are named after the disc's UPC and suffixed with `f` or `b`, for
/// example `012236047605f.jpg` and `012236047605b.jpg`. Multi-disc releases may
/// add a disc suffix: `044007867020.11f.jpg`.
#[derive(Debug, Default)]
pub struct CoverIndex {
    by_upc: HashMap<String, Covers>,
}

impl CoverIndex {
    /// Scans `dir` for cover images. A missing or unreadable directory simply
    /// produces an empty index.
    pub fn scan(dir: &Path) -> Self {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Self::default();
        };

        let names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                entry
                    .file_type()
                    .ok()
                    .filter(|kind| kind.is_file())
                    .map(|_| entry.file_name().to_string_lossy().into_owned())
            })
            .collect();

        Self::from_file_names(dir, names)
    }

    /// Builds an index from file names located in `dir`. Kept separate from
    /// [`Self::scan`] so the matching rules can be tested without a filesystem.
    pub fn from_file_names(dir: &Path, names: impl IntoIterator<Item = String>) -> Self {
        let mut names: Vec<String> = names.into_iter().collect();
        names.sort();

        let mut by_upc: HashMap<String, Covers> = HashMap::new();
        for name in names {
            let Some((key, side)) = parse_cover_name(&name) else {
                continue;
            };
            let covers = by_upc.entry(key).or_default();
            let slot = match side {
                CoverSide::Front => &mut covers.front,
                CoverSide::Back => &mut covers.back,
            };
            if slot.is_none() {
                *slot = Some(dir.join(&name));
            }
        }

        Self { by_upc }
    }

    /// Looks up covers for a UPC such as `012236-047605`. Hyphens, spaces and
    /// other separators are ignored, and leading zeroes are tolerated.
    pub fn for_upc(&self, upc: &str) -> Option<&Covers> {
        self.candidates(upc)
            .find_map(|key| self.by_upc.get(&key))
            .filter(|covers| !covers.is_empty())
    }

    fn candidates<'a>(&self, upc: &'a str) -> impl Iterator<Item = String> + 'a {
        let digits: String = upc.chars().filter(char::is_ascii_digit).collect();
        let mut keys = Vec::with_capacity(4);
        if !digits.is_empty() {
            keys.push(digits.clone());
            let trimmed = digits.trim_start_matches('0');
            if !trimmed.is_empty() && trimmed != digits {
                keys.push(trimmed.to_string());
            }
            for width in [12, 13] {
                if digits.len() < width {
                    keys.push(format!("{digits:0>width$}"));
                }
            }
        }
        keys.into_iter()
    }
}

/// Directory that holds the cover images. `MYMOVIES_COVERS` wins; otherwise the
/// `covers` folder in the user's home directory is used.
pub fn directory() -> PathBuf {
    if let Some(dir) = std::env::var_os("MYMOVIES_COVERS") {
        return PathBuf::from(dir);
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .map(|home| home.join("covers"))
        .unwrap_or_else(|| PathBuf::from("covers"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoverSide {
    Front,
    Back,
}

/// Parses `012236047605f.jpg` or `044007867020.16_front.jpg` into its UPC key
/// and side. Returns `None` for files that are not JPEG covers.
fn parse_cover_name(name: &str) -> Option<(String, CoverSide)> {
    let lower = name.to_ascii_lowercase();
    let stem = lower
        .strip_suffix(".jpeg")
        .or_else(|| lower.strip_suffix(".jpg"))?;

    let (stem, side) = if let Some(base) = stem.strip_suffix("_front") {
        (base, CoverSide::Front)
    } else if let Some(base) = stem.strip_suffix("_back") {
        (base, CoverSide::Back)
    } else {
        match stem.as_bytes().last()? {
            b'f' => (&stem[..stem.len() - 1], CoverSide::Front),
            b'b' => (&stem[..stem.len() - 1], CoverSide::Back),
            _ => return None,
        }
    };

    // Drop an optional locality or disc suffix such as `.16` in
    // `044007867020.16f.jpg` or `044007867020.11f.jpg`.
    let base = stem.split('.').next().unwrap_or(stem);
    let digits: String = base.chars().filter(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }

    Some((digits, side))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(names: &[&str]) -> CoverIndex {
        CoverIndex::from_file_names(
            Path::new("/covers"),
            names.iter().map(|name| name.to_string()),
        )
    }

    #[test]
    fn matches_hyphenated_and_plain_upcs() {
        let index = index(&["012236047605f.jpg", "012236047605b.jpg"]);
        let covers = index.for_upc("012236-047605").unwrap();
        assert_eq!(
            covers.front.as_deref(),
            Some(Path::new("/covers/012236047605f.jpg"))
        );
        assert_eq!(
            covers.back.as_deref(),
            Some(Path::new("/covers/012236047605b.jpg"))
        );
    }

    #[test]
    fn ignores_disc_suffix_when_matching() {
        let index = index(&["044007867020.11f.jpg", "044007867020.16b.jpg"]);
        let covers = index.for_upc("044007-867020").unwrap();
        assert!(covers.front.is_some());
        assert!(covers.back.is_some());
    }

    #[test]
    fn tolerates_leading_zeroes_and_case() {
        let index = index(&["012569505322F.JPG", "012569505322B.JPEG"]);
        let covers = index.for_upc("12569505322").unwrap();
        assert!(covers.front.is_some());
        assert!(covers.back.is_some());
    }

    #[test]
    fn matches_lookup_script_cover_names() {
        let index = index(&["044007867020.16_front.jpg", "044007867020.16_back.jpg"]);
        let covers = index.for_upc("044007-867020").unwrap();
        assert_eq!(
            covers.front.as_deref(),
            Some(Path::new("/covers/044007867020.16_front.jpg"))
        );
        assert_eq!(
            covers.back.as_deref(),
            Some(Path::new("/covers/044007867020.16_back.jpg"))
        );
    }

    #[test]
    fn ignores_unrelated_files_and_unknown_upcs() {
        let index = index(&["readme.txt", "cover.jpg", "012236047605.jpg", "abc.jpg"]);
        assert!(index.for_upc("012236-047605").is_none());
        assert!(index.for_upc("999999999999").is_none());
        assert!(index.for_upc("").is_none());
    }

    #[test]
    fn prefers_the_first_disc_variant() {
        let index = index(&["044007867020.11f.jpg", "044007867020.16f.jpg"]);
        let covers = index.for_upc("044007867020").unwrap();
        assert_eq!(
            covers.front.as_deref(),
            Some(Path::new("/covers/044007867020.11f.jpg"))
        );
    }
}
