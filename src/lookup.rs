use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use encoding_rs::{Encoding, WINDOWS_1252};
use flate2::read::ZlibDecoder;

use crate::covers;
use crate::model::DiscFormat;

const BASE_URL: &str = "http://www.invelos.com";
const REDIRECTS_URL: &str = "http://www.invelos.com/AppData/DVDPro/redirects.inv";
/// Match the legacy embedded-browser user-agent observed in the DVD Profiler trace.
const USER_AGENT: &str = "Mozilla/4.0 (compatible; MSIE 8.0; Windows NT 6.2; WOW64; Trident/7.0; .NET4.0C; .NET4.0E; .NET CLR 2.0.50727; .NET CLR 3.0.30729; .NET CLR 3.5.30729; Zoom 3.6.0)";
const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;
const TIMEOUT_SECONDS: u64 = 20;

/// The server does not expose these request tokens in the .doi file or preview
/// HTML; they were captured from DVD Profiler itself.
const CAPTURED_HQ_IMAGE_TOKENS: &[(&str, &str, &str)] = &[(
    "7332431025318.16",
    "RWDRA4",
    "A3167EE6E7B7A48AD8C3CC8D770CDC24",
)];

/// Movie details returned by the Invelos lookup.
///
/// Only the fields that the manual editor can hold are kept; the rest of the
/// profile data is deliberately not exposed by this app.
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

/// Looks up a barcode against the Invelos profile service.
///
/// This is a native translation of the reference Invelos client protocol:
/// it reads `redirects.inv`, downloads and decodes the compressed `.doi`
/// profile, parses the public preview page, and saves any available high
/// quality front/back cover JPEGs.
pub fn fetch(upc: &str, locality: &str) -> Result<MovieDetails, String> {
    fetch_with_cover_dir(upc, locality, &covers::directory())
}

fn fetch_with_cover_dir(
    upc: &str,
    locality: &str,
    cover_directory: &Path,
) -> Result<MovieDetails, String> {
    let upc = normalize_upc(upc)?;
    let locality = normalize_locality(locality)?;
    let profile_id = format!("{upc}.{locality}");

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(TIMEOUT_SECONDS))
        .redirects(10)
        .user_agent(USER_AGENT)
        .build();

    let (redirects, redirects_charset) = request_bytes(&agent, REDIRECTS_URL, &[])?;
    let profiles_server = server_for_type(&redirects, &redirects_charset, "Profiles")?;
    let hq_images_server = server_for_type(&redirects, &redirects_charset, "HQImages").ok();

    let doi_path = format!("AppData/DVDPro/data/{}/{profile_id}.doi", &upc[..2]);
    let profile_download_url = format!("{profiles_server}{doi_path}");
    let (doi_bytes, _) = request_bytes(&agent, &profile_download_url, &[])?;
    let profile = decode_profile_doi(&doi_bytes, &profile_id)?;

    let profile_url = format!("{BASE_URL}/ProfilePreview.aspx?id={profile_id}&t=u");
    let (page_bytes, page_charset) = request_bytes(&agent, &profile_url, &[])?;
    let page = decode_with_charset(&page_bytes, &page_charset);
    let preview = parse_preview(&page);

    let expected = [
        "Locality",
        "Produced",
        "Media Type",
        "Directed by",
        "Starring",
    ];
    if !expected
        .iter()
        .any(|label| preview.fields.contains_key(*label))
    {
        return Err(
            "Invelos returned a page, but it did not contain the expected preview fields. \
             The barcode/locality may not match, or the server page may have changed."
                .to_string(),
        );
    }

    let downloaded = hq_images_server
        .as_deref()
        .map(|server| fetch_hq_covers(&agent, server, &profile_id, &profile_url))
        .unwrap_or_default();
    let (front_cover, back_cover) = save_covers(&profile_id, downloaded, cover_directory)?;

    details_from_profile(&profile, &preview, &upc, front_cover, back_cover)
}

fn normalize_upc(value: &str) -> Result<String, String> {
    let upc: String = value
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '-')
        .collect();
    if upc.len() < 8 || upc.len() > 14 || !upc.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(
            "Enter a barcode containing 8 to 14 digits (spaces or hyphens are okay).".to_string(),
        );
    }
    Ok(upc)
}

