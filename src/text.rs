use std::collections::BTreeSet;

use anyhow::{Context, Result};
use glam::DVec2;
use harfrust::{ShapeOptions, Shaper, ShaperData, UnicodeBuffer};
use unicode_linebreak::{BreakOpportunity, linebreaks};

use crate::font::FontSelection;

pub const PAGE_WIDTH: f64 = 1080.0;
const PAGE_PADDING: f64 = 80.0;

// Lorem Ipsum is corrupted Latin, so this is a romanized Klingon adaptation of
// the underlying passage's ideas about pleasure, suffering, effort, and choice.
const KLINGON_LEAD: &str =
    "bech neHbogh pagh tu'lu', 'ach rut bel'a' SuqmeH vay', vumqu'nIS 'ej bechnIS.";
const KLINGON_PARAGRAPHS: [&str; 4] = [
    "bel parbogh pagh tu'lu'; belmo' parlu'be'. 'ach valbe'taHvIS bel tlha'chugh vay', QaghmeyDajmo' bechqu' ghaH. meqmey yajchugh, bel SuqlaH 'ej Sengmey junlaH.",
    "bech neHbogh pagh tu'lu'; bechmo' neHlu'be'. 'ach rut qaS wanI' Qatlh. bel'a' SuqmeH vay', vumqu'nIS ghaH, Sengmey SIQnIS, 'ej QatlhwI' jeynIS. tagha' QapDI', belna' chav.",
    "qatlh porghDaj qeqtaH vay'? HoSDaj ghurmoHmeH 'ej laHDaj DubmeH qeq. vay' chavlaHbe'chugh, Qu' Qatlh taghbe'. bechta'mo' HoSghajchoH, 'ej qaDmeyDaj jeyDI' valchoH.",
    "bel tIvlu'chugh 'ej Sengmey chenmoHbe'chugh belvam, wIvbogh vay' naDHa'laH pagh. bechlu'chugh 'ej bel Suqbe'lu'chugh, bechbe'meH vangbogh vay' naDHa'laH pagh. bel neHbe' SuvwI' val, bech neHbe' je; Qapla' neH.",
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub min: DVec2,
    pub max: DVec2,
}

