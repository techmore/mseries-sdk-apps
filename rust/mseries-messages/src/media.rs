//! Bounded public HTTPS previews for image, GIF, WebP, and YouTube links.
//!
//! This deliberately does not fetch MAP attachments or private-network URLs.

use image::metadata::Orientation;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use ipnet::IpNet;
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, LOCATION};
use reqwest::redirect::Policy;
use std::collections::HashSet;
use std::io::{BufReader, Cursor, Read};
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::OnceLock;
use std::time::Duration;
use url::{Host, Url};

pub const MAX_PREVIEW_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_DECODED_BYTES: usize = 12 * 1024 * 1024;
pub const MAX_SOURCE_PIXELS: u64 = 16_000_000;
pub const MAX_ANIMATED_FRAMES: usize = 120;
const THUMBNAIL_WIDTH: u32 = 280;
const THUMBNAIL_HEIGHT: u32 = 220;

#[derive(Clone, Debug)]
pub struct PreviewFrame {
    pub rgba: Vec<u8>,
    pub delay: Duration,
}

#[derive(Clone, Debug)]
pub struct LinkPreview {
    pub title: String,
    pub kind: PreviewKind,
    pub source_size: (u32, u32),
    pub size: (u32, u32),
    pub frames: Vec<PreviewFrame>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewKind {
    Image,
    Animation,
    Youtube,
}

/// Finds up to three distinct, supported HTTPS media links in a message.
pub fn media_links(body: &str) -> Vec<String> {
    if body.chars().any(is_bidi_control) {
        return Vec::new();
    }

    let mut links = Vec::new();
    let mut seen = HashSet::new();
    let mut from = 0;
    while links.len() < 3 {
        let Some(relative) = body[from..].find("https://") else {
            break;
        };
        let start = from + relative;
        if body[..start].chars().next_back().is_some_and(|previous| {
            previous.is_alphanumeric() || matches!(previous, '_' | '@' | '/')
        }) {
            from = start + "https://".len();
            continue;
        }
        let end = body[start..]
            .char_indices()
            .find(|(_, ch)| {
                ch.is_whitespace()
                    || ch.is_control()
                    || matches!(*ch, '<' | '>' | '"' | '\'' | '\u{201c}' | '\u{201d}')
            })
            .map(|(offset, _)| start + offset)
            .unwrap_or(body.len());
        from = end.max(start + "https://".len());
        let candidate = body[start..end].trim_end_matches(['.', ',', ';', '!']);
        if candidate.is_empty() || candidate.len() > 2048 || !seen.insert(candidate.to_string()) {
            continue;
        }
        let Ok(url) = Url::parse(candidate) else {
            continue;
        };
        if !is_safe_preview_url(&url) {
            continue;
        }
        let is_youtube = youtube_id(&url).is_some();
        let path = url.path().to_ascii_lowercase();
        if is_youtube
            || [".jpg", ".jpeg", ".png", ".gif", ".webp"]
                .iter()
                .any(|extension| path.ends_with(extension))
        {
            links.push(candidate.to_string());
        }
    }
    links
}

pub fn is_youtube_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| youtube_id(&url).is_some())
}

