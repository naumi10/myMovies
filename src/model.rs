use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Builds a DVD Profiler-style sort title by moving a leading English article
/// (`The`, `A`, `An`) to the end: `The English Patient` becomes
/// `English Patient, The`. Titles without a leading whole-word article are
/// returned unchanged.
pub fn sort_title_for(title: &str) -> String {
    let title = title.trim();
    for article in ["The", "A", "An"] {
        let Some(rest) = strip_article(title, article) else {
            continue;
        };
        let rest = rest.trim_start();
        if rest.is_empty() {
            return title.to_string();
        }
        return format!("{rest}, {}", &title[..article.len()]);
    }
    title.to_string()
}

fn strip_article<'a>(title: &'a str, article: &str) -> Option<&'a str> {
    let prefix = title.get(..article.len())?;
    if !prefix.eq_ignore_ascii_case(article) {
        return None;
    }
    let rest = &title[article.len()..];
    rest.starts_with(char::is_whitespace).then_some(rest)
}

/// The physical media type of a movie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DiscFormat {
    #[default]
    Dvd,
    BluRay,
    UltraHd,
}

impl DiscFormat {
    pub const ALL: [Self; 3] = [Self::Dvd, Self::BluRay, Self::UltraHd];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dvd => "DVD",
            Self::BluRay => "Blu-ray",
            Self::UltraHd => "4K Ultra HD",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Dvd => "DVD",
            Self::BluRay => "BD",
            Self::UltraHd => "4K",
        }
    }
}

impl std::fmt::Display for DiscFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Actor {
    pub name: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub voice: bool,
    #[serde(default)]
    pub uncredited: bool,
}

impl Actor {
    pub fn new(name: impl Into<String>, role: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            role: role.into(),
            voice: false,
            uncredited: false,
        }
    }
}

/// A single title in the collection.
///
/// Every field has a serde default so older or hand-edited JSON files keep
/// loading when the model grows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Movie {
    /// Stable identifier. Imported movies use the DVD Profiler profile ID,
    /// manually added movies use a generated `manual:...` id.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub original_title: String,
    #[serde(default)]
    pub sort_title: String,
    #[serde(default)]
    pub format: DiscFormat,
    /// The disc is a 3D edition (stereoscopic Blu-ray or anaglyph DVD).
    #[serde(default)]
    pub is_3d: bool,
    #[serde(default)]
    pub year: Option<u16>,
    #[serde(default)]
    pub released: String,
    #[serde(default)]
    pub runtime_minutes: Option<u32>,
    #[serde(default)]
    pub rating: String,
    #[serde(default)]
    pub rating_details: String,
    /// Letterboxd rating (0.5–5) imported from a Letterboxd export.
    #[serde(default)]
    pub letterboxd_rating: Option<f32>,
    /// IMDb title ID from OMDb, for example `tt0102926`.
    #[serde(default)]
    pub imdb_id: String,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub studios: Vec<String>,
    #[serde(default)]
    pub media_companies: Vec<String>,
    #[serde(default)]
    pub regions: Vec<String>,
    #[serde(default)]
    pub country: String,
    /// Invelos locality/country whose profile this release belongs to.
    #[serde(default)]
    pub locality: String,
    #[serde(default)]
    pub overview: String,
    #[serde(default)]
    pub actors: Vec<Actor>,
    #[serde(default)]
    pub directors: Vec<String>,
    #[serde(default)]
    pub writers: Vec<String>,
    #[serde(default)]
    pub audio: Vec<String>,
    #[serde(default)]
    pub subtitles: Vec<String>,
    #[serde(default)]
    pub disc_count: u32,
    #[serde(default)]
    pub upc: String,
    #[serde(default)]
    pub collection_number: String,
    #[serde(default)]
    pub case_type: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub purchase_date: String,
    #[serde(default)]
    pub purchase_price: String,
    #[serde(default)]
    pub purchase_place: String,
    #[serde(default)]
    pub notes: String,
}

impl Movie {
    /// Key used for sorting the collection list.
    pub fn sort_key(&self) -> String {
        let base = if self.sort_title.trim().is_empty() {
            self.title.as_str()
        } else {
            self.sort_title.as_str()
        };
        base.trim().to_lowercase()
    }

    pub fn year_label(&self) -> String {
        self.year.map(|y| y.to_string()).unwrap_or_default()
    }

    pub fn runtime_label(&self) -> String {
        match self.runtime_minutes {
            Some(minutes) if minutes >= 60 => {
                let (hours, rest) = (minutes / 60, minutes % 60);
                if rest == 0 {
                    format!("{hours} h")
                } else {
                    format!("{hours} h {rest} min")
                }
            }
            Some(minutes) => format!("{minutes} min"),
            None => String::new(),
        }
    }

