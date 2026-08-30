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
    let svg = chart_svg(wb, chart, theme, width as f32, height as f32)?;
    rasterize_svg(&svg, width, height)
}

/// Rasterize an SVG document.
pub fn rasterize_svg(svg: &str, width: u32, height: u32) -> Result<Vec<u8>, CoreError> {
    let opt = usvg::Options::default();
    let tree =
        usvg::Tree::from_str(svg, &opt).map_err(|err| error::xlsx_write(format!("svg: {err}")))?;
    let mut pixmap = tiny_skia::Pixmap::new(width.max(1), height.max(1))
        .ok_or_else(|| error::xlsx_write("png pixmap"))?;
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
        .map_err(|err| error::xlsx_write(format!("png: {err}")))
}

/// Shared scene used by GUI and export (parity).
pub fn chart_scene(
    wb: &Workbook,
    chart: &Chart,
    theme: &ChartTheme,
    width: f32,
    height: f32,
) -> Result<Scene, CoreError> {
    layout_chart(wb, chart, theme, width, height)
}