pub fn load_preview(value: &str) -> Result<LinkPreview, String> {
    let url = Url::parse(value).map_err(|_| "Invalid preview URL".to_string())?;
    if !is_safe_preview_url(&url) {
        return Err("Only public HTTPS preview links are supported".into());
    }

    if let Some(video_id) = youtube_id(&url) {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.append_pair("format", "json").append_pair(
            "url",
            &format!("https://www.youtube.com/watch?v={video_id}"),
        );
        let metadata_url = Url::parse(&format!(
            "https://www.youtube.com/oembed?{}",
            query.finish()
        ))
        .map_err(|_| "Invalid YouTube metadata URL".to_string())?;
        let (metadata, mime) = fetch_public_https(metadata_url)?;
        if metadata.len() > 262_144 || !mime.eq_ignore_ascii_case("application/json") {
            return Err("Invalid YouTube preview metadata".into());
        }
        let record: serde_json::Value = serde_json::from_slice(&metadata)
            .map_err(|_| "Invalid YouTube preview metadata".to_string())?;
        let thumbnail = record
            .get("thumbnail_url")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| Url::parse(value).ok())
            .ok_or_else(|| "YouTube did not provide a preview image".to_string())?;
        if !matches!(
            thumbnail.host_str(),
            Some("i.ytimg.com" | "img.youtube.com")
        ) {
            return Err("Unexpected YouTube preview image host".into());
        }
        let (data, mime) = fetch_public_https(thumbnail)?;
        if !mime.to_ascii_lowercase().starts_with("image/") {
            return Err("YouTube preview is not an image".into());
        }
        let title = record
            .get("title")
            .and_then(serde_json::Value::as_str)
            .filter(|title| !title.is_empty())
            .unwrap_or("YouTube")
            .chars()
            .filter(|character| !character.is_control() && !is_bidi_control(*character))
            .take(300)
            .collect();
        return decode_image(&data, title, PreviewKind::Youtube);
    }

    let (data, mime) = fetch_public_https(url)?;
    if !mime.to_ascii_lowercase().starts_with("image/") {
        return Err("This link is not an image".into());
    }
    let title = if mime.eq_ignore_ascii_case("image/gif") {
        "GIF"
    } else {
        "Photo"
    };
    decode_image(&data, title.into(), PreviewKind::Image)
}

fn fetch_public_https(mut url: Url) -> Result<(Vec<u8>, String), String> {
    for _ in 0..4 {
        let (client, request_url) = pinned_client(url)?;
        let mut response = client
            .get(request_url.clone())
            .header(ACCEPT_ENCODING, "identity")
            .send()
            .map_err(|_| "Preview request failed".to_string())?;

        if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| "Preview redirect has no destination".to_string())?;
            url = request_url
                .join(location)
                .map_err(|_| "Preview redirect has an invalid destination".to_string())?;
            continue;
        }
        if response.status().as_u16() != 200 {
            return Err("Preview server did not return content".into());
        }
        let content_encoding = response
            .headers()
            .get(CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("identity");
        if !content_encoding.eq_ignore_ascii_case("identity") {
            return Err("Compressed preview responses are not supported".into());
        }
        if response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|length| length > MAX_PREVIEW_BYTES as u64)
        {
            return Err("Preview is too large".into());
        }
        let mime = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        let mut body = Vec::new();
        response
            .by_ref()
            .take((MAX_PREVIEW_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .map_err(|_| "Preview download failed".to_string())?;
        if body.len() > MAX_PREVIEW_BYTES {
            return Err("Preview is too large".into());
        }
        return Ok((body, mime));
    }
    Err("Too many preview redirects".into())
}

fn pinned_client(url: Url) -> Result<(Client, Url), String> {
    if !is_safe_preview_url(&url) {
        return Err("Only public HTTPS preview links are supported".into());
    }
    let host = url
        .host()
        .ok_or_else(|| "Preview URL has no host".to_string())?;
    let mut builder = Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(12))
        .connect_timeout(Duration::from_secs(5))
        .redirect(Policy::none())
        .no_proxy()
        .user_agent("MSeriesMessages/1.0 (+https://github.com/techmore/ElementaryOS_MSeries)");
    match host {
        Host::Domain(domain) => {
            let addresses: Vec<SocketAddr> = (domain, 443)
                .to_socket_addrs()
                .map_err(|_| "Preview host could not be resolved".to_string())?
                .collect();
            if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(address.ip())) {
                return Err("Private-network preview addresses are blocked".into());
            }
            let addresses: Vec<SocketAddr> = addresses.into_iter().take(16).collect();
            builder = builder.resolve_to_addrs(domain, &addresses);
        }
        Host::Ipv4(address) => {
            if !is_public_ip(IpAddr::V4(address)) {
                return Err("Private-network preview addresses are blocked".into());
            }
        }
        Host::Ipv6(address) => {
            if !is_public_ip(IpAddr::V6(address)) {
                return Err("Private-network preview addresses are blocked".into());
            }
        }
    }
    let client = builder
        .build()
        .map_err(|_| "Could not prepare secure preview request".to_string())?;
    Ok((client, url))
}

