//! Import of the Letterboxd ratings export.
//!
//! Letterboxd exports a ZIP archive that contains `ratings.csv` with the
//! columns `Date,Name,Year,Letterboxd URI,Rating`. Only the ZIP central
//! directory is read here, which keeps the importer independent of a full ZIP
//! library while still supporting the deflate-compressed archives Letterboxd
//! produces.

use std::collections::{HashMap, HashSet};
use std::io::Read;

use flate2::read::DeflateDecoder;

use crate::model::Collection;

/// A single row from `ratings.csv`.
#[derive(Debug, Clone, PartialEq)]
pub struct Rating {
    pub name: String,
    pub year: Option<u16>,
    pub rating: f32,
}

/// Result of merging a Letterboxd export into the collection.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ImportStats {
    /// Number of rows read from `ratings.csv`.
    pub entries: usize,
    /// Number of collection movies that received a rating.
    pub rated: usize,
    /// Collection movies whose title matched but whose year did not match.
    pub skipped: usize,
}

/// Reads `ratings.csv` from a Letterboxd ZIP export.
pub fn parse_export(bytes: &[u8]) -> Result<Vec<Rating>, String> {
    let data = read_zip_entry(bytes, "ratings.csv")?
        .ok_or_else(|| "The ZIP file does not contain ratings.csv.".to_string())?;
    let text =
        String::from_utf8(data).map_err(|_| "ratings.csv is not valid UTF-8.".to_string())?;
    parse_ratings_csv(&text)
}

/// Merges Letterboxd ratings into the collection by normalized title and year.
///
/// Both the release title and the original title are considered. An exact year
/// match is preferred, then a one-year tolerance, and finally a title that
/// appears only once in the ratings file. Conflicting title matches are skipped
/// instead of guessing.
pub fn apply(collection: &mut Collection, ratings: &[Rating]) -> ImportStats {
    let mut by_title: HashMap<String, Vec<&Rating>> = HashMap::new();
    for rating in ratings {
        by_title
            .entry(normalize_title(&rating.name))
            .or_default()
            .push(rating);
    }

    let mut stats = ImportStats {
        entries: ratings.len(),
        ..Default::default()
    };

    // A Letterboxd export is authoritative: ratings removed from the export
    // should disappear here too.
    for movie in &mut collection.movies {
        movie.letterboxd_rating = None;
    }

    for movie in &mut collection.movies {
        let years = movie_years(movie.year, &movie.released);
        let mut candidates: Vec<&Rating> = Vec::new();
        let mut seen = HashSet::new();

        for title in [&movie.title, &movie.original_title] {
            if title.trim().is_empty() {
                continue;
            }
            if let Some(matches) = by_title.get(&normalize_title(title)) {
                for rating in matches {
                    let key = (
                        rating.name.clone(),
                        rating.year,
                        (rating.rating * 10.0) as u32,
                    );
                    if seen.insert(key) {
                        candidates.push(rating);
                    }
                }
            }
        }

        match pick_rating(&candidates, &years) {
            Some(rating) => {
                movie.letterboxd_rating = Some(rating);
                stats.rated += 1;
            }
            None if !candidates.is_empty() => stats.skipped += 1,
            None => {}
        }
    }

    stats
}

