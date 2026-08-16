use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use bymsdfgen_core::{
    Bitmap, DistanceMapping, ErrorCorrectionConfig, ErrorCorrectionMode, FillRule,
    MsdfGeneratorConfig, Projection, Range, SdfTransformation, Shape, Vector2,
    coloring::edge_coloring_ink_trap, correction::msdf_error_correction, generate_mtsdf,
    generator::DistanceCheckMode, raster::distance_sign_correction_multi,
};
use bymsdfgen_io::{Font, FontCoordinateScaling};

use crate::font::FontSelection;

pub const ATLAS_SCALE: f64 = 64.0;
pub const FIELD_RANGE_PX: f64 = 12.0;
const GUARD_PX: u32 = 2;
const SHELF_GAP: u32 = 2;

#[derive(Debug, Clone, Copy)]
pub struct AtlasGlyph {
    pub pixel_min: [u32; 2],
    pub pixel_max: [u32; 2],
    /// Glyph quad bounds relative to its baseline origin, in em units with Y down.
    pub plane_min: [f32; 2],
    pub plane_max: [f32; 2],
}

#[derive(Debug, Clone)]
pub struct CpuAtlas {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub glyphs: BTreeMap<u16, AtlasGlyph>,
}

#[derive(Debug)]
struct RawGlyph {
    glyph_id: u16,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    plane_min: [f32; 2],
    plane_max: [f32; 2],
}

impl CpuAtlas {
    pub fn build(
        selection: &FontSelection,
        glyph_ids: &BTreeSet<u16>,
        max_dimension: u32,
    ) -> Result<Self> {
        let font = Font::from_slice(&selection.data, selection.face_index)
            .map_err(|error| anyhow::anyhow!(error))?;
        let mut raw_glyphs = Vec::with_capacity(glyph_ids.len());
        for &glyph_id in glyph_ids {
            match generate_glyph(&font, glyph_id)? {
                Some(glyph) => raw_glyphs.push(glyph),
                None if glyph_id == 0 => {
                    log::warn!("selected font has no outline for its .notdef glyph");
                }
                None => {
                    log::warn!(
                        "glyph {glyph_id} has no outline; .notdef will be used if available"
                    );
                }
            }
        }
        anyhow::ensure!(!raw_glyphs.is_empty(), "font produced no renderable glyphs");

        raw_glyphs.sort_by_key(|glyph| (std::cmp::Reverse(glyph.height), glyph.glyph_id));
        let max_dimension = max_dimension.max(256);
        let mut dimension = 256;
        let placements = loop {
            if let Some(placements) = pack_shelves(&raw_glyphs, dimension) {
                break placements;
            }
            if dimension >= max_dimension {
                bail!(
                    "MSDF glyph atlas does not fit the GPU's {max_dimension}×{max_dimension} texture limit"
                );
            }
            dimension = (dimension * 2).min(max_dimension);
        };

        let mut pixels = vec![0_u8; dimension as usize * dimension as usize * 4];
        let mut glyphs = BTreeMap::new();
        for (raw, [x, y]) in raw_glyphs.iter().zip(placements) {
            copy_flipped_rgba(&mut pixels, dimension, raw, x, y);
            glyphs.insert(
                raw.glyph_id,
                AtlasGlyph {
                    pixel_min: [x, y],
                    pixel_max: [x + raw.width, y + raw.height],
                    plane_min: raw.plane_min,
                    plane_max: raw.plane_max,
                },
            );
        }

        Ok(Self {
            width: dimension,
            height: dimension,
            pixels,
            glyphs,
        })
    }
}

fn generate_glyph(font: &Font<'_>, glyph_id: u16) -> Result<Option<RawGlyph>> {
    let mut shape = Shape::new();
    if !font.load_glyph(&mut shape, glyph_id, FontCoordinateScaling::EmNormalized) {
        return Ok(None);
    }
    shape.normalize();
    anyhow::ensure!(shape.validate(), "glyph {glyph_id} has an invalid outline");
    edge_coloring_ink_trap(&mut shape, 3.0, glyph_id as u64);

    let bounds = shape.get_bounds(0.0);
    anyhow::ensure!(
        bounds.l.is_finite()
            && bounds.b.is_finite()
            && bounds.r.is_finite()
            && bounds.t.is_finite(),
        "glyph {glyph_id} has non-finite bounds"
    );
    let margin = FIELD_RANGE_PX * 0.5 + GUARD_PX as f64;
    let width =
        ((bounds.r - bounds.l) * ATLAS_SCALE).ceil().max(1.0) as u32 + 2 * margin.ceil() as u32;
    let height =
        ((bounds.t - bounds.b) * ATLAS_SCALE).ceil().max(1.0) as u32 + 2 * margin.ceil() as u32;
    let translate = Vector2::new(
        margin / ATLAS_SCALE - bounds.l,
        margin / ATLAS_SCALE - bounds.b,
    );
    let range = Range::symmetric(FIELD_RANGE_PX / ATLAS_SCALE);
    let transformation = SdfTransformation::new(
        Projection::new(Vector2::splat(ATLAS_SCALE), translate),
        DistanceMapping::from_range(range),
    );
    let mut bitmap: Bitmap<f32, 4> = Bitmap::new(width as usize, height as usize);

    // Match bymsdfgen's CLI pipeline. The scanline pass repairs locally inverted
    // MTSDF signs against the font's nonzero fill, after which edge-priority
    // correction can safely repair interpolation artifacts.
    let generator_config = MsdfGeneratorConfig {
        error_correction: ErrorCorrectionConfig {
            mode: ErrorCorrectionMode::Disabled,
            ..Default::default()
        },
        ..Default::default()
    };
    generate_mtsdf(&mut bitmap, &shape, &transformation, &generator_config);
    distance_sign_correction_multi(
        &mut bitmap,
        &shape,
        &transformation.projection,
        0.5,
        FillRule::NonZero,
    );
    let postprocess_config = MsdfGeneratorConfig {
        error_correction: ErrorCorrectionConfig {
            mode: ErrorCorrectionMode::EdgePriority,
            distance_check_mode: DistanceCheckMode::DoNotCheckDistance,
            ..Default::default()
        },
        ..Default::default()
    };
    msdf_error_correction(&mut bitmap, &shape, &transformation, &postprocess_config);

    let pixels = bitmap
        .data()
        .chunks_exact(4)
        .flat_map(|sample| {
            let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
            [
                channel(sample[0]),
                channel(sample[1]),
                channel(sample[2]),
                channel(sample[3]),
            ]
        })
        .collect();
    let left = -translate.x;
    let right = width as f64 / ATLAS_SCALE - translate.x;
    let bottom_up = -translate.y;
    let top_up = height as f64 / ATLAS_SCALE - translate.y;

    Ok(Some(RawGlyph {
        glyph_id,
        width,
        height,
        pixels,
        plane_min: [left as f32, (-top_up) as f32],
        plane_max: [right as f32, (-bottom_up) as f32],
    }))
}