fn is_safe_preview_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(|host| !host.is_empty())
        && url.as_str().len() <= 2048
}

fn is_public_ip(address: IpAddr) -> bool {
    static BLOCKED: OnceLock<Vec<IpNet>> = OnceLock::new();
    const BLOCKED_NETWORKS: &[&str] = &[
        "0.0.0.0/8",
        "10.0.0.0/8",
        "100.64.0.0/10",
        "127.0.0.0/8",
        "169.254.0.0/16",
        "172.16.0.0/12",
        "192.0.0.0/24",
        "192.0.2.0/24",
        "192.88.99.0/24",
        "192.168.0.0/16",
        "198.18.0.0/15",
        "198.51.100.0/24",
        "203.0.113.0/24",
        "224.0.0.0/4",
        "240.0.0.0/4",
        "2001::/23",
        "2001:db8::/32",
        "2002::/16",
        "3fff::/20",
    ];
    let blocked = BLOCKED.get_or_init(|| {
        BLOCKED_NETWORKS
            .iter()
            .map(|network| network.parse().expect("valid preview IP range"))
            .collect()
    });
    let globally_routable_v6 = "2000::/3".parse::<IpNet>().expect("valid IPv6 range");
    match address {
        IpAddr::V4(_) => !blocked.iter().any(|network| network.contains(&address)),
        IpAddr::V6(_) => {
            globally_routable_v6.contains(&address)
                && !blocked.iter().any(|network| network.contains(&address))
        }
    }
}

fn decode_image(
    data: &[u8],
    title: String,
    default_kind: PreviewKind,
) -> Result<LinkPreview, String> {
    let format = image::guess_format(data).map_err(|_| "Unsupported preview image".to_string())?;
    if !matches!(
        format,
        ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::Gif | ImageFormat::WebP
    ) {
        return Err("Unsupported preview image".into());
    }
    let dimensions = ImageReader::with_format(Cursor::new(data), format)
        .into_dimensions()
        .map_err(|_| "Preview image is invalid".to_string())?;
    validate_dimensions(dimensions)?;

    match format {
        ImageFormat::Gif => {
            let decoder = image::codecs::gif::GifDecoder::new(BufReader::new(Cursor::new(data)))
                .map_err(|_| "Preview animation is invalid".to_string())?;
            decode_animation(
                decoder.into_frames(),
                dimensions,
                title,
                default_kind,
                Orientation::NoTransforms,
            )
        }
        ImageFormat::WebP => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(data))
                .map_err(|_| "Preview image is invalid".to_string())?;
            let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
            decode_animation(
                decoder.into_frames(),
                dimensions,
                title,
                default_kind,
                orientation,
            )
        }
        _ => {
            let orientation = if format == ImageFormat::Jpeg {
                let mut decoder = image::codecs::jpeg::JpegDecoder::new(Cursor::new(data))
                    .map_err(|_| "Preview image is invalid".to_string())?;
                decoder.orientation().unwrap_or(Orientation::NoTransforms)
            } else {
                Orientation::NoTransforms
            };
            let decoded = image::load_from_memory_with_format(data, format)
                .map_err(|_| "Preview image is invalid".to_string())?;
            let mut decoded = decoded;
            decoded.apply_orientation(orientation);
            let thumbnail = thumbnail(decoded);
            let size = thumbnail.dimensions();
            let rgba = thumbnail.into_raw();
            if rgba.len() > MAX_DECODED_BYTES {
                return Err("Decoded preview is too large".into());
            }
            Ok(LinkPreview {
                title,
                kind: default_kind,
                source_size: dimensions,
                size,
                frames: vec![PreviewFrame {
                    rgba,
                    delay: Duration::from_millis(100),
                }],
            })
        }
    }
}

