use std::collections::HashSet;

use encoding_rs::WINDOWS_1252;
use roxmltree::{Document, Node};

use crate::model::{Actor, DiscFormat, Movie};

#[derive(Debug, Default)]
pub struct ParsedCollection {
    pub movies: Vec<Movie>,
    /// Entries that could not be used (empty title or duplicate id).
    pub skipped: usize,
}

/// Reads a DVD Profiler XML export.
///
/// DVD Profiler writes Windows-1252 encoded files, but UTF-8 exports are
/// accepted as well. The file is decoded to a Rust string before parsing, so
/// the parser itself only ever sees UTF-8.
pub fn parse_collection(bytes: &[u8]) -> Result<ParsedCollection, String> {
    let text = decode_document(bytes)?;
    let document =
        Document::parse(&text).map_err(|error| format!("Could not parse the XML file: {error}"))?;
    let root = document.root_element();

    let mut entries: Vec<Node<'_, '_>> = root
        .children()
        .filter(|node| node.is_element() && node.has_tag_name("DVD"))
        .collect();
    if entries.is_empty() && root.has_tag_name("DVD") {
        entries.push(root);
    }

    let mut result = ParsedCollection::default();
    let mut seen_ids = HashSet::new();

    for entry in entries {
        let Some(movie) = parse_entry(entry) else {
            result.skipped += 1;
            continue;
        };
        if !seen_ids.insert(movie.id.clone()) {
            result.skipped += 1;
            continue;
        }
        result.movies.push(movie);
    }

    if result.movies.is_empty() {
        return Err(
            "The XML file did not contain any DVD Profiler entries with a title.".to_string(),
        );
    }

    Ok(result)
}

fn decode_document(bytes: &[u8]) -> Result<String, String> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);

    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err("UTF-16 encoded XML files are not supported.".to_string());
    }

    let header_len = bytes.len().min(512);
    let header = String::from_utf8_lossy(&bytes[..header_len]).to_ascii_lowercase();
    let declared_encoding = extract_encoding(&header);

    let prefer_utf8 = match declared_encoding.as_deref() {
        Some(encoding) => matches!(encoding, "utf-8" | "utf8"),
        None => true,
    };

    if prefer_utf8 && let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(strip_declaration(text));
    }

    let (decoded, _, had_errors) = WINDOWS_1252.decode(bytes);
    if had_errors {
        return Err("The XML file uses an unsupported character encoding.".to_string());
    }
    Ok(strip_declaration(&decoded))
}

fn extract_encoding(header: &str) -> Option<String> {
    let start = header.find("encoding")?;
    let rest = &header[start + "encoding".len()..];
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &rest[1..];
    let end = rest.find(quote)?;
    Some(rest[..end].trim().to_string())
}

fn strip_declaration(text: &str) -> String {
    let text = text.trim_start_matches('\u{feff}');
    if text.trim_start().starts_with("<?xml")
        && let Some(end) = text.find("?>")
    {
        return text[end + 2..].trim_start_matches(['\r', '\n']).to_string();
    }
    text.to_string()
}

fn parse_entry(entry: Node<'_, '_>) -> Option<Movie> {
    let title = text_of(entry, "Title");
    if title.is_empty() {
        return None;
    }

    let year = text_of(entry, "ProductionYear").parse::<u16>().ok();
    let profile_id = text_of(entry, "ID");
    let upc = text_of(entry, "UPC");
    let id = if !profile_id.is_empty() {
        format!("dvdprofiler:{profile_id}")
    } else if !upc.is_empty() {
        format!("upc:{upc}")
    } else {
        format!("title:{}:{}", title.to_lowercase(), year.unwrap_or(0))
    };

    let mut sort_title = text_of(entry, "SortTitle");
    if sort_title.is_empty() {
        sort_title = title.clone();
    }

    let mut movie = Movie {
        id,
        title,
        sort_title,
        format: parse_format(entry),
        year,
        released: text_of(entry, "Released"),
        runtime_minutes: text_of(entry, "RunningTime").parse::<u32>().ok(),
        rating: text_of(entry, "Rating"),
        rating_details: text_of(entry, "RatingDetails"),
        genres: list_of(entry, "Genres", "Genre"),
        studios: list_of(entry, "Studios", "Studio"),
        media_companies: list_of(entry, "MediaCompanies", "MediaCompany"),
        regions: list_of(entry, "Regions", "Region"),
        country: text_of(entry, "CountryOfOrigin"),
        overview: clean_text(&text_of(entry, "Overview")),
        actors: parse_actors(entry),
        directors: parse_credits(entry, |credit_type, subtype| {
            credit_type == "Direction" || subtype == "Director"
        }),
        writers: parse_credits(entry, |credit_type, _| credit_type == "Writing"),
        audio: parse_audio(entry),
        subtitles: list_of(entry, "Subtitles", "Subtitle"),
        disc_count: 0,
        upc,
        collection_number: text_of(entry, "CollectionNumber"),
        case_type: text_of(entry, "CaseType"),
        location: String::new(),
        purchase_date: String::new(),
        purchase_price: String::new(),
        purchase_place: String::new(),
        notes: clean_text(&text_of(entry, "Notes")),
    };

    if let Some(discs) = child(entry, "Discs") {
        let disc_nodes: Vec<_> = discs
            .children()
            .filter(|node| node.is_element() && node.has_tag_name("Disc"))
            .collect();
        movie.disc_count = disc_nodes.len() as u32;
        movie.location = disc_nodes
            .iter()
            .map(|disc| text_of(*disc, "Location"))
            .find(|location| !location.is_empty())
            .unwrap_or_default();
    }

    if let Some(purchase) = child(entry, "PurchaseInfo") {
        movie.purchase_date = text_of(purchase, "PurchaseDate");
        movie.purchase_place = text_of(purchase, "PurchasePlace");

        if let Some(price) = child(purchase, "PurchasePrice") {
            let value = price.text().unwrap_or("").trim().replace(',', ".");
            if let Ok(value) = value.parse::<f64>()
                && value > 0.0
            {
                let currency = price.attribute("DenominationType").unwrap_or("").trim();
                movie.purchase_price = if currency.is_empty() {
                    format!("{value:.2}")
                } else {
                    format!("{value:.2} {currency}")
                };
            }
        }
    }

    Some(movie)
}