fn pick_rating(candidates: &[&Rating], years: &[u16]) -> Option<f32> {
    if candidates.is_empty() {
        return None;
    }

    // Prefer an exact year match, then allow for a one-year difference between
    // a production year and Letterboxd's release year.
    for tolerance in [0, 1] {
        let mut best: Option<(u16, f32)> = None;
        let mut ambiguous = false;
        for rating in candidates {
            let Some(year) = rating.year else {
                continue;
            };
            let distance = years
                .iter()
                .map(|movie_year| movie_year.abs_diff(year))
                .min()
                .unwrap_or(u16::MAX);
            if distance > tolerance {
                continue;
            }
            match best {
                None => best = Some((distance, rating.rating)),
                Some((best_distance, _)) if distance < best_distance => {
                    best = Some((distance, rating.rating));
                    ambiguous = false;
                }
                Some((best_distance, best_rating))
                    if distance == best_distance && rating.rating != best_rating =>
                {
                    ambiguous = true;
                }
                _ => {}
            }
        }
        if ambiguous {
            return None;
        }
        if let Some((_, rating)) = best {
            return Some(rating);
        }
    }

    // Without any year information, a title that maps to a single distinct
    // rating is the best available match. When the movie has a year, never
    // fall back to a different film with the same title (for example a remake).
    if years.is_empty() || candidates.iter().all(|rating| rating.year.is_none()) {
        let first = candidates[0].rating;
        if candidates.iter().all(|rating| rating.rating == first) {
            return Some(first);
        }
    }
    None
}

fn movie_years(year: Option<u16>, released: &str) -> Vec<u16> {
    let mut years = Vec::new();
    if let Some(year) = year.filter(|year| *year > 0) {
        years.push(year);
    }
    if let Some(year) = released
        .get(0..4)
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|year| *year > 0)
        && !years.contains(&year)
    {
        years.push(year);
    }
    years
}

/// Lowercases a title and ignores punctuation, spacing and a leading article.
fn normalize_title(title: &str) -> String {
    let lowered = title.to_lowercase().replace('&', " and ");
    let cleaned: String = lowered
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect();
    let mut words: Vec<&str> = cleaned.split_whitespace().collect();
    if matches!(words.first(), Some(&"the" | &"a" | &"an")) {
        words.remove(0);
    }
    words.join(" ")
}

fn parse_ratings_csv(text: &str) -> Result<Vec<Rating>, String> {
    let text = text.trim_start_matches('\u{feff}');
    let rows = parse_csv(text);
    let Some(header) = rows.first() else {
        return Err("ratings.csv is empty.".to_string());
    };

    let column = |name: &str| {
        header
            .iter()
            .position(|value| value.trim().eq_ignore_ascii_case(name))
    };
    let name_index = column("Name").ok_or_else(|| "ratings.csv has no Name column.".to_string())?;
    let year_index = column("Year");
    let rating_index =
        column("Rating").ok_or_else(|| "ratings.csv has no Rating column.".to_string())?;

    let mut ratings = Vec::new();
    for row in rows.iter().skip(1) {
        let Some(name) = row.get(name_index).map(|value| value.trim()) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let Some(value) = row.get(rating_index).map(|value| value.trim()) else {
            continue;
        };
        let Ok(rating) = value.parse::<f32>() else {
            continue;
        };
        if !(0.5..=5.0).contains(&rating) {
            continue;
        }
        let year = year_index
            .and_then(|index| row.get(index))
            .map(|value| value.trim())
            .and_then(|value| value.parse::<u16>().ok());

        ratings.push(Rating {
            name: name.to_string(),
            year,
            rating,
        });
    }

    Ok(ratings)
}

fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut characters = text.chars().peekable();

    while let Some(character) = characters.next() {
        match character {
            '"' if quoted => {
                if characters.peek() == Some(&'"') {
                    characters.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            }
            '"' => quoted = true,
            ',' if !quoted => {
                row.push(std::mem::take(&mut field));
            }
            '\r' if !quoted => {}
            '\n' if !quoted => {
                if !field.is_empty() || !row.is_empty() {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
            }
            _ => field.push(character),
        }
    }

    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }

    rows
}

