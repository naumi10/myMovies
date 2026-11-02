//! Open Movie Database lookup and mapping of its response to movie fields.

use std::io::Read;
use std::time::Duration;

use serde_json::Value;

use crate::model::{Actor, Movie, join_list};

const BASE_URL: &str = "https://www.omdbapi.com/";
const USER_AGENT: &str = concat!("myMovies/", env!("CARGO_PKG_VERSION"));
const TIMEOUT_SECONDS: u64 = 20;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// The OMDb fields that can update a movie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Year,
    Genres,
    Directors,
    Writers,
    Actors,
    Overview,
    Country,
    ImdbId,
}

impl Field {
    pub fn label(self) -> &'static str {
        match self {
            Self::Year => "Year",
            Self::Genres => "Genres",
            Self::Directors => "Directors",
            Self::Writers => "Writers",
            Self::Actors => "Actors",
            Self::Overview => "Plot",
            Self::Country => "Country",
            Self::ImdbId => "IMDb ID",
        }
    }
}

/// The response fields used by the app.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Details {
    pub year: Option<u16>,
    pub imdb_id: String,
    pub genres: Vec<String>,
    pub directors: Vec<String>,
    pub writers: Vec<String>,
    pub actors: Vec<String>,
    pub plot: String,
    pub country: String,
}

/// One field in the comparison dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub field: Field,
    pub label: &'static str,
    pub current: String,
    pub incoming: String,
    pub selected: bool,
}

impl Details {
    /// Builds the comparison rows for a movie. Fields that are empty in the
    /// OMDb response are not selected; changed values are selected by default.
    /// The year is only offered when the movie has no year yet.
    pub fn changes(&self, movie: &Movie) -> Vec<Change> {
        let mut rows = Vec::new();
        if movie.year.filter(|year| *year > 0).is_none() {
            rows.push((
                Field::Year,
                String::new(),
                self.year.map(|year| year.to_string()).unwrap_or_default(),
            ));
        }

        rows.extend([
            (
                Field::Genres,
                join_list(&movie.genres),
                join_list(&self.genres),
            ),
            (
                Field::Directors,
                join_list(&movie.directors),
                join_list(&self.directors),
            ),
            (
                Field::Writers,
                join_list(&movie.writers),
                join_list(&self.writers),
            ),
            (
                Field::Actors,
                join_list(
                    &movie
                        .actors
                        .iter()
                        .map(|actor| actor.name.clone())
                        .collect::<Vec<_>>(),
                ),
                join_list(&self.actors),
            ),
            (Field::Overview, movie.overview.clone(), self.plot.clone()),
            (Field::Country, movie.country.clone(), self.country.clone()),
            (Field::ImdbId, movie.imdb_id.clone(), self.imdb_id.clone()),
        ]);

        rows.into_iter()
            .map(|(field, current, incoming)| {
                let current_empty = current.trim().is_empty();
                Change {
                    field,
                    label: field.label(),
                    selected: !incoming.trim().is_empty() && (current_empty || incoming != current),
                    current,
                    incoming,
                }
            })
            .collect()
    }

    /// Applies the selected fields to a movie.
    pub fn apply(&self, movie: &mut Movie, fields: impl Iterator<Item = Field>) {
        for field in fields {
            match field {
                Field::Year => movie.year = self.year,
                Field::Genres => movie.genres = self.genres.clone(),
                Field::Directors => movie.directors = self.directors.clone(),
                Field::Writers => movie.writers = self.writers.clone(),
                Field::Actors => {
                    let previous = std::mem::take(&mut movie.actors);
                    movie.actors = self
                        .actors
                        .iter()
                        .map(|name| {
                            previous
                                .iter()
                                .find(|actor| actor.name.eq_ignore_ascii_case(name))
                                .cloned()
                                .unwrap_or_else(|| Actor::new(name.clone(), ""))
                        })
                        .collect();
                }
                Field::Overview => movie.overview = self.plot.clone(),
                Field::Country => movie.country = self.country.clone(),
                Field::ImdbId => movie.imdb_id = self.imdb_id.clone(),
            }
        }
    }
}

/// Looks up a movie by title and optional release year.
///
/// If the year does not match, OMDb's title-only result is tried once more so
/// a one-year difference does not hide otherwise correct data.
pub fn fetch(api_key: &str, title: &str, year: Option<u16>) -> Result<Details, String> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("The OMDb API key is empty.".to_string());
    }
    let title = title.trim();
    if title.is_empty() {
        return Err("The movie has no title to look up.".to_string());
    }

    let year = year.filter(|year| *year > 0);
    match fetch_once(api_key, title, year) {
        Err(error)
            if year.is_some()
                && (error.contains("not found") || error.contains("not be found")) =>
        {
            fetch_once(api_key, title, None)
        }
        result => result,
    }
}