fn parse_format(entry: Node<'_, '_>) -> DiscFormat {
    let media = child(entry, "MediaTypes");
    let is_set = |name: &str| {
        media
            .and_then(|media| child(media, name))
            .and_then(|node| node.text())
            .is_some_and(|text| text.trim().eq_ignore_ascii_case("true"))
    };

    if is_set("UltraHD") {
        DiscFormat::UltraHd
    } else if is_set("BluRay") {
        DiscFormat::BluRay
    } else {
        DiscFormat::Dvd
    }
}

fn parse_actors(entry: Node<'_, '_>) -> Vec<Actor> {
    let Some(actors) = child(entry, "Actors") else {
        return Vec::new();
    };

    actors
        .children()
        .filter(|node| node.is_element() && node.has_tag_name("Actor"))
        .filter_map(|actor| {
            let name = person_name(actor);
            if name.is_empty() {
                return None;
            }
            Some(Actor {
                name,
                role: attribute(actor, "Role"),
                voice: attribute(actor, "Voice").eq_ignore_ascii_case("true"),
                uncredited: attribute(actor, "Uncredited").eq_ignore_ascii_case("true"),
            })
        })
        .collect()
}

fn parse_credits(entry: Node<'_, '_>, wanted: impl Fn(&str, &str) -> bool) -> Vec<String> {
    let Some(credits) = child(entry, "Credits") else {
        return Vec::new();
    };

    let mut names = Vec::new();
    for credit in credits
        .children()
        .filter(|node| node.is_element() && node.has_tag_name("Credit"))
    {
        let credit_type = attribute(credit, "CreditType");
        let subtype = attribute(credit, "CreditSubtype");
        if !wanted(&credit_type, &subtype) {
            continue;
        }
        let name = person_name(credit);
        if !name.is_empty() && !names.iter().any(|seen: &String| seen == &name) {
            names.push(name);
        }
    }
    names
}

fn parse_audio(entry: Node<'_, '_>) -> Vec<String> {
    let Some(audio) = child(entry, "Audio") else {
        return Vec::new();
    };

    let mut tracks = Vec::new();
    for track in audio
        .children()
        .filter(|node| node.is_element() && node.has_tag_name("AudioTrack"))
    {
        let parts: Vec<String> = ["AudioContent", "AudioFormat", "AudioChannels"]
            .iter()
            .map(|tag| text_of(track, tag))
            .filter(|part| !part.is_empty())
            .collect();
        if parts.is_empty() {
            continue;
        }
        let description = parts.join(" · ");
        if !tracks.iter().any(|seen: &String| seen == &description) {
            tracks.push(description);
        }
    }
    tracks
}

fn person_name(node: Node<'_, '_>) -> String {
    let parts: Vec<String> = ["FirstName", "MiddleName", "LastName"]
        .iter()
        .map(|name| attribute(node, name))
        .filter(|part| !part.is_empty())
        .collect();
    if !parts.is_empty() {
        return parts.join(" ");
    }
    attribute(node, "CreditedAs")
}

fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|child| child.is_element() && child.has_tag_name(name))
}

fn text_of(node: Node<'_, '_>, name: &str) -> String {
    child(node, name)
        .and_then(|child| child.text())
        .map(clean_text)
        .unwrap_or_default()
}

fn attribute(node: Node<'_, '_>, name: &str) -> String {
    node.attribute(name)
        .map(str::trim)
        .unwrap_or("")
        .to_string()
}

fn list_of(node: Node<'_, '_>, parent: &str, child_name: &str) -> Vec<String> {
    let Some(parent) = child(node, parent) else {
        return Vec::new();
    };

    let mut items: Vec<String> = Vec::new();
    for item in parent
        .children()
        .filter(|node| node.is_element() && node.has_tag_name(child_name))
        .filter_map(|node| node.text())
        .map(clean_text)
        .filter(|text| !text.is_empty())
    {
        if !items.iter().any(|seen| seen.eq_ignore_ascii_case(&item)) {
            items.push(item);
        }
    }
    items
}

