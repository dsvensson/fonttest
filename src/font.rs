use std::{path::Path, sync::Arc};

use anyhow::{Context, Result, bail};
use fontdb::{Database, Family, Query, Stretch, Style, Weight};

#[derive(Clone)]
pub struct FontSelection {
    pub data: Arc<[u8]>,
    pub face_index: u32,
    pub resolved_name: String,
    pub source: String,
}

impl FontSelection {
    pub fn resolve(spec: &str) -> Result<Self> {
        let path = Path::new(spec);
        if path.is_file() {
            let data = std::fs::read(path)
                .with_context(|| format!("reading font file {}", path.display()))?;
            validate_face(&data, 0)
                .with_context(|| format!("parsing font file {}", path.display()))?;
            let resolved_name = path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("font")
                .to_owned();
            return Ok(Self {
                data: data.into(),
                face_index: 0,
                resolved_name,
                source: path.display().to_string(),
            });
        }

        let mut database = Database::new();
        database.load_system_fonts();
        let families = [Family::Name(spec)];
        let query = Query {
            families: &families,
            weight: Weight::NORMAL,
            stretch: Stretch::Normal,
            style: Style::Normal,
        };
        let id = database.query(&query).with_context(|| {
            format!("installed font family '{spec}' was not found; pass a family name or font path")
        })?;
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

        Ok(Self {
            data,
            face_index,
            resolved_name,
            source,
        })
    }
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
    fn resolves_default_arial_family() {
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
}
