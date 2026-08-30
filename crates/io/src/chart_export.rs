//! SVG/PNG export of a chart scene.

use omacell_core::chart::{Chart, ChartTheme, Scene, layout_chart, to_svg};
use omacell_core::error::CoreError;
use omacell_core::workbook::Workbook;

use crate::error;

/// Render `chart` to SVG.
pub fn chart_svg(
    wb: &Workbook,
    chart: &Chart,
    theme: &ChartTheme,
    width: f32,
    height: f32,
) -> Result<String, CoreError> {
    validate_svg_size(width, height)?;
    Ok(to_svg(&layout_chart(wb, chart, theme, width, height)?))
}

/// Render `chart` to PNG bytes.
pub fn chart_png(
    wb: &Workbook,
    chart: &Chart,
    theme: &ChartTheme,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, CoreError> {
    validate_raster_size(width, height)?;
    let svg = chart_svg(wb, chart, theme, width as f32, height as f32)?;
    rasterize_svg(&svg, width, height)
}

/// Rasterize an SVG document.
pub fn rasterize_svg(svg: &str, width: u32, height: u32) -> Result<Vec<u8>, CoreError> {
    validate_raster_size(width, height)?;
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_str(svg, &opt)
        .map_err(|err| error::chart_export(format!("svg: {err}")))?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| error::chart_export("cannot allocate PNG pixmap"))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(
            width as f32 / tree.size().width().max(1.0),
            height as f32 / tree.size().height().max(1.0),
        ),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|err| error::chart_export(format!("png: {err}")))
}

/// Shared scene used by GUI and export (parity).
pub fn chart_scene(
    wb: &Workbook,
    chart: &Chart,
    theme: &ChartTheme,
    width: f32,
    height: f32,
) -> Result<Scene, CoreError> {
    validate_svg_size(width, height)?;
    layout_chart(wb, chart, theme, width, height)
}

fn validate_svg_size(width: f32, height: f32) -> Result<(), CoreError> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(error::chart_export(
            "chart dimensions must be finite and greater than zero",
        ));
    }
    if width > 100_000.0 || height > 100_000.0 {
        return Err(error::chart_export(
            "chart vector dimension exceeds 100,000 units",
        ));
    }
    Ok(())
}

fn validate_raster_size(width: u32, height: u32) -> Result<(), CoreError> {
    const MAX_RASTER_PIXELS: u64 = 64 * 1024 * 1024;
    if width == 0 || height == 0 {
        return Err(error::chart_export(
            "PNG width and height must be greater than zero",
        ));
    }
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if pixels > MAX_RASTER_PIXELS {
        return Err(error::chart_export(format!(
            "PNG has {pixels} pixels; maximum is {MAX_RASTER_PIXELS}"
        )));
    }
    Ok(())
}