fn clean_text(text: &str) -> String {
    text.trim().replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_minimal_dvd_profiler_export() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<Collection>
  <DVD>
    <ID>012236047605</ID>
    <MediaTypes><DVD>true</DVD><BluRay>false</BluRay><UltraHD>false</UltraHD></MediaTypes>
    <UPC>012236-047605</UPC>
    <Title>Bad Lieutenant</Title>
    <SortTitle>Bad Lieutenant</SortTitle>
    <ProductionYear>1992</ProductionYear>
    <RunningTime>96</RunningTime>
    <Rating>NC-17</Rating>
    <Genres><Genre>Drama</Genre><Genre>Crime</Genre></Genres>
    <Studios><Studio>Lt. Productions</Studio></Studios>
    <Overview>He has survived on the streets...</Overview>
    <Actors>
      <Actor FirstName="Harvey" LastName="Keitel" Role="LT" Voice="false" Uncredited="false" />
      <Divider Caption="Others" Type="Group" />
      <Actor FirstName="Chris" LastName="Russo" Role="" Voice="true" Uncredited="false" />
    </Actors>
    <Credits>
      <Credit FirstName="Abel" LastName="Ferrara" CreditType="Direction" CreditSubtype="Director" />
      <Credit FirstName="Zoe" LastName="Lund" CreditType="Writing" CreditSubtype="Writer" />
    </Credits>
    <Audio><AudioTrack><AudioContent>English</AudioContent><AudioFormat>Dolby Digital</AudioFormat><AudioChannels>Dolby Surround</AudioChannels></AudioTrack></Audio>
    <Subtitles><Subtitle>Spanish</Subtitle></Subtitles>
    <Discs><Disc><Location>Shelf A</Location></Disc></Discs>
    <CollectionNumber>268</CollectionNumber>
    <CaseType>Keep Case</CaseType>
    <PurchaseInfo>
      <PurchasePrice DenominationType="USD">12.99</PurchasePrice>
      <PurchaseDate>2008-05-20</PurchaseDate>
      <PurchasePlace>Amazon</PurchasePlace>
    </PurchaseInfo>
  </DVD>
  <DVD>
    <Title></Title>
  </DVD>
</Collection>"#;

        let parsed = parse_collection(xml.as_bytes()).expect("should parse");
        assert_eq!(parsed.movies.len(), 1);
        assert_eq!(parsed.skipped, 1);

        let movie = &parsed.movies[0];
        assert_eq!(movie.id, "dvdprofiler:012236047605");
        assert_eq!(movie.title, "Bad Lieutenant");
        assert_eq!(movie.format, DiscFormat::Dvd);
        assert_eq!(movie.year, Some(1992));
        assert_eq!(movie.runtime_minutes, Some(96));
        assert_eq!(movie.genres, vec!["Drama", "Crime"]);
        assert_eq!(movie.directors, vec!["Abel Ferrara"]);
        assert_eq!(movie.writers, vec!["Zoe Lund"]);
        assert_eq!(movie.actors.len(), 2);
        assert_eq!(movie.actors[0].name, "Harvey Keitel");
        assert_eq!(movie.actors[0].role, "LT");
        assert!(movie.actors[1].voice);
        assert_eq!(
            movie.audio,
            vec!["English · Dolby Digital · Dolby Surround"]
        );
        assert_eq!(movie.disc_count, 1);
        assert_eq!(movie.location, "Shelf A");
        assert_eq!(movie.purchase_price, "12.99 USD");
        assert_eq!(movie.purchase_place, "Amazon");
    }

    #[test]
    fn decodes_windows_1252_files() {
        let mut xml =
            b"<?xml version=\"1.0\" encoding=\"windows-1252\"?><Collection><DVD><Title>Am".to_vec();
        xml.push(0xE9); // é in Windows-1252
        xml.extend_from_slice(b"lie</Title></DVD></Collection>");

        let parsed = parse_collection(&xml).expect("should parse");
        assert_eq!(parsed.movies[0].title, "Amélie");
    }

    #[test]
    fn detects_blu_ray_and_4k_media_types() {
        let xml = r#"<Collection>
          <DVD><Title>Blu</Title><MediaTypes><DVD>false</DVD><BluRay>true</BluRay><UltraHD>false</UltraHD></MediaTypes></DVD>
          <DVD><Title>Four K</Title><MediaTypes><DVD>false</DVD><BluRay>true</BluRay><UltraHD>true</UltraHD></MediaTypes></DVD>
        </Collection>"#;
        let parsed = parse_collection(xml.as_bytes()).expect("should parse");
        assert_eq!(parsed.movies[0].format, DiscFormat::BluRay);
        assert_eq!(parsed.movies[1].format, DiscFormat::UltraHd);
    }

    #[test]
    fn rejects_files_without_movies() {
        let xml = b"<Collection><DVD><Title> </Title></DVD></Collection>";
        assert!(parse_collection(xml).is_err());
    }
}