fn fetch_once(api_key: &str, title: &str, year: Option<u16>) -> Result<Details, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(TIMEOUT_SECONDS))
        .redirects(5)
        .user_agent(USER_AGENT)
        .build();

    let mut request = agent
        .get(BASE_URL)
        .query("apikey", api_key)
        .query("t", title);
    if let Some(year) = year {
        request = request.query("y", &year.to_string());
    }

    let response = match request.call() {
        Ok(response) => response,
        Err(ureq::Error::Status(401, _)) => {
            return Err("OMDb rejected the API key (HTTP 401).".to_string());
        }
        Err(ureq::Error::Status(code, _)) => {
            return Err(format!("OMDb returned HTTP {code}."));
        }
        Err(ureq::Error::Transport(error)) => {
            return Err(format!("Could not reach OMDb: {error}"));
        }
    };

    let mut body = String::new();
    response
        .into_reader()
        .take(MAX_RESPONSE_BYTES as u64 + 1)
        .read_to_string(&mut body)
        .map_err(|error| format!("Could not read the OMDb response: {error}"))?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err("OMDb returned an unexpectedly large response.".to_string());
    }

    let value: Value = serde_json::from_str(&body)
        .map_err(|error| format!("Could not parse the OMDb response: {error}"))?;
    if value.get("Response").and_then(Value::as_str) == Some("False") {
        return Err(text(&value, "Error")
            .filter(|message| !message.is_empty())
            .unwrap_or_else(|| "OMDb could not find the movie.".to_string()));
    }

    Ok(details_from_json(&value))
}

fn details_from_json(value: &Value) -> Details {
    Details {
        year: year(value, "Year"),
        imdb_id: text(value, "imdbID").unwrap_or_default(),
        genres: list(value, "Genre"),
        directors: list(value, "Director"),
        writers: list(value, "Writer"),
        actors: list(value, "Actors"),
        plot: text(value, "Plot").unwrap_or_default(),
        country: text(value, "Country").unwrap_or_default(),
    }
}

/// OMDb uses a plain four-digit year for movies and a range for some series,
/// so take the first four-digit component.
fn year(value: &Value, key: &str) -> Option<u16> {
    text(value, key)?
        .split(|character: char| !character.is_ascii_digit())
        .find(|part| part.len() == 4)?
        .parse()
        .ok()
}

fn text(value: &Value, key: &str) -> Option<String> {
    let text = value.get(key)?.as_str()?.trim();
    if text.is_empty() || text == "N/A" {
        None
    } else {
        Some(text.to_string())
    }
}