fn pack_shelves(glyphs: &[RawGlyph], dimension: u32) -> Option<Vec<[u32; 2]>> {
    let mut placements = Vec::with_capacity(glyphs.len());
    let mut x = SHELF_GAP;
    let mut y = SHELF_GAP;
    let mut row_height = 0;

    for glyph in glyphs {
        if glyph.width + 2 * SHELF_GAP > dimension || glyph.height + 2 * SHELF_GAP > dimension {
            return None;
        }
        if x + glyph.width + SHELF_GAP > dimension {
            x = SHELF_GAP;
            y += row_height + SHELF_GAP;
            row_height = 0;
        }
        if y + glyph.height + SHELF_GAP > dimension {
            return None;
        }
        placements.push([x, y]);
        x += glyph.width + SHELF_GAP;
        row_height = row_height.max(glyph.height);
    }

    Some(placements)
}

fn copy_flipped_rgba(destination: &mut [u8], atlas_width: u32, glyph: &RawGlyph, x: u32, y: u32) {
    let source_stride = glyph.width as usize * 4;
    let destination_stride = atlas_width as usize * 4;
    for destination_row in 0..glyph.height as usize {
        let source_row = glyph.height as usize - 1 - destination_row;
        let source_start = source_row * source_stride;
        let destination_start =
            (y as usize + destination_row) * destination_stride + x as usize * 4;
        destination[destination_start..destination_start + source_stride]
            .copy_from_slice(&glyph.pixels[source_start..source_start + source_stride]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    fn small_arial_atlas() -> CpuAtlas {
        let selection = FontSelection::resolve("Arial").expect("Arial should resolve");
        let font = Font::from_slice(&selection.data, selection.face_index).unwrap();
        let glyphs = ['A', 'g', '0']
            .into_iter()
            .map(|character| font.glyph_index(character).unwrap())
            .chain(std::iter::once(0))
            .collect();
        CpuAtlas::build(&selection, &glyphs, 1024).expect("atlas should build")
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn generates_a_padded_msdf_atlas() {
        let atlas = small_arial_atlas();
        assert!(atlas.width.is_power_of_two());
        assert_eq!(atlas.width, atlas.height);
        assert_eq!(
            atlas.pixels.len(),
            atlas.width as usize * atlas.height as usize * 4
        );
        assert!(atlas.pixels.chunks_exact(4).any(|pixel| pixel[0] > 128));
        assert!(atlas.pixels.chunks_exact(4).any(|pixel| pixel[0] < 127));
        assert!(atlas.glyphs.values().all(|glyph| {
            glyph.plane_min[0] < glyph.plane_max[0] && glyph.plane_min[1] < glyph.plane_max[1]
        }));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn packed_glyphs_do_not_overlap() {
        let atlas = small_arial_atlas();
        let glyphs: Vec<_> = atlas.glyphs.values().collect();
        for (index, a) in glyphs.iter().enumerate() {
            for b in &glyphs[index + 1..] {
                let separated = a.pixel_max[0] <= b.pixel_min[0]
                    || b.pixel_max[0] <= a.pixel_min[0]
                    || a.pixel_max[1] <= b.pixel_min[1]
                    || b.pixel_max[1] <= a.pixel_min[1];
                assert!(separated, "atlas glyph rectangles overlap");
            }
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn builds_the_complete_curated_document_atlas() {
        let selection = FontSelection::resolve("Arial").expect("Arial should resolve");
        let document = crate::text::Document::build(&selection).expect("document should shape");
        let atlas = CpuAtlas::build(&selection, &document.atlas_glyphs, 4096)
            .expect("complete atlas should fit");
        assert!(atlas.glyphs.len() >= 30);
        assert!(atlas.width <= 1024);
    }
}