impl Rect {
    pub fn size(self) -> DVec2 {
        self.max - self.min
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TextStyle {
    pub fill_top: [f32; 4],
    pub fill_bottom: [f32; 4],
    pub outline_color: [f32; 4],
    pub shadow_color: [f32; 4],
    pub outline_em: f32,
    pub glow_em: f32,
    pub shadow_offset_em: [f32; 2],
}

#[derive(Debug, Clone)]
pub struct DocumentGlyph {
    pub glyph_id: u16,
    pub origin: DVec2,
    pub font_size: f64,
    pub style: u32,
    pub whitespace: bool,
}

#[derive(Debug, Clone)]
pub struct Document {
    pub glyphs: Vec<DocumentGlyph>,
    pub hud_glyphs: Vec<DocumentGlyph>,
    pub styles: Vec<TextStyle>,
    pub bounds: Rect,
    pub atlas_glyphs: BTreeSet<u16>,
}

#[derive(Debug, Clone)]
struct RelativeGlyph {
    glyph_id: u16,
    offset: DVec2,
    whitespace: bool,
}

#[derive(Debug, Clone)]
struct ShapedLine {
    glyphs: Vec<RelativeGlyph>,
    width: f64,
}

impl Document {
    pub fn build(selection: &FontSelection) -> Result<Self> {
        let file =
            read_fonts::FileRef::new(&selection.data).map_err(|error| anyhow::anyhow!(error))?;
        let font_ref = file
            .fonts()
            .nth(selection.face_index as usize)
            .context("selected font face is out of range")?
            .map_err(|error| anyhow::anyhow!(error))?;
        let shaper_data = ShaperData::new(&font_ref);
        let shaper = shaper_data.shaper(&font_ref).build();

        let outline_font = bymsdfgen_io::Font::from_slice(&selection.data, selection.face_index)
            .map_err(|error| anyhow::anyhow!(error))?;
        let raw_metrics = outline_font.metrics(bymsdfgen_io::FontCoordinateScaling::None);
        let metrics = outline_font.metrics(bymsdfgen_io::FontCoordinateScaling::EmNormalized);
        let units_per_em = raw_metrics.em_size;
        anyhow::ensure!(units_per_em > 0.0, "font reports a zero units-per-em value");

        let styles = curated_styles();
        let mut glyphs = Vec::new();
        let mut y = 66.0;

        add_line(
            &mut glyphs,
            &shaper,
            units_per_em,
            "DISTANCE FIELD TYPE STUDY",
            PAGE_PADDING,
            y + metrics.ascender * 15.0,
            15.0,
            3,
        );
        y += 45.0;
        add_line(
            &mut glyphs,
            &shaper,
            units_per_em,
            "Lorem Ipsum",
            PAGE_PADDING,
            y + metrics.ascender * 92.0,
            92.0,
            0,
        );
        y += 120.0;
        add_line(
            &mut glyphs,
            &shaper,
            units_per_em,
            "MSDF typography · shaped with HarfRust",
            PAGE_PADDING + 4.0,
            y + metrics.ascender * 28.0,
            28.0,
            1,
        );
        y += 78.0;

        y = add_wrapped_block(
            &mut glyphs,
            &shaper,
            units_per_em,
            KLINGON_LEAD,
            PAGE_PADDING,
            y,
            880.0,
            39.0,
            1.28,
            metrics.ascender,
            1,
        );
        y += 46.0;

        for paragraph in KLINGON_PARAGRAPHS {
            y = add_wrapped_block(
                &mut glyphs,
                &shaper,
                units_per_em,
                paragraph,
                PAGE_PADDING,
                y,
                PAGE_WIDTH - 2.0 * PAGE_PADDING,
                25.0,
                1.52,
                metrics.ascender,
                2,
            );
            y += 28.0;
        }

        let mut hud_glyphs = Vec::new();
        add_line(
            &mut hud_glyphs,
            &shaper,
            units_per_em,
            "Wheel: zoom at cursor  |  Drag: pan  |  0: reset",
            22.0,
            22.0 + metrics.ascender * 15.0,
            15.0,
            3,
        );

        let mut atlas_glyphs: BTreeSet<_> = glyphs
            .iter()
            .chain(&hud_glyphs)
            .filter(|glyph| !glyph.whitespace)
            .map(|glyph| glyph.glyph_id)
            .collect();
        atlas_glyphs.insert(0);

        Ok(Self {
            glyphs,
            hud_glyphs,
            styles,
            bounds: Rect {
                min: DVec2::ZERO,
                max: DVec2::new(PAGE_WIDTH, y + PAGE_PADDING),
            },
            atlas_glyphs,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn add_wrapped_block(
    output: &mut Vec<DocumentGlyph>,
    shaper: &Shaper<'_>,
    units_per_em: f64,
    text: &str,
    x: f64,
    top: f64,
    max_width: f64,
    font_size: f64,
    line_height_factor: f64,
    ascender_em: f64,
    style: u32,
) -> f64 {
    let lines = wrap_lines(shaper, units_per_em, text, font_size, max_width);
    let line_height = font_size * line_height_factor;
    for (index, line) in lines.iter().enumerate() {
        let baseline = top + ascender_em * font_size + index as f64 * line_height;
        append_shaped(output, line, x, baseline, font_size, style);
    }
    top + lines.len() as f64 * line_height
}

#[allow(clippy::too_many_arguments)]
fn add_line(
    output: &mut Vec<DocumentGlyph>,
    shaper: &Shaper<'_>,
    units_per_em: f64,
    text: &str,
    x: f64,
    baseline: f64,
    font_size: f64,
    style: u32,
) {
    let line = shape_line(shaper, units_per_em, text, font_size);
    append_shaped(output, &line, x, baseline, font_size, style);
}

fn append_shaped(
    output: &mut Vec<DocumentGlyph>,
    line: &ShapedLine,
    x: f64,
    baseline: f64,
    font_size: f64,
    style: u32,
) {
    output.extend(line.glyphs.iter().map(|glyph| DocumentGlyph {
        glyph_id: glyph.glyph_id,
        origin: DVec2::new(x + glyph.offset.x, baseline + glyph.offset.y),
        font_size,
        style,
        whitespace: glyph.whitespace,
    }));
}

fn wrap_lines(
    shaper: &Shaper<'_>,
    units_per_em: f64,
    text: &str,
    font_size: f64,
    max_width: f64,
) -> Vec<ShapedLine> {
    let breaks: Vec<_> = linebreaks(text).collect();
    let mut lines = Vec::new();
    let mut start = 0;

    while start < text.len() {
        start = skip_whitespace(text, start);
        if start >= text.len() {
            break;
        }

        let mut best: Option<(usize, ShapedLine)> = None;
        for &(end, opportunity) in breaks.iter().filter(|(end, _)| *end > start) {
            let candidate = text[start..end].trim_end();
            if candidate.is_empty() {
                continue;
            }
            let shaped = shape_line(shaper, units_per_em, candidate, font_size);
            if shaped.width <= max_width || best.is_none() {
                let forced = matches!(opportunity, BreakOpportunity::Mandatory);
                best = Some((end, shaped));
                if forced {
                    break;
                }
            } else {
                break;
            }
        }

        let (end, shaped) = best.unwrap_or_else(|| {
            let candidate = text[start..].trim_end();
            (
                text.len(),
                shape_line(shaper, units_per_em, candidate, font_size),
            )
        });
        lines.push(shaped);
        start = end;
    }

    lines
}

fn skip_whitespace(text: &str, mut index: usize) -> usize {
    while let Some(character) = text[index..].chars().next() {
        if !character.is_whitespace() {
            break;
        }
        index += character.len_utf8();
        if index == text.len() {
            break;
        }
    }
    index
}

fn shape_line(shaper: &Shaper<'_>, units_per_em: f64, text: &str, font_size: f64) -> ShapedLine {
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    let shaped = shaper.shape(buffer, ShapeOptions::new());
    let scale = font_size / units_per_em;
    let mut pen = DVec2::ZERO;
    let mut glyphs = Vec::with_capacity(shaped.glyph_infos().len());

    for (info, position) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
        let cluster = info.cluster as usize;
        let whitespace = text
            .get(cluster..)
            .and_then(|tail| tail.chars().next())
            .is_some_and(char::is_whitespace);
        glyphs.push(RelativeGlyph {
            glyph_id: info.glyph_id as u16,
            offset: pen
                + DVec2::new(
                    position.x_offset as f64 * scale,
                    -(position.y_offset as f64) * scale,
                ),
            whitespace,
        });
        pen += DVec2::new(
            position.x_advance as f64 * scale,
            -(position.y_advance as f64) * scale,
        );
    }

    ShapedLine {
        glyphs,
        width: pen.x.abs(),
    }
}

fn curated_styles() -> Vec<TextStyle> {
    vec![
        TextStyle {
            fill_top: [0.19, 0.88, 1.0, 1.0],
            fill_bottom: [0.55, 0.20, 1.0, 1.0],
            outline_color: [0.055, 0.035, 0.19, 0.96],
            shadow_color: [0.04, 0.24, 0.95, 0.55],
            outline_em: 0.024,
            glow_em: 0.07,
            shadow_offset_em: [0.035, 0.055],
        },
        TextStyle {
            fill_top: [1.0, 0.73, 0.24, 1.0],
            fill_bottom: [0.93, 0.34, 0.11, 1.0],
            outline_color: [0.13, 0.055, 0.025, 0.92],
            shadow_color: [0.82, 0.18, 0.045, 0.3],
            outline_em: 0.017,
            glow_em: 0.045,
            shadow_offset_em: [0.025, 0.04],
        },
        TextStyle {
            fill_top: [0.95, 0.96, 1.0, 1.0],
            fill_bottom: [0.72, 0.78, 0.9, 1.0],
            outline_color: [0.04, 0.055, 0.11, 0.42],
            shadow_color: [0.0, 0.0, 0.02, 0.55],
            outline_em: 0.006,
            glow_em: 0.02,
            shadow_offset_em: [0.025, 0.045],
        },
        TextStyle {
            fill_top: [0.47, 0.56, 0.75, 0.92],
            fill_bottom: [0.3, 0.38, 0.6, 0.92],
            outline_color: [0.015, 0.02, 0.05, 0.5],
            shadow_color: [0.0, 0.0, 0.0, 0.35],
            outline_em: 0.008,
            glow_em: 0.018,
            shadow_offset_em: [0.02, 0.035],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_copy_is_romanized_klingon() {
        let copy = std::iter::once(KLINGON_LEAD)
            .chain(KLINGON_PARAGRAPHS)
            .collect::<Vec<_>>()
            .join(" ");

        assert!(!copy.contains("Lorem ipsum"));
        assert!(copy.contains("Qapla'"));
        assert!(
            KLINGON_PARAGRAPHS
                .iter()
                .all(|paragraph| paragraph.contains('\''))
        );
    }

    #[cfg(target_os = "windows")]
    fn arial_document() -> Document {
        let font = FontSelection::resolve("Arial").expect("Arial should resolve");
        Document::build(&font).expect("document should shape")
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn shapes_curated_document_and_hud() {
        let document = arial_document();
        assert!(document.glyphs.len() > 300);
        assert!(document.hud_glyphs.len() > 20);
        assert!(document.atlas_glyphs.len() > 20);
        assert!(document.glyphs.iter().all(|glyph| glyph.origin.is_finite()));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn wraps_page_to_the_declared_width() {
        let document = arial_document();
        assert_eq!(document.bounds.min, DVec2::ZERO);
        assert_eq!(document.bounds.max.x, PAGE_WIDTH);
        assert!(document.bounds.max.y > 800.0);
        assert!(document.bounds.size().is_finite());
    }
}