fn normalize_locality(value: &str) -> Result<String, String> {
    let value = value.trim();
    let number: u32 = value.parse().map_err(|_| {
        "Locality ID must be a positive number. The captured Finland example uses 16.".to_string()
    })?;
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || !(1..=999).contains(&number)
    {
        return Err(
            "Locality ID must be a positive number. The captured Finland example uses 16."
                .to_string(),
        );
    }
    Ok(number.to_string())
}

fn request_bytes(
    agent: &ureq::Agent,
    url: &str,
    extra_headers: &[(&str, &str)],
) -> Result<(Vec<u8>, String), String> {
    let mut request = agent
        .get(url)
        .set("Accept", "*/*")
        .set("Accept-Language", "en-FI");
    for (name, value) in extra_headers {
        request = request.set(name, value);
    }

    let response = match request.call() {
        Ok(response) => response,
        Err(ureq::Error::Status(code, _)) => {
            return Err(format!(
                "Invelos returned HTTP {code} for {}.",
                url_path(url)
            ));
        }
        Err(ureq::Error::Transport(error)) => {
            return Err(format!("Could not reach Invelos: {error}"));
        }
    };

    let status = response.status();
    if status != 200 {
        return Err(format!(
            "Invelos returned HTTP {status} for {}.",
            url_path(url)
        ));
    }
    let charset = charset_from_content_type(response.header("content-type"));
    let mut data = Vec::new();
    response
        .into_reader()
        .take(MAX_PAGE_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|error| format!("Could not read the Invelos response: {error}"))?;
    if data.len() > MAX_PAGE_BYTES {
        return Err(
            "Invelos returned an unexpectedly large response; it was not processed.".to_string(),
        );
    }
    Ok((data, charset))
}

/// Keeps errors useful without echoing query parameters.
fn url_path(url: &str) -> &str {
    let after_scheme = url
        .find("://")
        .map(|index| &url[index + 3..])
        .unwrap_or(url);
    let path = after_scheme
        .find('/')
        .map(|index| &after_scheme[index..])
        .unwrap_or("/");
    path.split(['?', '#']).next().unwrap_or("/")
}

fn charset_from_content_type(value: Option<&str>) -> String {
    let Some(value) = value else {
        return "utf-8".to_string();
    };
    for part in value.split(';') {
        let Some((name, value)) = part.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("charset") {
            return value.trim().trim_matches('"').to_string();
        }
    }
    "utf-8".to_string()
}

fn decode_with_charset(bytes: &[u8], charset: &str) -> String {
    let encoding = Encoding::for_label(charset.trim().as_bytes()).unwrap_or(encoding_rs::UTF_8);
    let (text, _, _) = encoding.decode(bytes);
    text.into_owned()
}