    /// Full format label, including the 3D marker for 3D editions.
    pub fn format_label(&self) -> String {
        if self.is_3d {
            format!("{} 3D", self.format.label())
        } else {
            self.format.label().to_string()
        }
    }

    /// Compact label used by the format badge in the list and details header.
    pub fn format_badge_label(&self) -> String {
        if self.is_3d {
            format!("{} 3D", self.format.short_label())
        } else {
            self.format.short_label().to_string()
        }
    }

    /// One-line summary used under the title in the list.
    pub fn list_subtitle(&self) -> String {
        let mut parts = Vec::new();
        if let Some(year) = self.year {
            parts.push(year.to_string());
        }
        let runtime = self.runtime_label();
        if !runtime.is_empty() {
            parts.push(runtime);
        }
        if !self.rating.trim().is_empty() {
            parts.push(self.rating.clone());
        }
        parts.join("  ·  ")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collection {
    #[serde(default = "current_version")]
    pub version: u32,
    #[serde(default)]
    pub movies: Vec<Movie>,
}

fn current_version() -> u32 {
    1
}

impl Default for Collection {
    fn default() -> Self {
        Self {
            version: current_version(),
            movies: Vec::new(),
        }
    }
}

impl Collection {
    pub fn find(&self, id: &str) -> Option<&Movie> {
        self.movies.iter().find(|movie| movie.id == id)
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut Movie> {
        self.movies.iter_mut().find(|movie| movie.id == id)
    }

    pub fn remove(&mut self, id: &str) -> Option<Movie> {
        let index = self.movies.iter().position(|movie| movie.id == id)?;
        Some(self.movies.remove(index))
    }

    /// Adds a movie, or replaces an existing one with the same id.
    /// Returns true when an existing entry was updated.
    pub fn upsert(&mut self, movie: Movie) -> bool {
        if let Some(existing) = self.find_mut(&movie.id) {
            *existing = movie;
            true
        } else {
            self.movies.push(movie);
            false
        }
    }

    pub fn sort(&mut self) {
        self.movies.sort_by_key(Movie::sort_key);
    }

    /// Merges freshly imported movies into the collection.
    ///
    /// Titles are matched by their stable `id`. Personal notes on an existing
    /// entry survive a re-import of the same DVD Profiler profile.
    pub fn merge_imported(&mut self, movies: Vec<Movie>) -> MergeStats {
        let mut stats = MergeStats::default();
        for movie in movies {
            if let Some(existing) = self.find_mut(&movie.id) {
                let notes = std::mem::take(&mut existing.notes);
                let letterboxd_rating = existing.letterboxd_rating.take();
                let imdb_id = std::mem::take(&mut existing.imdb_id);
                *existing = movie;
                if existing.notes.is_empty() {
                    existing.notes = notes;
                }
                if existing.letterboxd_rating.is_none() {
                    existing.letterboxd_rating = letterboxd_rating;
                }
                if existing.imdb_id.is_empty() {
                    existing.imdb_id = imdb_id;
                }
                stats.updated += 1;
            } else {
                self.movies.push(movie);
                stats.added += 1;
            }
        }
        self.sort();
        stats
    }

    /// Indices of the movies matching the search text, format filter and 3D
    /// filter.
    pub fn filter_indices(
        &self,
        query: &str,
        format: Option<DiscFormat>,
        only_3d: bool,
    ) -> Vec<usize> {
        let query = query.trim().to_lowercase();
        self.movies
            .iter()
            .enumerate()
            .filter(|(_, movie)| {
                if only_3d && !movie.is_3d {
                    return false;
                }
                if let Some(format) = format
                    && movie.format != format
                {
                    return false;
                }
                if query.is_empty() {
                    return true;
                }

                if movie.title.to_lowercase().contains(&query)
                    || movie.sort_title.to_lowercase().contains(&query)
                    || movie.rating.to_lowercase().contains(&query)
                    || movie.upc.to_lowercase().contains(&query)
                    || movie
                        .year
                        .is_some_and(|year| year.to_string().contains(&query))
                {
                    return true;
                }

                movie
                    .actors
                    .iter()
                    .any(|actor| actor.name.to_lowercase().contains(&query))
                    || movie
                        .directors
                        .iter()
                        .any(|director| director.to_lowercase().contains(&query))
            })
            .map(|(index, _)| index)
            .collect()
    }
}

/// Generates an id for a manually added movie.
pub fn new_manual_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("manual:{nanos}-{sequence}")
}

/// Splits a comma or semicolon separated list, trimming and de-duplicating.
pub fn split_list(input: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for part in input.split([',', ';']) {
        let part = part.trim();
        if !part.is_empty() && !result.iter().any(|seen| seen.eq_ignore_ascii_case(part)) {
            result.push(part.to_string());
        }
    }
    result
}

/// Splits a newline or semicolon separated list. Newlines are used for names
/// (studios, directors) because those can legitimately contain commas.
pub fn split_multiline(input: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for part in input.split(['\n', ';']) {
        let part = part.trim();
        if !part.is_empty() && !result.iter().any(|seen| seen.eq_ignore_ascii_case(part)) {
            result.push(part.to_string());
        }
    }
    result
}

pub fn join_list(items: &[String]) -> String {
    items.join(", ")
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MergeStats {
    pub added: usize,
    pub updated: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie(id: &str, title: &str, format: DiscFormat, year: u16) -> Movie {
        Movie {
            id: id.to_string(),
            title: title.to_string(),
            format,
            year: Some(year),
            ..Default::default()
        }
    }

    #[test]
    fn builds_sort_titles_from_leading_articles() {
        assert_eq!(
            sort_title_for("The English Patient"),
            "English Patient, The"
        );
        assert_eq!(sort_title_for("A Clockwork Orange"), "Clockwork Orange, A");
        assert_eq!(sort_title_for("An Education"), "Education, An");
        assert_eq!(sort_title_for("the thing"), "thing, the");
        assert_eq!(
            sort_title_for("English Patient, The"),
            "English Patient, The"
        );
        assert_eq!(sort_title_for("The"), "The");
        assert_eq!(sort_title_for("Theatre of Blood"), "Theatre of Blood");
        assert_eq!(sort_title_for("Annie Hall"), "Annie Hall");
    }

    #[test]
    fn sorts_by_sort_title() {
        let mut collection = Collection {
            version: 1,
            movies: vec![
                movie("1", "The Thing", DiscFormat::Dvd, 1982),
                movie("2", "Alien", DiscFormat::BluRay, 1979),
            ],
        };
        collection.movies[0].sort_title = "Thing, The".to_string();
        collection.sort();
        assert_eq!(collection.movies[0].title, "Alien");
        assert_eq!(collection.movies[1].title, "The Thing");
    }

    #[test]
    fn filters_by_title_actor_year_and_format() {
        let collection = Collection {
            version: 1,
            movies: vec![
                Movie {
                    id: "1".to_string(),
                    title: "Bad Lieutenant".to_string(),
                    format: DiscFormat::Dvd,
                    year: Some(1992),
                    actors: vec![Actor::new("Harvey Keitel", "LT")],
                    ..Default::default()
                },
                Movie {
                    id: "2".to_string(),
                    title: "Dune".to_string(),
                    format: DiscFormat::BluRay,
                    is_3d: true,
                    year: Some(2021),
                    ..Default::default()
                },
            ],
        };

        assert_eq!(
            collection.filter_indices("lieutenant", None, false),
            vec![0]
        );
        assert_eq!(collection.filter_indices("keitel", None, false), vec![0]);
        assert_eq!(collection.filter_indices("2021", None, false), vec![1]);
        assert_eq!(
            collection.filter_indices("", Some(DiscFormat::BluRay), false),
            vec![1]
        );
        assert_eq!(collection.filter_indices("", None, true), vec![1]);
        assert!(
            collection
                .filter_indices("", Some(DiscFormat::Dvd), true)
                .is_empty()
        );
    }

    #[test]
    fn labels_3d_editions_with_their_base_format() {
        let mut disc = movie("1", "Dune", DiscFormat::BluRay, 2021);
        assert_eq!(disc.format_label(), "Blu-ray");
        assert_eq!(disc.format_badge_label(), "BD");

        disc.is_3d = true;
        assert_eq!(disc.format_label(), "Blu-ray 3D");
        assert_eq!(disc.format_badge_label(), "BD 3D");
    }

    #[test]
    fn splits_and_deduplicates_lists() {
        assert_eq!(
            split_list("Drama, Thriller; drama, , Crime"),
            vec!["Drama", "Thriller", "Crime"]
        );
    }

    #[test]
    fn manual_ids_are_unique() {
        assert_ne!(new_manual_id(), new_manual_id());
    }

    #[test]
    fn reimport_keeps_letterboxd_ratings_and_imdb_ids() {
        let mut existing = movie("1", "The Thing", DiscFormat::BluRay, 1982);
        existing.letterboxd_rating = Some(4.5);
        existing.imdb_id = "tt0084787".to_string();
        existing.notes = "Seen at the cinema".to_string();
        let mut collection = Collection {
            version: 1,
            movies: vec![existing],
        };

        let stats =
            collection.merge_imported(vec![movie("1", "The Thing", DiscFormat::BluRay, 1982)]);

        assert_eq!(stats.updated, 1);
        assert_eq!(collection.movies[0].letterboxd_rating, Some(4.5));
        assert_eq!(collection.movies[0].imdb_id, "tt0084787");
        assert_eq!(collection.movies[0].notes, "Seen at the cinema");
    }
}