fn decode_animation(
    frames: impl Iterator<Item = image::ImageResult<image::Frame>>,
    source_size: (u32, u32),
    title: String,
    default_kind: PreviewKind,
    orientation: Orientation,
) -> Result<LinkPreview, String> {
    let mut decoded = Vec::new();
    let mut memory = 0usize;
    let mut source_pixels = 0u64;
    for frame in frames {
        if decoded.len() >= MAX_ANIMATED_FRAMES {
            return Err("Preview animation has too many frames".into());
        }
        let frame = frame.map_err(|_| "Preview animation is invalid".to_string())?;
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let buffer = frame.into_buffer();
        source_pixels =
            source_pixels.saturating_add(u64::from(buffer.width()) * u64::from(buffer.height()));
        if source_pixels > 96_000_000 {
            return Err("Preview animation is too large".into());
        }
        let delay = numerator
            .checked_div(denominator)
            .unwrap_or(100)
            .clamp(40, 2000);
        let mut decoded_image = DynamicImage::ImageRgba8(buffer);
        decoded_image.apply_orientation(orientation);
        let thumbnail = thumbnail(decoded_image);
        let rgba = thumbnail.into_raw();
        memory = memory.saturating_add(rgba.len());
        if memory > MAX_DECODED_BYTES {
            return Err("Decoded preview is too large".into());
        }
        decoded.push(PreviewFrame {
            rgba,
            delay: Duration::from_millis(delay as u64),
        });
    }
    if decoded.is_empty() {
        return Err("Preview animation has no frames".into());
    }
    let kind = if decoded.len() > 1 {
        PreviewKind::Animation
    } else {
        default_kind
    };
    let size = thumbnail_size(source_size);
    Ok(LinkPreview {
        title,
        kind,
        source_size,
        size,
        frames: decoded,
    })
}

fn thumbnail(image: DynamicImage) -> image::RgbaImage {
    image
        .thumbnail(THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT)
        .into_rgba8()
}

fn thumbnail_size((width, height): (u32, u32)) -> (u32, u32) {
    let scale = (THUMBNAIL_WIDTH as f64 / width as f64)
        .min(THUMBNAIL_HEIGHT as f64 / height as f64)
        .min(1.0);
    (
        ((width as f64 * scale).round() as u32).max(1),
        ((height as f64 * scale).round() as u32).max(1),
    )
}

fn validate_dimensions((width, height): (u32, u32)) -> Result<(), String> {
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS {
        Err("Preview image dimensions are too large".into())
    } else {
        Ok(())
    }
}

fn youtube_id(url: &Url) -> Option<String> {
    let host = url.host_str()?.to_ascii_lowercase();
    let candidate = if matches!(
        host.as_str(),
        "youtube.com" | "www.youtube.com" | "m.youtube.com"
    ) {
        if url.path() == "/watch" {
            url.query_pairs()
                .find(|(key, _)| key == "v")
                .map(|(_, value)| value.into_owned())
        } else if let Some(rest) = url.path().strip_prefix("/shorts/") {
            rest.split('/').next().map(str::to_string)
        } else if let Some(rest) = url.path().strip_prefix("/embed/") {
            rest.split('/').next().map(str::to_string)
        } else {
            None
        }
    } else if host == "youtu.be" {
        Some(url.path().trim_matches('/').to_string())
    } else {
        None
    }?;
    (candidate.len() == 11
        && candidate.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        }))
    .then_some(candidate)
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'
            | '\u{202b}'
            | '\u{202c}'
            | '\u{202d}'
            | '\u{202e}'
            | '\u{2066}'
            | '\u{2067}'
            | '\u{2068}'
            | '\u{2069}'
    )
}