fn read_zip_entry(data: &[u8], wanted: &str) -> Result<Option<Vec<u8>>, String> {
    let Some(end) = find_end_of_central_directory(data) else {
        return Err("The file is not a valid ZIP archive.".to_string());
    };
    let count = u16_at(data, end + 10).ok_or_else(malformed_zip)?;
    let directory_offset = u32_at(data, end + 16).ok_or_else(malformed_zip)? as usize;
    if count == u16::MAX || directory_offset == u32::MAX as usize {
        return Err("ZIP64 archives are not supported.".to_string());
    }

    let mut position = directory_offset;
    let mut exact_match: Option<(usize, u16, usize, usize)> = None;
    let mut any_match: Option<(usize, u16, usize, usize)> = None;

    for _ in 0..count {
        if u32_at(data, position) != Some(0x0201_4b50) {
            return Err(malformed_zip());
        }
        let method = u16_at(data, position + 10).ok_or_else(malformed_zip)?;
        let compressed_size = u32_at(data, position + 20).ok_or_else(malformed_zip)? as usize;
        let uncompressed_size = u32_at(data, position + 24).ok_or_else(malformed_zip)? as usize;
        let name_length = u16_at(data, position + 28).ok_or_else(malformed_zip)? as usize;
        let extra_length = u16_at(data, position + 30).ok_or_else(malformed_zip)? as usize;
        let comment_length = u16_at(data, position + 32).ok_or_else(malformed_zip)? as usize;
        let local_offset = u32_at(data, position + 42).ok_or_else(malformed_zip)? as usize;
        let name_start = position + 46;
        let name_end = name_start
            .checked_add(name_length)
            .ok_or_else(malformed_zip)?;
        let name = data.get(name_start..name_end).ok_or_else(malformed_zip)?;
        let name = String::from_utf8_lossy(name);
        let entry = (local_offset, method, compressed_size, uncompressed_size);

        if name.eq_ignore_ascii_case(wanted) {
            exact_match = Some(entry);
        } else if name
            .rsplit('/')
            .next()
            .is_some_and(|file_name| file_name.eq_ignore_ascii_case(wanted))
            && any_match.is_none()
        {
            any_match = Some(entry);
        }

        position = name_end
            .checked_add(extra_length)
            .and_then(|value| value.checked_add(comment_length))
            .ok_or_else(malformed_zip)?;
    }

    let Some((local_offset, method, compressed_size, uncompressed_size)) =
        exact_match.or(any_match)
    else {
        return Ok(None);
    };

    extract_zip_entry(
        data,
        local_offset,
        method,
        compressed_size,
        uncompressed_size,
    )
    .map(Some)
}

fn extract_zip_entry(
    data: &[u8],
    local_offset: usize,
    method: u16,
    compressed_size: usize,
    uncompressed_size: usize,
) -> Result<Vec<u8>, String> {
    const MAX_ENTRY_SIZE: usize = 64 * 1024 * 1024;

    if uncompressed_size > MAX_ENTRY_SIZE {
        return Err("ratings.csv is unexpectedly large.".to_string());
    }
    if u32_at(data, local_offset) != Some(0x0403_4b50) {
        return Err(malformed_zip());
    }
    let name_length = u16_at(data, local_offset + 26).ok_or_else(malformed_zip)? as usize;
    let extra_length = u16_at(data, local_offset + 28).ok_or_else(malformed_zip)? as usize;
    let start = local_offset
        .checked_add(30)
        .and_then(|value| value.checked_add(name_length))
        .and_then(|value| value.checked_add(extra_length))
        .ok_or_else(malformed_zip)?;
    let end = start
        .checked_add(compressed_size)
        .ok_or_else(malformed_zip)?;
    let compressed = data.get(start..end).ok_or_else(malformed_zip)?;

    match method {
        0 => Ok(compressed.to_vec()),
        8 => {
            let decoder = DeflateDecoder::new(compressed);
            let mut output = Vec::with_capacity(uncompressed_size.min(MAX_ENTRY_SIZE));
            decoder
                .take(MAX_ENTRY_SIZE as u64 + 1)
                .read_to_end(&mut output)
                .map_err(|error| format!("Could not decompress ratings.csv: {error}"))?;
            if output.len() > MAX_ENTRY_SIZE {
                return Err("ratings.csv is unexpectedly large.".to_string());
            }
            Ok(output)
        }
        method => Err(format!(
            "ratings.csv uses unsupported ZIP compression (method {method})."
        )),
    }
}