fn list(value: &Value, key: &str) -> Vec<String> {
    text(value, key)
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DiscFormat;

    fn sample_json() -> Value {
        serde_json::json!({
            "Title": "The Silence of the Lambs",
            "Year": "1991",
            "Genre": "Crime, Drama, Thriller",
            "Director": "Jonathan Demme",
            "Writer": "Thomas Harris, Ted Tally",
            "Actors": "Jodie Foster, Anthony Hopkins, Scott Glenn",
            "Plot": "A young F.B.I. cadet must receive the help of an incarcerated cannibal.",
            "Country": "United States",
            "imdbID": "tt0102926",
            "Response": "True"
        })
    }

    #[test]
    fn maps_response_fields() {
        let details = details_from_json(&sample_json());
        assert_eq!(details.year, Some(1991));
        assert_eq!(details.imdb_id, "tt0102926");
        assert_eq!(details.genres, vec!["Crime", "Drama", "Thriller"]);
        assert_eq!(details.directors, vec!["Jonathan Demme"]);
        assert_eq!(details.writers, vec!["Thomas Harris", "Ted Tally"]);
        assert_eq!(
            details.actors,
            vec!["Jodie Foster", "Anthony Hopkins", "Scott Glenn"]
        );
        assert_eq!(details.country, "United States");
        assert!(details.plot.starts_with("A young F.B.I."));
    }

    #[test]
    fn treats_missing_values_as_empty() {
        let details = details_from_json(&serde_json::json!({
            "Genre": "N/A",
            "Plot": "N/A",
            "Response": "True"
        }));
        assert!(details.genres.is_empty());
        assert!(details.plot.is_empty());
        assert!(details.imdb_id.is_empty());
        assert_eq!(details.year, None);
    }

    #[test]
    fn parses_the_first_year_from_a_range() {
        let details = details_from_json(&serde_json::json!({
            "Year": "2019–2023",
            "Response": "True"
        }));
        assert_eq!(details.year, Some(2019));
    }

    #[test]
    fn offers_the_year_only_when_the_movie_has_none() {
        let details = details_from_json(&sample_json());
        let mut movie = Movie {
            title: "The Silence of the Lambs".to_string(),
            year: Some(1991),
            ..Default::default()
        };
        assert!(
            details
                .changes(&movie)
                .iter()
                .all(|change| change.field != Field::Year)
        );

        movie.year = None;
        let changes = details.changes(&movie);
        let year = changes
            .iter()
            .find(|change| change.field == Field::Year)
            .unwrap();
        assert!(year.selected);
        assert_eq!(year.incoming, "1991");

        details.apply(&mut movie, [Field::Year].into_iter());
        assert_eq!(movie.year, Some(1991));

        // A zero year is treated like a missing year, because imports can store 0.
        movie.year = Some(0);
        let changes = details.changes(&movie);
        let year = changes
            .iter()
            .find(|change| change.field == Field::Year)
            .unwrap();
        assert!(year.selected);
        details.apply(&mut movie, [Field::Year].into_iter());
        assert_eq!(movie.year, Some(1991));
    }

    #[test]
    fn builds_selected_changes() {
        let mut movie = Movie {
            title: "The Silence of the Lambs".to_string(),
            genres: vec!["Crime".to_string()],
            actors: vec![Actor::new("Jodie Foster", "Clarice Starling")],
            overview: "Old plot".to_string(),
            year: Some(1991),
            format: DiscFormat::Dvd,
            ..Default::default()
        };
        movie.imdb_id.clear();

        let details = details_from_json(&sample_json());
        let changes = details.changes(&movie);
        let genres = changes
            .iter()
            .find(|change| change.field == Field::Genres)
            .unwrap();
        assert!(genres.selected);
        let imdb = changes
            .iter()
            .find(|change| change.field == Field::ImdbId)
            .unwrap();
        assert!(imdb.selected);

        let selected = changes
            .iter()
            .filter(|change| change.selected)
            .map(|change| change.field);
        details.apply(&mut movie, selected);
        assert_eq!(movie.genres, vec!["Crime", "Drama", "Thriller"]);
        assert_eq!(movie.imdb_id, "tt0102926");
        assert_eq!(movie.actors[0].role, "Clarice Starling");
        assert_eq!(movie.actors.len(), 3);
    }

    #[test]
    fn selects_omdb_when_current_detail_is_empty() {
        let mut movie = Movie {
            title: "The Silence of the Lambs".to_string(),
            format: DiscFormat::Dvd,
            ..Default::default()
        };
        movie.country.clear();
        movie.imdb_id.clear();

        let details = details_from_json(&sample_json());
        let changes = details.changes(&movie);
        let country = changes
            .iter()
            .find(|change| change.field == Field::Country)
            .unwrap();
        assert!(country.selected);
        let imdb = changes
            .iter()
            .find(|change| change.field == Field::ImdbId)
            .unwrap();
        assert!(imdb.selected);
    }

    #[test]
    fn unchanged_fields_are_not_selected() {
        let details = details_from_json(&sample_json());
        let mut movie = Movie {
            title: "The Silence of the Lambs".to_string(),
            genres: details.genres.clone(),
            directors: details.directors.clone(),
            writers: details.writers.clone(),
            overview: details.plot.clone(),
            country: details.country.clone(),
            imdb_id: details.imdb_id.clone(),
            year: details.year,
            actors: details
                .actors
                .iter()
                .map(|name| Actor::new(name.clone(), ""))
                .collect(),
            ..Default::default()
        };
        assert!(
            details
                .changes(&movie)
                .iter()
                .all(|change| !change.selected)
        );
        movie.title = "Unchanged".to_string();
    }

    #[test]
    #[ignore = "contacts OMDb and needs OMDB_API_KEY"]
    fn live_lookup_returns_the_expected_profile() {
        let key = std::env::var("OMDB_API_KEY").expect("set OMDB_API_KEY");
        let details = fetch(&key, "The Silence of the Lambs", Some(1991)).unwrap();
        assert_eq!(details.year, Some(1991));
        assert_eq!(details.imdb_id, "tt0102926");
        assert_eq!(details.directors, vec!["Jonathan Demme"]);
    }
}
