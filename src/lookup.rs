use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::covers;
use crate::model::DiscFormat;

/// Movie details returned by the Invelos lookup script.
///
/// Only the fields that the manual editor can hold are kept; the raw profile
/// data stays in the script's JSON output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovieDetails {
    pub upc: String,
    pub profile_id: String,
    pub title: String,
    pub sort_title: String,
    pub format: DiscFormat,
    pub year: Option<u16>,
    pub runtime_minutes: Option<u32>,
    pub rating: String,
    pub directors: Vec<String>,
    pub cast: Vec<(String, String)>,
    pub synopsis: String,
    pub front_cover: bool,
    pub back_cover: bool,
}

/// The Invelos locality whose profiles should be used. Finland is 16.
pub fn locality() -> String {
    std::env::var("MYMOVIES_LOCALITY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "16".to_string())
}

/// Looks up a barcode by running `invelos_lookup.py`.
///
/// The Python script owns the Invelos protocol and its JSON schema; this
/// function only translates the result into the editor's fields.
pub fn fetch(upc: &str, locality: &str) -> Result<MovieDetails, String> {
    let upc = normalize_upc(upc)?;
    let script = script_path();
    if !script.is_file() {
        return Err(format!(
            "Lookup script not found at {}. Set MYMOVIES_LOOKUP_SCRIPT to its path.",
            script.display()
        ));
    }

    let json_path = temp_json_path();
    let output = Command::new(python_command())
        .arg(&script)
        .arg(&upc)
        .arg("--locality")
        .arg(locality)
        .arg("--json")
        .arg(&json_path)
        .arg("--covers-dir")
        .arg(covers::directory())
        .output()
        .map_err(|error| format!("Could not start the lookup script: {error}"))?;

    let result = if output.status.success() {
        match std::fs::read(&json_path) {
            Ok(bytes) => details_from_json(&bytes, &upc),
            Err(error) => Err(format!(
                "The lookup finished but did not write its result: {error}"
            )),
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(error_message(&stderr, &stdout))
    };

    let _ = std::fs::remove_file(&json_path);
    result
}

/// Resolves the lookup script. `MYMOVIES_LOOKUP_SCRIPT` wins; otherwise the
/// home-directory location used on this machine is tried.
pub fn script_path() -> PathBuf {
    if let Some(path) = std::env::var_os("MYMOVIES_LOOKUP_SCRIPT") {
        return PathBuf::from(path);
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let candidate = PathBuf::from(home)
            .join("invelos_lookup")
            .join("invelos_lookup.py");
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from("invelos_lookup").join("invelos_lookup.py")
}

fn python_command() -> String {
    std::env::var("MYMOVIES_PYTHON")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "python3".to_string())
}

fn temp_json_path() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "mymovies-lookup-{}-{stamp}.json",
        std::process::id()
    ))
}

/// Keeps only digits and checks the length accepted by the lookup script.
fn normalize_upc(value: &str) -> Result<String, String> {
    let digits: String = value.chars().filter(char::is_ascii_digit).collect();
    if !(8..=14).contains(&digits.len()) {
        return Err("Enter a barcode containing 8 to 14 digits.".to_string());
    }
    Ok(digits)
}