fn server_for_type(bytes: &[u8], charset: &str, site_type: &str) -> Result<String, String> {
    let text = decode_with_charset(bytes, charset);
    let document = roxmltree::Document::parse(&text)
        .map_err(|_| "Invelos returned an unreadable redirects.inv file.".to_string())?;
    for node in document.descendants().filter(|node| node.is_element()) {
        if !node.tag_name().name().eq_ignore_ascii_case("site") {
            continue;
        }
        if !node
            .attribute("Type")
            .is_some_and(|value| value.eq_ignore_ascii_case(site_type))
        {
            continue;
        }
        let server = node.attribute("Server").unwrap_or("").trim();
        if server.starts_with("http://") || server.starts_with("https://") {
            return Ok(format!("{}/", server.trim_end_matches('/')));
        }
    }
    Err(format!(
        "Invelos did not provide a {site_type} server in redirects.inv."
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProfileData {
    profile_id: String,
    title: String,
    sort_title: String,
    production_year: String,
    rating: String,
    runtime_minutes: String,
    cast: Vec<CastCredit>,
    crew: Vec<CrewCredit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CastCredit {
    name: String,
    role: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CrewCredit {
    name: String,
    credit_type_id: u32,
}

fn decode_profile_doi(data: &[u8], expected_id: &str) -> Result<ProfileData, String> {
    let decompressed = decompress_wrapper(data, "doi", "profile file")?;
    let records = parse_records(&decompressed);
    if records.is_empty() {
        return Err("The Invelos .doi file did not contain profile fields.".to_string());
    }

    let text_for = |tag: u16| -> Vec<String> {
        records
            .iter()
            .filter(|(field_tag, _)| *field_tag == tag)
            .map(|(_, raw)| decode_cp1252(raw))
            .collect()
    };
    let first_or_empty =
        |tag: u16| -> String { text_for(tag).into_iter().next().unwrap_or_default() };

    let tag_one = text_for(1);
    let profile_id = tag_one.first().cloned().unwrap_or_default();
    if profile_id != expected_id {
        return Err(
            "The downloaded .doi profile ID did not match the requested barcode and locality."
                .to_string(),
        );
    }
    let title = tag_one.get(1).cloned().unwrap_or_default();
    if title.is_empty() {
        return Err("The Invelos .doi file did not contain the expected title field.".to_string());
    }

    let cast = text_for(45)
        .iter()
        .filter_map(|raw| parse_cast_credit(raw))
        .collect();
    let crew = text_for(107)
        .iter()
        .filter_map(|raw| parse_crew_credit(raw))
        .collect();

    Ok(ProfileData {
        profile_id,
        title,
        sort_title: first_or_empty(97),
        production_year: first_or_empty(2),
        rating: first_or_empty(145),
        runtime_minutes: first_or_empty(4),
        cast,
        crew,
    })
}

fn parse_records(data: &[u8]) -> Vec<(u16, &[u8])> {
    let mut records = Vec::new();
    let mut offset = 0;
    while offset + 4 <= data.len() {
        let tag = u16::from_le_bytes([data[offset], data[offset + 1]]);
        let length = u16::from_le_bytes([data[offset + 2], data[offset + 3]]) as usize;
        let value_start = offset + 4;
        let Some(value_end) = value_start.checked_add(length) else {
            break;
        };
        if value_end > data.len() {
            break;
        }
        records.push((tag, &data[value_start..value_end]));
        offset = value_end;
    }
    records
}

fn decode_cp1252(raw: &[u8]) -> String {
    let (text, _, _) = WINDOWS_1252.decode(raw);
    text.trim_end_matches('\0').trim().to_string()
}

fn parse_cast_credit(raw: &str) -> Option<CastCredit> {
    let parts: Vec<&str> = raw.splitn(4, '"').collect();
    if parts.len() != 4 {
        return None;
    }
    let (role, _) = parts[3].split_once('\u{ff}').unwrap_or((parts[3], ""));
    let name = [parts[0], parts[1], parts[2]]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    Some(CastCredit {
        name,
        role: role.to_string(),
    })
}

fn parse_crew_credit(raw: &str) -> Option<CrewCredit> {
    let parts: Vec<&str> = raw.splitn(5, '"').collect();
    if parts.len() != 5 {
        return None;
    }
    let credit_type_id = parts[2].trim().parse().ok()?;
    let _credit_subtype_id: u32 = parts[3].trim().parse().ok()?;
    let name = [parts[0], parts[1]]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    Some(CrewCredit {
        name,
        credit_type_id,
    })
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Preview {
    fields: BTreeMap<String, String>,
    synopsis: String,
}

/// Reads the visible fields from Invelos' `ProfilePreview.aspx` HTML.
fn parse_preview(html: &str) -> Preview {
    let mut preview = Preview::default();
    let mut paragraph: Option<String> = None;
    let mut synopsis_depth = 0usize;
    let mut synopsis = String::new();
    let mut position = 0;

    while position < html.len() {
        let rest = &html[position..];
        let Some(lt) = rest.find('<') else {
            push_text(rest, paragraph.as_mut(), synopsis_depth, &mut synopsis);
            break;
        };
        if lt > 0 {
            push_text(
                &rest[..lt],
                paragraph.as_mut(),
                synopsis_depth,
                &mut synopsis,
            );
            position += lt;
        }

        let Some(end) = find_tag_end(html, position) else {
            break;
        };
        let raw = html[position + 1..end].trim();
        position = end + 1;

        if raw.starts_with('!') || raw.starts_with('?') {
            continue;
        }
        let closing = raw.starts_with('/');
        let body = raw.trim_start_matches('/').trim_start();
        let name = body
            .split(|character: char| {
                character.is_whitespace() || character == '/' || character == '>'
            })
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();

        match (closing, name.as_str()) {
            (true, "p") => {
                if let Some(text) = paragraph.take() {
                    let value = collapse_whitespace(&decode_entities(&text));
                    if let Some((label, value)) = value.split_once(':') {
                        preview
                            .fields
                            .insert(label.trim().to_string(), value.trim().to_string());
                    }
                }
            }
            (true, "div") => {
                synopsis_depth = synopsis_depth.saturating_sub(1);
            }
            (false, "p") => paragraph = Some(String::new()),
            (false, "div") => {
                if attribute_value(body, "style")
                    .is_some_and(|style| style.to_ascii_lowercase().contains("overflow"))
                {
                    synopsis_depth += 1;
                }
            }
            (false, "script") | (false, "style") => {
                // Skip raw-text elements so a stray '<' in script content does
                // not confuse the tokenizer.
                let close = format!("</{name}");
                if let Some(index) = html[position..].to_ascii_lowercase().find(&close) {
                    position += index;
                }
            }
            _ => {}
        }
    }

    preview.synopsis = collapse_whitespace(&decode_entities(&synopsis));
    preview
}

fn push_text(
    text: &str,
    paragraph: Option<&mut String>,
    synopsis_depth: usize,
    synopsis: &mut String,
) {
    if let Some(paragraph) = paragraph {
        paragraph.push_str(text);
    }
    if synopsis_depth > 0 {
        synopsis.push_str(text);
    }
}

fn find_tag_end(html: &str, start: usize) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut quote: Option<u8> = None;
    let mut index = start + 1;
    while index < bytes.len() {
        let byte = bytes[index];
        match quote {
            Some(active) if byte == active => quote = None,
            Some(_) => {}
            None if byte == b'"' || byte == b'\'' => quote = Some(byte),
            None if byte == b'>' => return Some(index),
            None => {}
        }
        index += 1;
    }
    None
}

fn attribute_value(body: &str, wanted: &str) -> Option<String> {
    let bytes = body.as_bytes();
    let mut index = 0;
    // Skip the element name.
    while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
        index += 1;
    }

    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let name_start = index;
        while index < bytes.len()
            && !bytes[index].is_ascii_whitespace()
            && bytes[index] != b'='
            && bytes[index] != b'/'
        {
            index += 1;
        }
        let name = &body[name_start..index];
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            if name.is_empty() {
                break;
            }
            continue;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let value = if bytes[index] == b'"' || bytes[index] == b'\'' {
            let quote = bytes[index];
            index += 1;
            let start = index;
            while index < bytes.len() && bytes[index] != quote {
                index += 1;
            }
            let value = &body[start..index];
            index += usize::from(index < bytes.len());
            value
        } else {
            let start = index;
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            &body[start..index]
        };
        if name.eq_ignore_ascii_case(wanted) {
            return Some(value.to_string());
        }
    }
    None
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_entities(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut index = 0;
    while index < value.len() {
        let rest = &value[index..];
        let Some(amp) = rest.find('&') else {
            output.push_str(rest);
            break;
        };
        output.push_str(&rest[..amp]);
        index += amp;
        let after = &value[index + 1..];
        let Some(semicolon) = after.find(';') else {
            output.push('&');
            index += 1;
            continue;
        };
        if semicolon > 32 {
            output.push('&');
            index += 1;
            continue;
        }
        let entity = &after[..semicolon];
        let decoded = if let Some(number) = entity.strip_prefix('#') {
            let number = if let Some(hex) = number.strip_prefix(['x', 'X']) {
                u32::from_str_radix(hex, 16).ok()
            } else {
                number.parse::<u32>().ok()
            };
            number.and_then(char::from_u32)
        } else {
            Some(match entity {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => '\u{a0}',
                "copy" => '\u{a9}',
                "reg" => '\u{ae}',
                "trade" => '\u{2122}',
                "hellip" => '\u{2026}',
                "mdash" => '\u{2014}',
                "ndash" => '\u{2013}',
                "lsquo" => '\u{2018}',
                "rsquo" => '\u{2019}',
                "ldquo" => '\u{201c}',
                "rdquo" => '\u{201d}',
                "eacute" => '\u{e9}',
                "egrave" => '\u{e8}',
                "agrave" => '\u{e0}',
                "auml" => '\u{e4}',
                "ouml" => '\u{f6}',
                "uuml" => '\u{fc}',
                "szlig" => '\u{df}',
                _ => {
                    output.push('&');
                    index += 1;
                    continue;
                }
            })
        };
        match decoded {
            Some(character) => {
                output.push(character);
                index += semicolon + 2;
            }
            None => {
                output.push('&');
                index += 1;
            }
        }
    }
    output
}

#[derive(Debug, Default)]
struct CoverDownloads {
    front: Option<Vec<u8>>,
    back: Option<Vec<u8>>,
}

fn fetch_hq_covers(
    agent: &ureq::Agent,
    hq_images_server: &str,
    profile_id: &str,
    profile_url: &str,
) -> CoverDownloads {
    let Some((route, hash)) = CAPTURED_HQ_IMAGE_TOKENS
        .iter()
        .find(|(id, _, _)| *id == profile_id)
        .map(|(_, route, hash)| (*route, *hash))
    else {
        return CoverDownloads::default();
    };

    let url = format!(
        "{hq_images_server}dvdpro/GetHQImages.aspx?r={route}&h={hash}&file={profile_id}.dos"
    );
    let Ok((data, _)) = request_bytes(agent, &url, &[("Referer", profile_url)]) else {
        return CoverDownloads::default();
    };
    let Ok((front, back)) = decode_dos(&data) else {
        return CoverDownloads::default();
    };
    CoverDownloads {
        front: Some(front),
        back: Some(back),
    }
}

fn save_covers(
    profile_id: &str,
    covers: CoverDownloads,
    directory: &Path,
) -> Result<(bool, bool), String> {
    if covers.front.is_none() && covers.back.is_none() {
        return Ok((false, false));
    }

    std::fs::create_dir_all(directory).map_err(|error| {
        format!(
            "Could not create the cover folder {}: {error}",
            directory.display()
        )
    })?;

    let mut saved = (false, false);
    if let Some(bytes) = covers.front {
        let path = directory.join(format!("{profile_id}_front.jpg"));
        std::fs::write(&path, bytes).map_err(|error| {
            format!("Could not save the front cover {}: {error}", path.display())
        })?;
        saved.0 = true;
    }
    if let Some(bytes) = covers.back {
        let path = directory.join(format!("{profile_id}_back.jpg"));
        std::fs::write(&path, bytes).map_err(|error| {
            format!("Could not save the back cover {}: {error}", path.display())
        })?;
        saved.1 = true;
    }
    Ok(saved)
}

/// Decodes the observed wrapper: total size, vendor length, vendor marker, then
/// a zlib stream.
fn decompress_wrapper(data: &[u8], extension: &str, description: &str) -> Result<Vec<u8>, String> {
    if data.len() < 8 {
        return Err(format!(
            "The Invelos .{extension} {description} is too short."
        ));
    }
    let declared_size = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let vendor_length = u16::from_le_bytes([data[4], data[5]]) as usize;
    let compressed_offset = 6 + vendor_length;
    if compressed_offset > data.len()
        || declared_size != data.len()
        || &data[6..compressed_offset] != b"Invelos Software"
    {
        return Err(format!(
            "The Invelos .{extension} file has an unexpected header."
        ));
    }

    let mut decompressed = Vec::new();
    ZlibDecoder::new(&data[compressed_offset..])
        .read_to_end(&mut decompressed)
        .map_err(|_| format!("The Invelos .{extension} data could not be decompressed."))?;
    Ok(decompressed)
}

/// Decodes the `.dos` wrapper into its length-prefixed front/back JPEGs.
fn decode_dos(data: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    let cover_data = decompress_wrapper(data, "dos", "cover archive")?;
    let mut images: Vec<&[u8]> = Vec::new();
    let mut offset = 0;
    while offset < cover_data.len() {
        if offset + 4 > cover_data.len() {
            return Err("The Invelos .dos archive ended before an image length.".to_string());
        }
        let length =
            u32::from_le_bytes(cover_data[offset..offset + 4].try_into().unwrap()) as usize;
        let start = offset + 4;
        let Some(end) = start.checked_add(length) else {
            return Err("The Invelos .dos archive contains an invalid image length.".to_string());
        };
        if length == 0 || end > cover_data.len() {
            return Err("The Invelos .dos archive contains an invalid image length.".to_string());
        }
        let image = &cover_data[start..end];
        if !(image.starts_with(b"\xff\xd8\xff") && image.ends_with(b"\xff\xd9")) {
            return Err(
                "The Invelos .dos archive contains data that is not a complete JPEG.".to_string(),
            );
        }
        images.push(image);
        offset = end;
    }

    if images.len() != 2 {
        return Err(format!(
            "Expected front and back JPEGs in the .dos archive; received {} image(s).",
            images.len()
        ));
    }
    Ok((images[0].to_vec(), images[1].to_vec()))
}

fn details_from_profile(
    profile: &ProfileData,
    preview: &Preview,
    upc: &str,
    front_cover: bool,
    back_cover: bool,
) -> Result<MovieDetails, String> {
    let title = profile.title.trim().to_string();
    if title.is_empty() {
        return Err("The Invelos profile did not contain a title.".to_string());
    }
    let sort_title = if profile.sort_title.trim().is_empty() {
        title.clone()
    } else {
        profile.sort_title.trim().to_string()
    };

    let mut directors = unique_names(
        profile
            .crew
            .iter()
            .filter(|credit| credit.credit_type_id == 0)
            .map(|credit| credit.name.as_str()),
    );
    if directors.is_empty() {
        directors = preview
            .fields
            .get("Directed by")
            .map(|value| unique_names(value.split(',').map(str::trim)))
            .unwrap_or_default();
    }

    let mut cast: Vec<(String, String)> = Vec::new();
    for credit in &profile.cast {
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
        cast = preview
            .fields
            .get("Starring")
            .map(|value| {
                unique_names(value.split(',').map(str::trim))
                    .into_iter()
                    .map(|name| (name, String::new()))
                    .collect()
            })
            .unwrap_or_default();
    }

    let format = preview
        .fields
        .get("Media Type")
        .map(|media_type| format_from_media_type(media_type))
        .unwrap_or_default();

    let year = parse_number::<u16>(&profile.production_year)
        .or_else(|| {
            preview
                .fields
                .get("Produced")
                .and_then(|value| parse_number(value))
        })
        .filter(|year| (1888..=2200).contains(year));
    let runtime_minutes = parse_number::<u32>(&profile.runtime_minutes);

    Ok(MovieDetails {
        upc: upc.to_string(),
        profile_id: profile.profile_id.clone(),
        title,
        sort_title,
        format,
        year,
        runtime_minutes,
        rating: profile.rating.trim().to_string(),
        directors,
        cast,
        synopsis: preview.synopsis.trim().to_string(),
        front_cover,
        back_cover,
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

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write;

    fn wrapped(extension: &str, payload: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(payload).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut data = Vec::new();
        let total = 6 + b"Invelos Software".len() + compressed.len();
        data.extend_from_slice(&(total as u32).to_le_bytes());
        data.extend_from_slice(&(b"Invelos Software".len() as u16).to_le_bytes());
        data.extend_from_slice(b"Invelos Software");
        data.extend_from_slice(&compressed);
        assert!(extension == "doi" || extension == "dos");
        data
    }

    fn doi_record(tag: u16, value: &str) -> Vec<u8> {
        let bytes = value.as_bytes();
        let mut record = Vec::new();
        record.extend_from_slice(&tag.to_le_bytes());
        record.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
        record.extend_from_slice(bytes);
        record
    }

    fn sample_doi() -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend(doi_record(1, "044007867020.16"));
        payload.extend(doi_record(1, "Ihmissusi Lontoossa"));
        payload.extend(doi_record(117, "An American Werewolf in London"));
        payload.extend(doi_record(97, "Ihmissusi Lontoossa"));
        payload.extend(doi_record(2, "1981"));
        payload.extend(doi_record(145, "18"));
        payload.extend(doi_record(4, "93"));
        payload.extend(doi_record(45, "David\"\"Naughton\"David Kessler"));
        payload.extend(doi_record(45, "Jenny\"\"Agutter\"Nurse Alex Price"));
        payload.extend(doi_record(45, "David\"\"Naughton\"Duplicate"));
        payload.extend(doi_record(107, "John\"Landis\"0\"0\""));
        payload.extend(doi_record(107, "John\"Landis\"1\"1\""));
        wrapped("doi", &payload)
    }

    #[test]
    fn decodes_profile_fields_and_credits() {
        let profile = decode_profile_doi(&sample_doi(), "044007867020.16").unwrap();
        assert_eq!(profile.profile_id, "044007867020.16");
        assert_eq!(profile.title, "Ihmissusi Lontoossa");
        assert_eq!(profile.sort_title, "Ihmissusi Lontoossa");
        assert_eq!(profile.production_year, "1981");
        assert_eq!(profile.rating, "18");
        assert_eq!(profile.runtime_minutes, "93");
        assert_eq!(
            profile.cast,
            vec![
                CastCredit {
                    name: "David Naughton".to_string(),
                    role: "David Kessler".to_string(),
                },
                CastCredit {
                    name: "Jenny Agutter".to_string(),
                    role: "Nurse Alex Price".to_string(),
                },
                CastCredit {
                    name: "David Naughton".to_string(),
                    role: "Duplicate".to_string(),
                },
            ]
        );
        assert_eq!(
            profile.crew,
            vec![
                CrewCredit {
                    name: "John Landis".to_string(),
                    credit_type_id: 0,
                },
                CrewCredit {
                    name: "John Landis".to_string(),
                    credit_type_id: 1,
                },
            ]
        );
    }

    #[test]
    fn rejects_mismatched_profile_ids_and_bad_headers() {
        assert!(decode_profile_doi(&sample_doi(), "000000000000.16").is_err());
        let mut bad = sample_doi();
        bad[0] ^= 0xff;
        assert!(decode_profile_doi(&bad, "044007867020.16").is_err());
    }

    #[test]
    fn parses_preview_fields_synopsis_and_entities() {
        let html = r#"
            <html><head><script>if (a < b) { x(); }</script></head><body>
            <table><tr><td>
                <p>Locality: Finland</p>
                <p>Produced: 1981</p>
                <p>Media Type: DVD &amp; Blu-ray</p>
                <p>Directed by: John Landis</p>
                <p>Starring: David Naughton, Jenny Agutter</p>
            </td>
            <td><div style="height: 130px; overflow: auto;">
                Two backpackers&#39; trip goes wrong &amp; becomes strange.
            </div></td></tr></table>
            </body></html>
        "#;
        let preview = parse_preview(html);
        assert_eq!(preview.fields.get("Locality").unwrap(), "Finland");
        assert_eq!(preview.fields.get("Produced").unwrap(), "1981");
        assert_eq!(preview.fields.get("Media Type").unwrap(), "DVD & Blu-ray");
        assert_eq!(preview.fields.get("Directed by").unwrap(), "John Landis");
        assert_eq!(
            preview.synopsis,
            "Two backpackers' trip goes wrong & becomes strange."
        );
    }

    #[test]
    fn reads_redirect_servers() {
        let xml = br#"<Sites>
            <Site Type="HQImages" Server="http://images.example/" />
            <Site Type="Profiles" Server="https://profiles.example" />
        </Sites>"#;
        assert_eq!(
            server_for_type(xml, "utf-8", "Profiles").unwrap(),
            "https://profiles.example/"
        );
        assert_eq!(
            server_for_type(xml, "utf-8", "HQImages").unwrap(),
            "http://images.example/"
        );
        assert!(server_for_type(xml, "utf-8", "Other").is_err());
        assert!(server_for_type(b"not xml", "utf-8", "Profiles").is_err());
    }

    #[test]
    fn decodes_dos_front_and_back_jpegs() {
        let front = b"\xff\xd8\xfffront\xff\xd9";
        let back = b"\xff\xd8\xffback\xff\xd9";
        let mut payload = Vec::new();
        payload.extend_from_slice(&(front.len() as u32).to_le_bytes());
        payload.extend_from_slice(front);
        payload.extend_from_slice(&(back.len() as u32).to_le_bytes());
        payload.extend_from_slice(back);
        let data = wrapped("dos", &payload);
        let (decoded_front, decoded_back) = decode_dos(&data).unwrap();
        assert_eq!(decoded_front, front);
        assert_eq!(decoded_back, back);
    }

    #[test]
    fn maps_profile_to_editor_details() {
        let profile = decode_profile_doi(&sample_doi(), "044007867020.16").unwrap();
        let html = r#"
            <p>Locality: Finland</p>
            <p>Produced: 1981</p>
            <p>Media Type: DVD</p>
            <p>Directed by: John Landis</p>
            <p>Starring: David Naughton, Jenny Agutter</p>
            <div style="overflow: auto;">Two backpackers run into trouble on the moors.</div>
        "#;
        let preview = parse_preview(html);
        let details =
            details_from_profile(&profile, &preview, "044007867020", true, false).unwrap();
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
        assert_eq!(
            details.synopsis,
            "Two backpackers run into trouble on the moors."
        );
        assert!(details.front_cover);
        assert!(!details.back_cover);
    }

    #[test]
    fn infers_formats_and_falls_back_to_preview_credits() {
        assert_eq!(format_from_media_type("DVD"), DiscFormat::Dvd);
        assert_eq!(format_from_media_type("Blu-ray Disc"), DiscFormat::BluRay);
        assert_eq!(format_from_media_type("4K Ultra HD"), DiscFormat::UltraHd);
        assert_eq!(format_from_media_type("UHD Blu-ray"), DiscFormat::UltraHd);

        let profile = ProfileData {
            profile_id: "123456789012.16".to_string(),
            title: "Fallback".to_string(),
            sort_title: String::new(),
            production_year: String::new(),
            rating: String::new(),
            runtime_minutes: String::new(),
            cast: Vec::new(),
            crew: Vec::new(),
        };
        let preview = parse_preview(
            "<p>Produced: 1999</p><p>Directed by: First Director, Second Director</p>\
             <p>Starring: One Actor, Two Actor</p>",
        );
        let details =
            details_from_profile(&profile, &preview, "123456789012", false, false).unwrap();
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
    fn validates_barcodes_and_localities() {
        assert_eq!(normalize_upc(" 044007-867020 ").unwrap(), "044007867020");
        assert!(normalize_upc("123").is_err());
        assert!(normalize_upc("").is_err());
        assert!(normalize_upc("12345678x").is_err());
        assert_eq!(normalize_locality("016").unwrap(), "16");
        assert!(normalize_locality("0").is_err());
        assert!(normalize_locality("1000").is_err());
        assert!(normalize_locality("abc").is_err());
    }

    #[test]
    fn extracts_the_path_from_urls() {
        assert_eq!(
            url_path("http://www.invelos.com/AppData/x.doi?q=1"),
            "/AppData/x.doi"
        );
        assert_eq!(url_path("http://example.com"), "/");
    }

    #[test]
    #[ignore = "contacts the live Invelos service"]
    fn live_lookup_returns_the_expected_profile() {
        let directory = temp_dir("live-reds");
        let details = fetch_with_cover_dir("7332431025318", "16", &directory).unwrap();
        println!("{details:#?}");
        assert_eq!(details.profile_id, "7332431025318.16");
        assert_eq!(details.title, "Punaiset");
        assert_eq!(details.sort_title, "Punaiset");
        assert_eq!(details.format, DiscFormat::Dvd);
        assert_eq!(details.year, Some(1981));
        assert_eq!(details.runtime_minutes, Some(188));
        assert_eq!(details.rating, "15");
        assert_eq!(details.directors, vec!["Warren Beatty"]);
        assert_eq!(
            details.cast.first().map(|(name, _)| name.as_str()),
            Some("Warren Beatty")
        );
        assert!(details.synopsis.starts_with("Tositapahtumiin perustuvassa"));
        assert!(details.front_cover);
        assert!(details.back_cover);
        assert!(directory.join("7332431025318.16_front.jpg").is_file());
        assert!(directory.join("7332431025318.16_back.jpg").is_file());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    #[ignore = "contacts the live Invelos service"]
    fn live_lookup_without_captured_cover_tokens_still_succeeds() {
        let directory = temp_dir("live-contact");
        let details = fetch_with_cover_dir("7321944150419", "16", &directory).unwrap();
        assert_eq!(details.profile_id, "7321944150419.16");
        assert_eq!(details.title, "Ensimmäinen yhteys");
        assert_eq!(details.format, DiscFormat::Dvd);
        assert_eq!(details.runtime_minutes, Some(144));
        assert!(!details.front_cover);
        assert!(!details.back_cover);
        assert!(!details.cast.is_empty());
        assert!(!directory.exists());
    }

    #[test]
    fn saves_covers_with_profile_based_file_names() {
        let directory = temp_dir("save-covers");
        let covers = CoverDownloads {
            front: Some(vec![1, 2, 3]),
            back: Some(vec![4, 5, 6]),
        };
        assert_eq!(
            save_covers("044007867020.16", covers, &directory).unwrap(),
            (true, true)
        );
        assert_eq!(
            std::fs::read(directory.join("044007867020.16_front.jpg")).unwrap(),
            vec![1, 2, 3]
        );
        assert_eq!(
            std::fs::read(directory.join("044007867020.16_back.jpg")).unwrap(),
            vec![4, 5, 6]
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("mymovies-{name}-{}", std::process::id()))
    }
}
