use std::{path::Path, sync::Arc};

#[cfg(not(target_arch = "wasm32"))]
use std::{fs, path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail, ensure};
#[cfg(not(target_arch = "wasm32"))]
use directories::ProjectDirs;
#[cfg(not(target_arch = "wasm32"))]
use fontdb::{Database, Family, Query, Stretch, Style, Weight};

const GOOGLE_FONTS_CSS_BASE: &str = "https://fonts.googleapis.com/css?family=";
const GOOGLE_FONTS_CDN: &str = "https://fonts.gstatic.com/";
#[cfg(not(target_arch = "wasm32"))]
const GOOGLE_FONTS_USER_AGENT: &str = "Mozilla/5.0";
const CSS_SIZE_LIMIT: usize = 64 * 1024;
const FONT_SIZE_LIMIT: usize = 20 * 1024 * 1024;
#[cfg(not(target_arch = "wasm32"))]
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub struct FontSelection {
    pub data: Arc<[u8]>,
    pub face_index: u32,
    pub resolved_name: String,
    pub source: String,
}

impl FontSelection {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn resolve(spec: &str) -> Result<Self> {
        let spec = spec.trim();
        ensure!(!spec.is_empty(), "font family or path cannot be empty");
        let path = Path::new(spec);
        if path.is_file() {
            return Self::from_path(path);
        }
        if looks_like_font_path(spec) {
            bail!("font file {} does not exist", path.display());
        }

        if let Some(font) = Self::from_system_family(spec)? {
            return Ok(font);
        }