fn details_from_json(bytes: &[u8], requested_upc: &str) -> Result<MovieDetails, String> {
    let raw: RawLookup = serde_json::from_slice(bytes)
        .map_err(|error| format!("The lookup returned unreadable data: {error}"))?;

    let title = raw.title.trim().to_string();
    if title.is_empty() {
        return Err("The Invelos profile did not contain a title.".to_string());
    }

    let sort_title = if raw.sort_title.trim().is_empty() {
        title.clone()
    } else {
        raw.sort_title.trim().to_string()
    };

    let mut directors = unique_names(
        raw.crew_credits
            .iter()
            .filter(|credit| credit.credit_type.eq_ignore_ascii_case("Direction"))
            .map(|credit| credit.name.as_str()),
    );
    if directors.is_empty() {
        directors = raw
            .fields
            .get("Directed by")
            .map(|directed_by| unique_names(directed_by.split(',').map(str::trim)))
            .unwrap_or_default();
    }

    let mut cast: Vec<(String, String)> = Vec::new();
    for credit in &raw.cast_credits {
        let name = credit.name.trim();
        if name.is_empty()
            || cast
                .iter()
                .any(|(existing, _)| existing.eq_ignore_ascii_case(name))
        {
            continue;
        }
        cast.push((name.to_string(), credit.role.trim().to_string()));
        if cast.len() == 40 {
            break;
        }
    }
    if cast.is_empty() {
        cast = raw
            .fields
            .get("Starring")
            .map(|starring| {
                unique_names(starring.split(',').map(str::trim))
                    .into_iter()
                    .map(|name| (name, String::new()))
                    .collect()
            })
            .unwrap_or_default();
    }

    let format = raw
        .fields
        .get("Media Type")
        .map(|media_type| format_from_media_type(media_type))
        .unwrap_or_default();

    let year = parse_number::<u16>(&raw.profile_details.production_year)
        .or_else(|| {
            raw.fields
                .get("Produced")
                .and_then(|value| parse_number(value))
        })
        .filter(|year| (1888..=2200).contains(year));
    let runtime_minutes = parse_number::<u32>(&raw.profile_details.runtime_minutes);

    let upc = if raw.upc.trim().is_empty() {
        requested_upc.to_string()
    } else {
        normalize_upc(&raw.upc)?
    };

    Ok(MovieDetails {
        upc,
        profile_id: raw.profile_id,
        title,
        sort_title,
        format,
        year,
        runtime_minutes,
        rating: raw.profile_details.rating.trim().to_string(),
        directors,
        cast,
        synopsis: raw.synopsis.trim().to_string(),
        front_cover: raw.front_cover_available,
        back_cover: raw.back_cover_available,
    })
}

fn format_from_media_type(media_type: &str) -> DiscFormat {
    let value = media_type.to_ascii_lowercase();
    if value.contains("4k") || value.contains("ultra hd") || value.contains("uhd") {
        DiscFormat::UltraHd
    } else if value.contains("blu") {
        DiscFormat::BluRay
    } else {
        DiscFormat::Dvd
    }
}

fn unique_names<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for name in names {
        let name = name.trim();
        if name.is_empty()
            || result
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(name))
        {
            continue;
        }
        result.push(name.to_string());
    }
    result
}

fn parse_number<T: std::str::FromStr>(value: &str) -> Option<T> {
    let digits: String = value
        .chars()
        .skip_while(|character| !character.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// Pulls a useful line out of a Python traceback for the dialog.
fn error_message(stderr: &str, stdout: &str) -> String {
    for line in stderr.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(message) = line.strip_prefix("ValueError: ") {
            return message.to_string();
        }
        if let Some(message) = line.strip_prefix("RuntimeError: ") {
            return message.to_string();
        }
        if !line.starts_with("Traceback") && !line.starts_with("File \"") {
            return line.to_string();
        }
    }

    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "The lookup failed without an error message.".to_string())
}

#[derive(Debug, Default, Deserialize)]
struct RawLookup {
    #[serde(default)]
    upc: String,
    #[serde(default)]
    profile_id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    sort_title: String,
    #[serde(default)]
    fields: BTreeMap<String, String>,
    #[serde(default)]
    synopsis: String,
    #[serde(default)]
    profile_details: ProfileDetails,
    #[serde(default)]
    cast_credits: Vec<CastCredit>,
    #[serde(default)]
    crew_credits: Vec<CrewCredit>,
    #[serde(default)]
    front_cover_available: bool,
    #[serde(default)]
    back_cover_available: bool,
}

#[derive(Debug, Default, Deserialize)]
struct ProfileDetails {
    #[serde(default)]
    production_year: String,
    #[serde(default)]
    rating: String,
    #[serde(default)]
    runtime_minutes: String,
}

#[derive(Debug, Default, Deserialize)]
struct CastCredit {
    #[serde(default)]
    name: String,
    #[serde(default)]
    role: String,
}