fn find_end_of_central_directory(data: &[u8]) -> Option<usize> {
    if data.len() < 22 {
        return None;
    }
    let last = data.len() - 22;
    let first = last.saturating_sub(u16::MAX as usize);
    (first..=last)
        .rev()
        .find(|&offset| u32_at(data, offset) == Some(0x0605_4b50))
}

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = data.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn malformed_zip() -> String {
    "The ZIP archive has an unexpected structure.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiscFormat, Movie};

    fn movie(id: &str, title: &str, year: u16) -> Movie {
        Movie {
            id: id.to_string(),
            title: title.to_string(),
            year: Some(year),
            format: DiscFormat::Dvd,
            ..Default::default()
        }
    }

    fn stored_zip(name: &str, contents: &[u8]) -> Vec<u8> {
        let crc = 0u32;
        let size = contents.len() as u32;
        let name = name.as_bytes();
        let mut data = Vec::new();

        // Local file header.
        data.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        data.extend_from_slice(&20u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&crc.to_le_bytes());
        data.extend_from_slice(&size.to_le_bytes());
        data.extend_from_slice(&size.to_le_bytes());
        data.extend_from_slice(&(name.len() as u16).to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(name);
        data.extend_from_slice(contents);

        let directory_offset = data.len() as u32;

        // Central directory entry.
        data.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        data.extend_from_slice(&20u16.to_le_bytes());
        data.extend_from_slice(&20u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&crc.to_le_bytes());
        data.extend_from_slice(&size.to_le_bytes());
        data.extend_from_slice(&size.to_le_bytes());
        data.extend_from_slice(&(name.len() as u16).to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes()); // extra
        data.extend_from_slice(&0u16.to_le_bytes()); // comment
        data.extend_from_slice(&0u16.to_le_bytes()); // disk
        data.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        data.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        data.extend_from_slice(&0u32.to_le_bytes()); // local header offset
        data.extend_from_slice(name);

        let directory_size = data.len() as u32 - directory_offset;
        data.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&directory_size.to_le_bytes());
        data.extend_from_slice(&directory_offset.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data
    }

    #[test]
    fn reads_ratings_from_a_zip() {
        let csv = "Date,Name,Year,Letterboxd URI,Rating\n\
                   2020-02-09,The Silence of the Lambs,1991,https://boxd.it/2aHW,5\n\
                   2020-02-13,\"Kick-Ass, Vol. 2\",2013,https://boxd.it/hqu,3.5\n";
        let zip = stored_zip("ratings.csv", csv.as_bytes());
        let ratings = parse_export(&zip).unwrap();
        assert_eq!(ratings.len(), 2);
        assert_eq!(ratings[0].name, "The Silence of the Lambs");
        assert_eq!(ratings[0].year, Some(1991));
        assert_eq!(ratings[0].rating, 5.0);
        assert_eq!(ratings[1].name, "Kick-Ass, Vol. 2");
        assert_eq!(ratings[1].rating, 3.5);
    }

    #[test]
    fn reports_a_missing_ratings_file() {
        let zip = stored_zip("watched.csv", b"Date,Name\n");
        assert_eq!(
            parse_export(&zip).unwrap_err(),
            "The ZIP file does not contain ratings.csv."
        );
    }

    #[test]
    fn matches_titles_and_years() {
        let mut collection = Collection {
            version: 1,
            movies: vec![
                movie("1", "T2", 1991),
                movie("2", "The Thing", 1982),
                movie("3", "Unknown Film", 2000),
            ],
        };
        collection.movies[0].original_title = "Terminator 2: Judgment Day".to_string();
        collection.movies[2].letterboxd_rating = Some(1.0);
        let ratings = vec![
            Rating {
                name: "Terminator 2: Judgment Day".to_string(),
                year: Some(1991),
                rating: 4.5,
            },
            Rating {
                name: "The Thing".to_string(),
                year: Some(1982),
                rating: 5.0,
            },
            Rating {
                name: "Different Film".to_string(),
                year: Some(2001),
                rating: 3.0,
            },
        ];

        let stats = apply(&mut collection, &ratings);
        assert_eq!(stats.entries, 3);
        assert_eq!(stats.rated, 2);
        assert_eq!(collection.movies[0].letterboxd_rating, Some(4.5));
        assert_eq!(collection.movies[1].letterboxd_rating, Some(5.0));
        assert_eq!(collection.movies[2].letterboxd_rating, None);
    }

    #[test]
    fn tolerates_one_year_and_ignores_leading_articles() {
        let mut collection = Collection {
            version: 1,
            movies: vec![movie("1", "The Thing", 1982)],
        };
        let ratings = vec![Rating {
            name: "Thing".to_string(),
            year: Some(1983),
            rating: 4.0,
        }];

        let stats = apply(&mut collection, &ratings);
        assert_eq!(stats.rated, 1);
        assert_eq!(collection.movies[0].letterboxd_rating, Some(4.0));
    }

    #[test]
    fn prefers_the_exact_year_over_a_title_only_match() {
        let mut collection = Collection {
            version: 1,
            movies: vec![movie("1", "Solaris", 2002)],
        };
        let ratings = vec![
            Rating {
                name: "Solaris".to_string(),
                year: Some(1972),
                rating: 5.0,
            },
            Rating {
                name: "Solaris".to_string(),
                year: Some(2002),
                rating: 2.0,
            },
        ];

        let stats = apply(&mut collection, &ratings);
        assert_eq!(stats.rated, 1);
        assert_eq!(collection.movies[0].letterboxd_rating, Some(2.0));
    }

    #[test]
    #[ignore = "reads a real Letterboxd export from MYMOVIES_LETTERBOXD_TEST_ZIP"]
    fn live_letterboxd_export_is_readable() {
        let Some(path) = std::env::var_os("MYMOVIES_LETTERBOXD_TEST_ZIP") else {
            return;
        };
        let bytes = std::fs::read(path).expect("read the Letterboxd export");
        let ratings = parse_export(&bytes).expect("parse the Letterboxd export");
        assert!(!ratings.is_empty());
    }

    #[test]
    fn does_not_match_a_remake_with_a_far_apart_year() {
        let mut collection = Collection {
            version: 1,
            movies: vec![movie("1", "All Quiet on the Western Front", 1930)],
        };
        let ratings = vec![Rating {
            name: "All Quiet on the Western Front".to_string(),
            year: Some(2022),
            rating: 4.5,
        }];

        let stats = apply(&mut collection, &ratings);
        assert_eq!(stats.rated, 0);
        assert_eq!(collection.movies[0].letterboxd_rating, None);
    }

    #[test]
    fn matches_a_title_when_the_movie_has_no_year() {
        let mut collection = Collection {
            version: 1,
            movies: vec![Movie {
                id: "1".to_string(),
                title: "Abigail".to_string(),
                ..Default::default()
            }],
        };
        let ratings = vec![Rating {
            name: "Abigail".to_string(),
            year: Some(2024),
            rating: 3.5,
        }];

        let stats = apply(&mut collection, &ratings);
        assert_eq!(stats.rated, 1);
        assert_eq!(collection.movies[0].letterboxd_rating, Some(3.5));
    }

    #[test]
    fn skips_conflicting_title_only_matches() {
        let ratings = vec![
            Rating {
                name: "Solaris".to_string(),
                year: Some(1972),
                rating: 5.0,
            },
            Rating {
                name: "Solaris".to_string(),
                year: Some(2002),
                rating: 2.0,
            },
        ];
        let mut collection = Collection {
            version: 1,
            movies: vec![Movie {
                id: "1".to_string(),
                title: "Solaris".to_string(),
                ..Default::default()
            }],
        };

        let stats = apply(&mut collection, &ratings);
        assert_eq!(stats.rated, 0);
        assert_eq!(stats.skipped, 1);
    }
}