        Self::from_google_fonts(spec).with_context(|| {
            format!(
                "installed font family '{spec}' was not found and Google Fonts could not provide it"
            )
        })
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn resolve(spec: &str) -> Result<Self> {
        let spec = spec.trim();
        ensure!(!spec.is_empty(), "font family cannot be empty");
        ensure!(
            !looks_like_font_path(spec),
            "font file paths are unavailable in a browser; use a Google Fonts family name"
        );
        let css_url = google_css_url(spec)?;
        log::info!("fetching '{spec}' from Google Fonts");
        let data = download_google_font(&css_url)
            .await
            .with_context(|| format!("downloading Google Fonts family '{spec}'"))?;
        log::info!("downloaded '{}' ({} decoded bytes)", spec, data.len());
        validate_face(&data, 0)
            .with_context(|| format!("validating downloaded Google font '{spec}'"))?;
        Ok(Self {
            data: data.into(),
            face_index: 0,
            resolved_name: spec.to_owned(),
            source: format!("Google Fonts browser fetch ({css_url})"),
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn from_path(path: &Path) -> Result<Self> {
        let data =
            fs::read(path).with_context(|| format!("reading font file {}", path.display()))?;
        validate_face(&data, 0).with_context(|| format!("parsing font file {}", path.display()))?;
        let resolved_name = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("font")
            .to_owned();
        Ok(Self {
            data: data.into(),
            face_index: 0,
            resolved_name,
            source: path.display().to_string(),
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn from_system_family(spec: &str) -> Result<Option<Self>> {
        let mut database = Database::new();
        database.load_system_fonts();
        let families = [Family::Name(spec)];
        let query = Query {
            families: &families,
            weight: Weight::NORMAL,
            stretch: Stretch::Normal,
            style: Style::Normal,
        };
        let Some(id) = database.query(&query) else {
            return Ok(None);
        };
        let face = database
            .face(id)
            .context("font database returned an unknown face")?;
        let face_index = face.index;
        let resolved_name = face
            .families
            .first()
            .map(|family| family.0.clone())
            .unwrap_or_else(|| spec.to_owned());
        let source = format!("system family {resolved_name}");
        let data = database
            .with_face_data(id, |bytes, _| Arc::<[u8]>::from(bytes))
            .context("loading the selected system font data")?;
        validate_face(&data, face_index)
            .with_context(|| format!("parsing selected face {face_index} for {resolved_name}"))?;

        Ok(Some(Self {
            data,
            face_index,
            resolved_name,
            source,
        }))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn from_google_fonts(spec: &str) -> Result<Self> {
        let css_url = google_css_url(spec)?;
        let cache_path = google_cache_path(spec);
        if let Some(cache_path) = cache_path.as_ref() {
            match fs::read(cache_path) {
                Ok(data) => match validate_face(&data, 0) {
                    Ok(()) => {
                        return Ok(Self {
                            data: data.into(),
                            face_index: 0,
                            resolved_name: spec.to_owned(),
                            source: format!("Google Fonts cache {}", cache_path.display()),
                        });
                    }
                    Err(error) => log::warn!(
                        "ignoring invalid cached Google font {}: {error:#}",
                        cache_path.display()
                    ),
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => log::warn!(
                    "could not read Google Fonts cache {}: {error}",
                    cache_path.display()
                ),
            }
        } else {
            log::warn!("the operating system did not provide an application cache directory");
        }

        log::info!("fetching '{spec}' from Google Fonts");
        let data = download_google_font(&css_url)
            .with_context(|| format!("downloading Google Fonts family '{spec}'"))?;
        log::info!("downloaded '{}' ({} decoded bytes)", spec, data.len());
        validate_face(&data, 0)
            .with_context(|| format!("validating downloaded Google font '{spec}'"))?;
        if let Some(cache_path) = cache_path.as_ref()
            && let Err(error) = write_google_cache(cache_path, &data)
        {
            log::warn!(
                "could not cache Google font at {}: {error:#}",
                cache_path.display()
            );
        }

        Ok(Self {
            data: data.into(),
            face_index: 0,
            resolved_name: spec.to_owned(),
            source: format!("Google Fonts download ({css_url})"),
        })
    }
}

fn looks_like_font_path(spec: &str) -> bool {
    let path = Path::new(spec);
    path.is_absolute()
        || spec.contains('/')
        || spec.contains('\\')
        || path.extension().is_some_and(|extension| {
            matches!(
                extension.to_str().map(str::to_ascii_lowercase).as_deref(),
                Some("ttf" | "otf" | "ttc")
            )
        })
}

fn google_css_url(family: &str) -> Result<String> {
    ensure!(
        family
            .chars()
            .all(|character| character.is_ascii_alphanumeric()
                || character == ' '
                || character == '-'),
        "Google Fonts family names may contain only ASCII letters, digits, spaces, and hyphens"
    );
    let encoded = family.split_whitespace().collect::<Vec<_>>().join("+");
    ensure!(
        !encoded.is_empty(),
        "Google Fonts family name cannot be empty"
    );
    Ok(format!("{GOOGLE_FONTS_CSS_BASE}{encoded}"))
}

#[cfg(not(target_arch = "wasm32"))]
fn download_google_font(css_url: &str) -> Result<Vec<u8>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(DOWNLOAD_TIMEOUT))
        .build()
        .into();
    let css = agent
        .get(css_url)
        .header("User-Agent", GOOGLE_FONTS_USER_AGENT)
        .call()
        .with_context(|| format!("requesting stylesheet {css_url}"))?
        .body_mut()
        .with_config()
        .limit(CSS_SIZE_LIMIT as u64)
        .read_to_string()
        .context("reading Google Fonts stylesheet")?;
    let font_url = extract_font_url(&css)?;
    let data = agent
        .get(&font_url)
        .header("User-Agent", GOOGLE_FONTS_USER_AGENT)
        .call()
        .with_context(|| format!("requesting font file {font_url}"))?
        .body_mut()
        .with_config()
        .limit(FONT_SIZE_LIMIT as u64)
        .read_to_vec()
        .context("reading Google font file")?;
    decode_font_payload(data)
}

#[cfg(target_arch = "wasm32")]
async fn download_google_font(css_url: &str) -> Result<Vec<u8>> {
    use gloo_net::http::Request;

    let css_response = Request::get(css_url)
        .send()
        .await
        .map_err(|error| anyhow::anyhow!("requesting stylesheet {css_url}: {error}"))?;
    ensure!(
        css_response.ok(),
        "Google Fonts stylesheet request returned HTTP {}",
        css_response.status()
    );
    let css = css_response
        .text()
        .await
        .map_err(|error| anyhow::anyhow!("reading Google Fonts stylesheet: {error}"))?;
    ensure!(
        css.len() <= CSS_SIZE_LIMIT,
        "Google Fonts stylesheet exceeded the size limit"
    );
    let font_url = extract_font_url(&css)?;
    let font_response = Request::get(&font_url)
        .send()
        .await
        .map_err(|error| anyhow::anyhow!("requesting font file {font_url}: {error}"))?;
    ensure!(
        font_response.ok(),
        "Google font request returned HTTP {}",
        font_response.status()
    );
    if let Some(content_length) = font_response.headers().get("content-length")
        && let Ok(content_length) = content_length.parse::<usize>()
    {
        ensure!(
            content_length <= FONT_SIZE_LIMIT,
            "Google font file exceeded the size limit"
        );
    }
    let data = font_response
        .binary()
        .await
        .map_err(|error| anyhow::anyhow!("reading Google font file: {error}"))?;
    ensure!(
        data.len() <= FONT_SIZE_LIMIT,
        "Google font file exceeded the size limit"
    );
    decode_font_payload(data)
}

fn decode_font_payload(data: Vec<u8>) -> Result<Vec<u8>> {
    ensure!(!data.is_empty(), "Google Fonts returned an empty font file");
    if data.starts_with(b"wOF2") {
        return wuff::decompress_woff2(&data)
            .map_err(|error| anyhow::anyhow!("decompressing Google WOFF2 font: {error:?}"));
    }
    Ok(data)
}

fn extract_font_url(css: &str) -> Result<String> {
    let mut remaining = css;
    let mut selected = None;
    while let Some(start) = remaining.find("url(") {
        remaining = &remaining[start + 4..];
        let Some(end) = remaining.find(')') else {
            break;
        };
        let candidate = remaining[..end]
            .trim()
            .trim_matches(|character| character == '\'' || character == '"');
        if candidate.starts_with(GOOGLE_FONTS_CDN)
            && (candidate.contains(".ttf") || candidate.contains(".woff2"))
        {
            // Google orders unicode subsets from specialized to general. Keeping the
            // last face selects the Latin subset used by this sample in browsers.
            selected = Some(candidate.to_owned());
        }
        remaining = &remaining[end + 1..];
    }
    selected.context("Google Fonts stylesheet did not contain a trusted font download URL")
}

#[cfg(not(target_arch = "wasm32"))]
fn google_cache_path(family: &str) -> Option<PathBuf> {
    let project_dirs = ProjectDirs::from("dev", "fonttest", "MSDF Font Explorer")?;
    Some(
        project_dirs
            .cache_dir()
            .join("google-fonts")
            .join(google_cache_file_name(family)),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn google_cache_file_name(family: &str) -> String {
    let normalized = family
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let mut slug = String::new();
    for character in normalized.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    let hash = normalized
        .bytes()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    format!("{slug}-{hash:016x}.ttf")
}

#[cfg(not(target_arch = "wasm32"))]
fn write_google_cache(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .context("Google Fonts cache path has no parent")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("creating cache directory {}", parent.display()))?;
    fs::write(path, data).with_context(|| format!("writing cache file {}", path.display()))
}

fn validate_face(data: &[u8], face_index: u32) -> Result<()> {
    bymsdfgen_io::Font::from_slice(data, face_index)
        .map(|_| ())
        .map_err(|error| anyhow::anyhow!(error))?;

    let file = read_fonts::FileRef::new(data).map_err(|error| anyhow::anyhow!(error))?;
    let face = file.fonts().nth(face_index as usize);
    match face {
        Some(Ok(_)) => Ok(()),
        Some(Err(error)) => Err(anyhow::anyhow!(error)),
        None => bail!("font face index {face_index} is out of range"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    #[test]
    fn resolves_installed_arial_family() {
        let font = FontSelection::resolve("Arial").expect("Arial should be installed on Windows");
        assert!(!font.data.is_empty());
        assert_eq!(font.resolved_name, "Arial");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn resolves_font_path() {
        let font = FontSelection::resolve(r"C:\Windows\Fonts\arial.ttf")
            .expect("the standard Arial path should load");
        assert!(!font.data.is_empty());
        assert_eq!(font.face_index, 0);
    }

    #[test]
    fn creates_google_fonts_css_url() {
        assert_eq!(
            google_css_url("Playfair   Display").expect("family should encode"),
            "https://fonts.googleapis.com/css?family=Playfair+Display"
        );
        assert!(google_css_url("Roboto&family=Other").is_err());
    }

    #[test]
    fn extracts_only_trusted_google_font_urls() {
        let css = r#"
            @font-face {
                src: url('https://fonts.gstatic.com/s/playfair/v1/font.ttf') format('truetype');
            }
        "#;
        assert_eq!(
            extract_font_url(css).expect("trusted TTF URL should parse"),
            "https://fonts.gstatic.com/s/playfair/v1/font.ttf"
        );
        assert!(extract_font_url("src: url(https://example.com/font.ttf)").is_err());
        assert_eq!(
            extract_font_url("src: url(https://fonts.gstatic.com/font.woff2)")
                .expect("trusted WOFF2 URL should parse"),
            "https://fonts.gstatic.com/font.woff2"
        );
    }

    #[test]
    fn chooses_the_last_google_font_subset() {
        let css = r#"
            src: url(https://fonts.gstatic.com/s/font/cyrillic.woff2) format('woff2');
            src: url(https://fonts.gstatic.com/s/font/latin.woff2) format('woff2');
        "#;
        assert_eq!(
            extract_font_url(css).expect("Latin subset should parse"),
            "https://fonts.gstatic.com/s/font/latin.woff2"
        );
    }

    #[test]
    fn google_cache_keys_are_normalized_and_distinct() {
        assert_eq!(
            google_cache_file_name("Playfair Display"),
            google_cache_file_name("  playfair   display ")
        );
        assert_ne!(
            google_cache_file_name("Playfair Display"),
            google_cache_file_name("Playfair")
        );
    }
}