#[derive(Debug, Default, Deserialize)]
struct CrewCredit {
    #[serde(default)]
    name: String,
    #[serde(default)]
    credit_type: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "upc": "044007867020",
        "locality_id": "16",
        "profile_id": "044007867020.16",
        "title": "Ihmissusi Lontoossa",
        "sort_title": "Ihmissusi Lontoossa",
        "fields": {
            "Locality": "Finland",
            "Produced": "1981",
            "Media Type": "DVD",
            "Directed by": "John Landis",
            "Starring": "David Naughton, Jenny Agutter"
        },
        "synopsis": "Two backpackers run into trouble on the moors.",
        "profile_details": {
            "production_year": "1981",
            "rating": "18",
            "runtime_minutes": "93"
        },
        "cast_credits": [
            {"name": "David Naughton", "role": "David Kessler"},
            {"name": "Jenny Agutter", "role": "Nurse Alex Price"},
            {"name": "David Naughton", "role": "Duplicate"}
        ],
        "crew_credits": [
            {"name": "John Landis", "credit_type": "Direction"},
            {"name": "John Landis", "credit_type": "Writing"},
            {"name": "John Landis", "credit_type": "Direction"}
        ],
        "front_cover_available": true,
        "back_cover_available": false
    }"#;

    #[test]
    fn maps_profile_json_to_editor_details() {
        let details = details_from_json(SAMPLE.as_bytes(), "044007867020").unwrap();
        assert_eq!(details.title, "Ihmissusi Lontoossa");
        assert_eq!(details.format, DiscFormat::Dvd);
        assert_eq!(details.year, Some(1981));
        assert_eq!(details.runtime_minutes, Some(93));
        assert_eq!(details.rating, "18");
        assert_eq!(details.directors, vec!["John Landis"]);
        assert_eq!(
            details.cast,
            vec![
                ("David Naughton".to_string(), "David Kessler".to_string()),
                ("Jenny Agutter".to_string(), "Nurse Alex Price".to_string()),
            ]
        );
        assert_eq!(details.upc, "044007867020");
        assert!(details.front_cover);
        assert!(!details.back_cover);
    }

    #[test]
    fn infers_blu_ray_and_4k_media_types() {
        assert_eq!(format_from_media_type("DVD"), DiscFormat::Dvd);
        assert_eq!(format_from_media_type("Blu-ray Disc"), DiscFormat::BluRay);
        assert_eq!(format_from_media_type("4K Ultra HD"), DiscFormat::UltraHd);
        assert_eq!(format_from_media_type("UHD Blu-ray"), DiscFormat::UltraHd);
    }

    #[test]
    fn falls_back_to_preview_credits() {
        let json = r#"{
            "upc": "123456789012",
            "profile_id": "123456789012.16",
            "title": "Fallback",
            "fields": {
                "Produced": "1999",
                "Directed by": "First Director, Second Director",
                "Starring": "One Actor, Two Actor"
            }
        }"#;
        let details = details_from_json(json.as_bytes(), "123456789012").unwrap();
        assert_eq!(details.year, Some(1999));
        assert_eq!(details.directors, vec!["First Director", "Second Director"]);
        assert_eq!(
            details.cast,
            vec![
                ("One Actor".to_string(), String::new()),
                ("Two Actor".to_string(), String::new()),
            ]
        );
    }

    #[test]
    fn validates_barcodes() {
        assert_eq!(normalize_upc(" 044007-867020 ").unwrap(), "044007867020");
        assert!(normalize_upc("123").is_err());
        assert!(normalize_upc("").is_err());
    }

    #[test]
    fn extracts_the_final_python_error() {
        let stderr = "Traceback (most recent call last):\n  File \"x.py\", line 1\n    boom\nValueError: Invelos returned HTTP 404 for /x.doi.\n";
        assert_eq!(
            error_message(stderr, ""),
            "Invelos returned HTTP 404 for /x.doi."
        );
        assert_eq!(error_message("", "some output\n"), "some output");
    }
}
