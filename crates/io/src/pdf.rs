//! PDF export of paginated sheets (spec F-11.2). Same grid + chart scene as the GUI.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use omacell_core::addr::{SheetId, col_to_letters};
use omacell_core::chart::{ChartTheme, Op, layout_chart};
use omacell_core::error::CoreError;
use omacell_core::geometry::{DEFAULT_COL_PX, DEFAULT_ROW_PX};
use omacell_core::locale::LocaleId;
use omacell_core::numfmt::{FormatValue, format};
use omacell_core::print::{PX_TO_PT, PageBox, PageSetup, expand_header, paginate, print_bounds};
use omacell_core::sheet::{Hyperlink, Sheet};
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use pdf_writer::types::{
    ActionType, AnnotationType, CidFontType, FontFlags, SystemInfo, UnicodeCmap,
};
use pdf_writer::{Content, Name, Pdf, Rect, Ref, Str};

use crate::error;

const FONT_NAME: Name<'_> = Name(b"F1");
const MAX_EMBEDDED_FONT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PDF_STREAM_BYTES: usize = 128 * 1024 * 1024;
const MAX_PDF_PAGES: usize = 10_000;
const SYSTEM_INFO: SystemInfo<'_> = SystemInfo {
    registry: Str(b"Adobe"),
    ordering: Str(b"Identity"),
    supplement: 0,
};

/// Options for [`write_pdf`].
#[derive(Clone, Debug)]
pub struct PdfOptions {
    /// Optional TrueType face to embed (`/FontFile2`). Helvetica otherwise.
    pub font_path: Option<PathBuf>,
    /// File name substituted into header/footer `&F`.
    pub file_name: String,
    /// Chart colours (print equals screen when the GUI theme is supplied).
    pub theme: ChartTheme,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            font_path: None,
            file_name: "workbook.pdf".into(),
            theme: ChartTheme {
                background: "#ffffff".into(),
                foreground: "#111111".into(),
                axis: "#444444".into(),
                gridline: "#cccccc".into(),
                palette: [
                    "#1e66f5".into(),
                    "#40a02b".into(),
                    "#df8e1d".into(),
                    "#d20f39".into(),
                    "#8839ef".into(),
                    "#04a5e5".into(),
                    "#fe640b".into(),
                    "#4c4f69".into(),
                ],
            },
        }
    }
}

/// Encode `wb` as PDF bytes.
pub fn write_pdf(wb: &Workbook, options: &PdfOptions) -> Result<Vec<u8>, CoreError> {
    let font_data = match &options.font_path {
        Some(path) => Some(read_font(path)?),
        None => None,
    };
    let face = font_data
        .as_deref()
        .map(|bytes| ttf_parser::Face::parse(bytes, 0))
        .transpose()
        .map_err(|err| error::pdf_write(format!("ttf: {err}")))?;

    let mut alloc = Alloc::new();
    let catalog_id = alloc.next();
    let page_tree_id = alloc.next();
    let font_id = alloc.next();
    let mut extra_font_ids = ExtraFontIds::default();
    if face.is_some() {
        extra_font_ids.cid = Some(alloc.next());
        extra_font_ids.desc = Some(alloc.next());
        extra_font_ids.cmap = Some(alloc.next());
        extra_font_ids.stream = Some(alloc.next());
    }

    let mut pages: Vec<PlannedPage<'_>> = Vec::new();
    for sheet in wb.sheets() {
        let setup = &sheet.page_setup;
        let boxes = paginate(sheet, setup)?;
        for page in boxes {
            pages.push(PlannedPage { sheet, setup, page });
        }
        if pages.len() > MAX_PDF_PAGES {
            return Err(CoreError::new(
                "pdf.limit",
                format!("PDF has more than {MAX_PDF_PAGES} pages"),
            ));
        }
    }
    if pages.is_empty() {
        let sheet = wb
            .sheets()
            .next()
            .ok_or_else(|| error::pdf_write("workbook has no sheets"))?;
        pages.push(PlannedPage {
            sheet,
            setup: &sheet.page_setup,
            page: PageBox {
                row0: 0,
                row1: 0,
                col0: 0,
                col1: 0,
                scale: 1.0,
                page: 1,
                pages: 1,
            },
        });
    }

    let page_ids: Vec<Ref> = (0..pages.len()).map(|_| alloc.next()).collect();
    let content_ids: Vec<Ref> = (0..pages.len()).map(|_| alloc.next()).collect();
    let mut annot_ids: Vec<Vec<Ref>> = Vec::new();

    let mut pdf = Pdf::new();
    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.pages(page_tree_id)
        .kids(page_ids.iter().copied())
        .count(page_ids.len() as i32);

    write_font(
        &mut pdf,
        font_id,
        &extra_font_ids,
        face.as_ref(),
        font_data.as_deref(),
    )?;

    let mut stream_bytes = font_data.as_ref().map_or(0, Vec::len);
    for (i, planned) in pages.iter().enumerate() {
        let (media_w, media_h) = planned.setup.media_pt();
        let links = collect_links(wb, planned);
        let mut this_annots = Vec::new();
        for _ in &links {
            this_annots.push(alloc.next());
        }
        {
            let mut page = pdf.page(page_ids[i]);
            page.media_box(Rect::new(0.0, 0.0, media_w as f32, media_h as f32));
            page.parent(page_tree_id);
            page.contents(content_ids[i]);
            page.resources().fonts().pair(FONT_NAME, font_id);
            if !this_annots.is_empty() {
                page.annotations(this_annots.iter().copied());
            }
        }
        for (annot_id, link) in this_annots.iter().zip(&links) {
            let mut annot = pdf.annotation(*annot_id);
            annot.subtype(AnnotationType::Link);
            annot.rect(link.rect);
            annot.page(page_ids[i]);
            annot
                .action()
                .action_type(ActionType::Uri)
                .uri(Str(link.uri.as_bytes()));
        }
        let content = build_content(
            wb,
            planned,
            face.as_ref(),
            &options.file_name,
            &options.theme,
        )?;
        stream_bytes = stream_bytes.saturating_add(content.len());
        if stream_bytes > MAX_PDF_STREAM_BYTES {
            return Err(CoreError::new(
                "pdf.limit",
                format!(
                    "PDF streams exceed {} MiB",
                    MAX_PDF_STREAM_BYTES / (1024 * 1024)
                ),
            )
            .with_hint("set a smaller print area or reduce cell text"));
        }
        pdf.stream(content_ids[i], &content);
        annot_ids.push(this_annots);
    }

    let _ = annot_ids;
    Ok(pdf.finish())
}

fn read_font(path: &Path) -> Result<Vec<u8>, CoreError> {
    let meta = std::fs::metadata(path).map_err(|err| error::pdf_write(err.to_string()))?;
    if meta.len() > MAX_EMBEDDED_FONT_BYTES {
        return Err(CoreError::new(
            "pdf.limit",
            format!(
                "font is {} bytes; maximum is {MAX_EMBEDDED_FONT_BYTES}",
                meta.len()
            ),
        ));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(meta.len()).unwrap_or(0));
    File::open(path)
        .map_err(|err| error::pdf_write(err.to_string()))?
        .take(MAX_EMBEDDED_FONT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| error::pdf_write(err.to_string()))?;
    if bytes.len() as u64 > MAX_EMBEDDED_FONT_BYTES {
        return Err(CoreError::new(
            "pdf.limit",
            format!("font exceeds {MAX_EMBEDDED_FONT_BYTES} bytes"),
        ));
    }
    Ok(bytes)
}

struct Alloc(i32);

impl Alloc {
    fn new() -> Self {
        Self(1)
    }

    fn next(&mut self) -> Ref {
        let id = Ref::new(self.0);
        self.0 += 1;
        id
    }
}

#[derive(Default)]
struct ExtraFontIds {
    cid: Option<Ref>,
    desc: Option<Ref>,
    cmap: Option<Ref>,
    stream: Option<Ref>,
}

struct PlannedPage<'a> {
    sheet: &'a Sheet,
    setup: &'a PageSetup,
    page: PageBox,
}

struct LinkAnn {
    rect: Rect,
    uri: String,
}

fn write_font(
    pdf: &mut Pdf,
    font_id: Ref,
    extra: &ExtraFontIds,
    face: Option<&ttf_parser::Face<'_>>,
    font_data: Option<&[u8]>,
) -> Result<(), CoreError> {
    let Some(face) = face else {
        pdf.type1_font(font_id).base_font(Name(b"Helvetica"));
        return Ok(());
    };
    let Some(cid_id) = extra.cid else {
        pdf.type1_font(font_id).base_font(Name(b"Helvetica"));
        return Ok(());
    };
    let desc_id = extra
        .desc
        .ok_or_else(|| error::pdf_write("font descriptor"))?;
    let cmap_id = extra.cmap.ok_or_else(|| error::pdf_write("font cmap"))?;
    let stream_id = extra
        .stream
        .ok_or_else(|| error::pdf_write("font stream"))?;
    let postscript = face
        .names()
        .into_iter()
        .find(|n| n.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
        .and_then(|n| n.to_string())
        .unwrap_or_else(|| "Embedded".into());
    let base = format!("OMA1+{postscript}");
    let base_bytes = base.as_bytes().to_vec();
    pdf.type0_font(font_id)
        .base_font(Name(&base_bytes))
        .encoding_predefined(Name(b"Identity-H"))
        .descendant_font(cid_id)
        .to_unicode(cmap_id);
    let units = f32::from(face.units_per_em().max(1));
    let to_pdf = |v: f32| v / units * 1000.0;
    let mut widths = vec![0.0f32; usize::from(face.number_of_glyphs())];
    for gid in 0..face.number_of_glyphs() {
        if let Some(w) = face.glyph_hor_advance(ttf_parser::GlyphId(gid)) {
            widths[usize::from(gid)] = to_pdf(f32::from(w));
        }
    }
    {
        let mut cid = pdf.cid_font(cid_id);
        cid.subtype(CidFontType::Type2)
            .base_font(Name(&base_bytes))
            .system_info(SYSTEM_INFO)
            .font_descriptor(desc_id)
            .cid_to_gid_map_predefined(Name(b"Identity"));
        cid.widths().consecutive(0, widths);
    }
    let bbox = face.global_bounding_box();
    {
        let mut desc = pdf.font_descriptor(desc_id);
        desc.name(Name(&base_bytes))
            .flags(FontFlags::NON_SYMBOLIC)
            .bbox(Rect::new(
                to_pdf(f32::from(bbox.x_min)),
                to_pdf(f32::from(bbox.y_min)),
                to_pdf(f32::from(bbox.x_max)),
                to_pdf(f32::from(bbox.y_max)),
            ))
            .italic_angle(face.italic_angle())
            .ascent(to_pdf(f32::from(face.ascender())))
            .descent(to_pdf(f32::from(face.descender())))
            .cap_height(to_pdf(f32::from(
                face.capital_height().unwrap_or(face.ascender()),
            )))
            .stem_v(80.0)
            .font_file2(stream_id);
    }
    let data = font_data.ok_or_else(|| error::pdf_write("missing ttf bytes"))?;
    {
        let mut stream = pdf.stream(stream_id, data);
        stream.pair(
            Name(b"Length1"),
            i32::try_from(data.len()).unwrap_or(i32::MAX),
        );
    }
    let mut cmap = UnicodeCmap::new(Name(b"Omacell"), SYSTEM_INFO);
    for cp in 0x20u32..=0x7E {
        if let Some(ch) = char::from_u32(cp)
            && let Some(gid) = face.glyph_index(ch)
        {
            cmap.pair(gid.0, ch);
        }
    }
    pdf.cmap(cmap_id, &cmap.finish());
    Ok(())
}

fn build_content(
    wb: &Workbook,
    planned: &PlannedPage<'_>,
    face: Option<&ttf_parser::Face<'_>>,
    file_name: &str,
    theme: &ChartTheme,
) -> Result<Vec<u8>, CoreError> {
    let setup = planned.setup;
    let page = &planned.page;
    let sheet = planned.sheet;
    let (media_w, media_h) = setup.media_pt();
    let left = setup.margins.left_pt();
    let top = setup.margins.top_pt();
    let scale = page.scale;
    let bw = setup.black_and_white;
    let mut content = Content::new();

    if let Some(header) = &setup.header {
        let text = expand_header(header, page, &sheet.name, file_name);
        show_text(
            &mut content,
            face,
            left,
            media_h - setup.margins.header_pt() - 10.0,
            9.0,
            &text,
            (0.0, 0.0, 0.0),
        );
    }
    if let Some(footer) = &setup.footer {
        let text = expand_header(footer, page, &sheet.name, file_name);
        show_text(
            &mut content,
            face,
            left,
            setup.margins.footer_pt(),
            9.0,
            &text,
            (0.0, 0.0, 0.0),
        );
    }

    let heading_w = if setup.headings { 28.0 } else { 0.0 };
    let heading_h = if setup.headings { 14.0 } else { 0.0 };
    let origin_x = left + heading_w;
    let mut y_from_top = heading_h;

    let (area_r0, area_c0, area_r1, area_c1) = print_bounds(sheet, setup);
    let title_rows = setup
        .title_rows
        .min(area_r1.saturating_sub(area_r0).saturating_add(1));
    let title_cols = setup
        .title_cols
        .min(area_c1.saturating_sub(area_c0).saturating_add(1));
    let title_r1 = area_r0.saturating_add(title_rows).saturating_sub(1);
    let title_c1 = area_c0.saturating_add(title_cols).saturating_sub(1);
    let title_c0 = (title_cols > 0).then_some(area_c0);
    let title_w = if title_cols == 0 {
        0.0
    } else {
        col_span_pt(sheet, area_c0, title_c1) * scale
    };
    let title_h = if title_rows == 0 {
        0.0
    } else {
        row_span_pt(sheet, area_r0, title_r1) * scale
    };
    let data_x = origin_x + title_w;

    if setup.headings {
        if title_cols > 0 {
            draw_col_labels(
                sheet,
                &mut content,
                face,
                area_c0,
                title_c1,
                origin_x,
                media_h - top - heading_h + 2.0,
                scale,
            );
        }
        draw_col_labels(
            sheet,
            &mut content,
            face,
            page.col0,
            page.col1,
            data_x,
            media_h - top - heading_h + 2.0,
            scale,
        );
    }

    if title_rows > 0 {
        if setup.headings {
            draw_row_labels(
                sheet,
                &mut content,
                face,
                area_r0,
                title_r1,
                left,
                media_h,
                top,
                y_from_top,
                scale,
            );
        }
        y_from_top += draw_split_band(
            wb,
            sheet,
            &mut content,
            face,
            area_r0,
            title_r1,
            title_c0,
            title_c1,
            page.col0,
            page.col1,
            origin_x,
            data_x,
            media_h,
            top,
            y_from_top,
            scale,
            setup.gridlines,
            bw,
        );
    }

    if setup.headings {
        draw_row_labels(
            sheet,
            &mut content,
            face,
            page.row0,
            page.row1,
            left,
            media_h,
            top,
            y_from_top,
            scale,
        );
    }
    y_from_top += draw_split_band(
        wb,
        sheet,
        &mut content,
        face,
        page.row0,
        page.row1,
        title_c0,
        title_c1,
        page.col0,
        page.col1,
        origin_x,
        data_x,
        media_h,
        top,
        y_from_top,
        scale,
        setup.gridlines,
        bw,
    );

    let _ = (y_from_top, media_w);

    for chart in &sheet.charts {
        let overlaps = chart.anchor.to_row >= page.row0
            && chart.anchor.from_row <= page.row1
            && chart.anchor.to_col >= page.col0
            && chart.anchor.from_col <= page.col1;
        if !overlaps {
            continue;
        }
        let scene = layout_chart(wb, chart, theme, 320.0, 200.0)?;
        let x0 =
            data_x + col_offset_pt(sheet, page.col0, chart.anchor.from_col.max(page.col0)) * scale;
        let y0 = heading_h
            + title_h
            + row_offset_pt(sheet, page.row0, chart.anchor.from_row.max(page.row0)) * scale;
        let w = col_span_pt(
            sheet,
            chart.anchor.from_col.max(page.col0),
            chart.anchor.to_col.min(page.col1),
        ) * scale;
        let h = row_span_pt(
            sheet,
            chart.anchor.from_row.max(page.row0),
            chart.anchor.to_row.min(page.row1),
        ) * scale;
        paint_scene(
            &mut content,
            face,
            &scene,
            x0,
            media_h - top - y0 - h,
            w.max(1.0),
            h.max(1.0),
            bw,
        );
    }

    Ok(content.finish())
}

#[allow(clippy::too_many_arguments)]
fn draw_col_labels(
    sheet: &Sheet,
    content: &mut Content,
    face: Option<&ttf_parser::Face<'_>>,
    c0: u16,
    c1: u16,
    origin_x: f64,
    y: f64,
    scale: f64,
) {
    if c1 < c0 {
        return;
    }
    let mut x = origin_x;
    for col in c0..=c1 {
        let w = col_pt(sheet, col) * scale;
        let label = col_to_letters(col).unwrap_or_else(|_| "?".into());
        show_text(content, face, x + 2.0, y, 8.0, &label, (0.2, 0.2, 0.2));
        x += w;
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_row_labels(
    sheet: &Sheet,
    content: &mut Content,
    face: Option<&ttf_parser::Face<'_>>,
    r0: u32,
    r1: u32,
    left: f64,
    media_h: f64,
    top: f64,
    y_from_top: f64,
    scale: f64,
) {
    if r1 < r0 {
        return;
    }
    let mut y = y_from_top;
    for row in r0..=r1 {
        let h = row_pt(sheet, row) * scale;
        if h > 0.0 {
            show_text(
                content,
                face,
                left + 2.0,
                media_h - top - y - h + (h * 0.25).max(1.0),
                8.0,
                &(row + 1).to_string(),
                (0.2, 0.2, 0.2),
            );
        }
        y += h;
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_split_band(
    wb: &Workbook,
    sheet: &Sheet,
    content: &mut Content,
    face: Option<&ttf_parser::Face<'_>>,
    r0: u32,
    r1: u32,
    title_c0: Option<u16>,
    title_c1: u16,
    data_c0: u16,
    data_c1: u16,
    title_x: f64,
    data_x: f64,
    media_h: f64,
    top: f64,
    y_from_top: f64,
    scale: f64,
    gridlines: bool,
    bw: bool,
) -> f64 {
    if let Some(title_c0) = title_c0 {
        draw_row_band(
            wb, sheet, content, face, r0, r1, title_c0, title_c1, title_x, media_h, top,
            y_from_top, scale, gridlines, bw,
        );
    }
    if data_c1 >= data_c0 {
        draw_row_band(
            wb, sheet, content, face, r0, r1, data_c0, data_c1, data_x, media_h, top, y_from_top,
            scale, gridlines, bw,
        );
    }
    row_span_pt(sheet, r0, r1) * scale
}

#[allow(clippy::too_many_arguments)]
fn draw_row_band(
    wb: &Workbook,
    sheet: &Sheet,
    content: &mut Content,
    face: Option<&ttf_parser::Face<'_>>,
    r0: u32,
    r1: u32,
    c0: u16,
    c1: u16,
    origin_x: f64,
    media_h: f64,
    top: f64,
    y_from_top: f64,
    scale: f64,
    gridlines: bool,
    bw: bool,
) -> f64 {
    if r1 < r0 {
        return 0.0;
    }
    let mut y = y_from_top;
    for row in r0..=r1 {
        let h = row_pt(sheet, row) * scale;
        if h <= 0.0 {
            continue;
        }
        let mut x = origin_x;
        for col in c0..=c1 {
            let w = col_pt(sheet, col) * scale;
            if w <= 0.0 {
                continue;
            }
            let pdf_y = media_h - top - y - h;
            if gridlines {
                content.set_stroke_rgb(0.75, 0.75, 0.75);
                content.set_line_width(0.3);
                content.rect(x as f32, pdf_y as f32, w as f32, h as f32);
                content.stroke();
            }
            let text = cell_text(wb, sheet.id, row, col);
            if !text.is_empty() {
                let rgb = if bw {
                    (0.0, 0.0, 0.0)
                } else {
                    (0.05, 0.05, 0.05)
                };
                show_text(
                    content,
                    face,
                    x + 2.0,
                    pdf_y + (h * 0.25).max(1.0),
                    (10.0 * scale).clamp(6.0, 18.0),
                    &text,
                    rgb,
                );
            }
            x += w;
        }
        y += h;
    }
    y - y_from_top
}

fn cell_text(wb: &Workbook, sheet: SheetId, row: u32, col: u16) -> String {
    let Ok(Some(slot)) = wb.get(sheet, row, col) else {
        return String::new();
    };
    match slot.value {
        Value::Empty | Value::Array(_) => String::new(),
        Value::Number(n) => {
            let code = wb
                .intern()
                .styles
                .get(slot.style)
                .map(|s| s.num_fmt)
                .and_then(|id| wb.num_fmt_code(id))
                .unwrap_or_else(|| "General".into());
            format(FormatValue::Number(n), code.as_ref(), LocaleId::EN_US).text
        }
        Value::Bool(true) => "TRUE".into(),
        Value::Bool(false) => "FALSE".into(),
        Value::Text(id) => wb.intern().strings.get(id).unwrap_or("").to_string(),
        Value::Error(kind) => format!("#{}", kind.as_str().trim_start_matches('#')),
    }
}

fn show_text(
    content: &mut Content,
    face: Option<&ttf_parser::Face<'_>>,
    x: f64,
    y: f64,
    size: f64,
    text: &str,
    rgb: (f32, f32, f32),
) {
    if text.is_empty() {
        return;
    }
    content.begin_text();
    content.set_fill_rgb(rgb.0, rgb.1, rgb.2);
    content.set_font(FONT_NAME, size as f32);
    content.next_line(x as f32, y as f32);
    if let Some(face) = face {
        let mut encoded = Vec::new();
        for ch in text.chars() {
            if let Some(gid) = face.glyph_index(ch) {
                encoded.extend_from_slice(&gid.0.to_be_bytes());
            }
        }
        if !encoded.is_empty() {
            content.show(Str(&encoded));
        }
    } else {
        let bytes: Vec<u8> = text
            .chars()
            .map(|c| if c as u32 <= 255 { c as u8 } else { b'?' })
            .collect();
        content.show(Str(&bytes));
    }
    content.end_text();
}

#[allow(clippy::too_many_arguments)]
fn paint_scene(
    content: &mut Content,
    face: Option<&ttf_parser::Face<'_>>,
    scene: &omacell_core::chart::Scene,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    bw: bool,
) {
    let sx = w / f64::from(scene.width.max(1.0));
    let sy = h / f64::from(scene.height.max(1.0));
    let map_x = |px: f32| x + f64::from(px) * sx;
    let map_y = |py: f32| y + h - f64::from(py) * sy;
    for op in &scene.ops {
        match op {
            Op::FillRect {
                x: rx,
                y: ry,
                w: rw,
                h: rh,
                color,
            } => {
                let (r, g, b) = rgb(color, bw);
                content.set_fill_rgb(r, g, b);
                let pdf_h = f64::from(*rh) * sy;
                content.rect(
                    map_x(*rx) as f32,
                    (map_y(*ry) - pdf_h) as f32,
                    (f64::from(*rw) * sx) as f32,
                    pdf_h as f32,
                );
                content.fill_nonzero();
            }
            Op::Polyline {
                points,
                color,
                width,
            } => {
                let Some((x0, y0)) = points.first() else {
                    continue;
                };
                let (r, g, b) = rgb(color, bw);
                content.set_stroke_rgb(r, g, b);
                content.set_line_width(*width * sx as f32);
                content.move_to(map_x(*x0) as f32, map_y(*y0) as f32);
                for (px, py) in points.iter().skip(1) {
                    content.line_to(map_x(*px) as f32, map_y(*py) as f32);
                }
                content.stroke();
            }
            Op::Polygon { points, color } => {
                let Some((x0, y0)) = points.first() else {
                    continue;
                };
                let (r, g, b) = rgb(color, bw);
                content.set_fill_rgb(r, g, b);
                content.move_to(map_x(*x0) as f32, map_y(*y0) as f32);
                for (px, py) in points.iter().skip(1) {
                    content.line_to(map_x(*px) as f32, map_y(*py) as f32);
                }
                content.close_path();
                content.fill_nonzero();
            }
            Op::Circle {
                x: cx,
                y: cy,
                r,
                color,
            } => {
                let (red, g, b) = rgb(color, bw);
                content.set_fill_rgb(red, g, b);
                circle(
                    content,
                    map_x(*cx) as f32,
                    map_y(*cy) as f32,
                    (*r as f64 * sx) as f32,
                );
                content.fill_nonzero();
            }
            Op::Text {
                x: tx,
                y: ty,
                text,
                color,
                size,
            } => {
                let (r, g, b) = rgb(color, bw);
                show_text(
                    content,
                    face,
                    map_x(*tx),
                    map_y(*ty),
                    f64::from(*size) * sy,
                    text,
                    (r, g, b),
                );
            }
        }
    }
}

fn circle(content: &mut Content, cx: f32, cy: f32, r: f32) {
    let k = 0.5523 * r;
    content.move_to(cx + r, cy);
    content.cubic_to(cx + r, cy + k, cx + k, cy + r, cx, cy + r);
    content.cubic_to(cx - k, cy + r, cx - r, cy + k, cx - r, cy);
    content.cubic_to(cx - r, cy - k, cx - k, cy - r, cx, cy - r);
    content.cubic_to(cx + k, cy - r, cx + r, cy - k, cx + r, cy);
    content.close_path();
}

fn rgb(hex: &str, bw: bool) -> (f32, f32, f32) {
    let t = hex.trim().trim_start_matches('#');
    let n = u32::from_str_radix(t.get(..6).unwrap_or("000000"), 16).unwrap_or(0);
    let r = ((n >> 16) & 0xFF) as f32 / 255.0;
    let g = ((n >> 8) & 0xFF) as f32 / 255.0;
    let b = (n & 0xFF) as f32 / 255.0;
    if bw {
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        (y, y, y)
    } else {
        (r, g, b)
    }
}

fn row_pt(sheet: &Sheet, row: u32) -> f64 {
    f64::from(sheet.geometry.rows.size(row).unwrap_or(DEFAULT_ROW_PX)) * PX_TO_PT
}

fn col_pt(sheet: &Sheet, col: u16) -> f64 {
    f64::from(
        sheet
            .geometry
            .cols
            .size(u32::from(col))
            .unwrap_or(DEFAULT_COL_PX),
    ) * PX_TO_PT
}

fn row_span_pt(sheet: &Sheet, r0: u32, r1: u32) -> f64 {
    if r1 < r0 {
        return 0.0;
    }
    (r0..=r1).map(|r| row_pt(sheet, r)).sum()
}

fn col_span_pt(sheet: &Sheet, c0: u16, c1: u16) -> f64 {
    if c1 < c0 {
        return 0.0;
    }
    (c0..=c1).map(|c| col_pt(sheet, c)).sum()
}

fn row_offset_pt(sheet: &Sheet, from: u32, to: u32) -> f64 {
    if to <= from {
        0.0
    } else {
        row_span_pt(sheet, from, to.saturating_sub(1))
    }
}

fn col_offset_pt(sheet: &Sheet, from: u16, to: u16) -> f64 {
    if to <= from {
        0.0
    } else {
        col_span_pt(sheet, from, to.saturating_sub(1))
    }
}

fn collect_links(_wb: &Workbook, planned: &PlannedPage<'_>) -> Vec<LinkAnn> {
    let sheet = planned.sheet;
    let page = &planned.page;
    let setup = planned.setup;
    let (_, media_h) = setup.media_pt();
    let left = setup.margins.left_pt();
    let top = setup.margins.top_pt();
    let scale = page.scale;
    let heading_w = if setup.headings { 28.0 } else { 0.0 };
    let heading_h = if setup.headings { 14.0 } else { 0.0 };
    let (area_r0, area_c0, area_r1, area_c1) = print_bounds(sheet, setup);
    let title_rows = setup
        .title_rows
        .min(area_r1.saturating_sub(area_r0).saturating_add(1));
    let title_cols = setup
        .title_cols
        .min(area_c1.saturating_sub(area_c0).saturating_add(1));
    let title_r1 = area_r0.saturating_add(title_rows).saturating_sub(1);
    let title_c1 = area_c0.saturating_add(title_cols).saturating_sub(1);
    let title_w = if title_cols == 0 {
        0.0
    } else {
        col_span_pt(sheet, area_c0, title_c1) * scale
    };
    let title_h = if title_rows == 0 {
        0.0
    } else {
        row_span_pt(sheet, area_r0, title_r1) * scale
    };
    let mut out = Vec::new();
    let mut links: Vec<((u32, u16), &Hyperlink)> =
        sheet.hyperlinks.iter().map(|(k, v)| (*k, v)).collect();
    links.sort_by_key(|(k, _)| *k);
    for ((row, col), link) in links {
        let title_col = title_cols > 0 && (area_c0..=title_c1).contains(&col);
        let data_col = page.col1 >= page.col0 && (page.col0..=page.col1).contains(&col);
        let title_row = title_rows > 0 && (area_r0..=title_r1).contains(&row);
        let data_row = page.row1 >= page.row0 && (page.row0..=page.row1).contains(&row);
        if !(title_col || data_col) || !(title_row || data_row) {
            continue;
        }
        if !(link.target.starts_with("http://") || link.target.starts_with("https://")) {
            continue;
        }
        let x = left
            + heading_w
            + if title_col {
                col_offset_pt(sheet, area_c0, col) * scale
            } else {
                title_w + col_offset_pt(sheet, page.col0, col) * scale
            };
        let y_top = heading_h
            + if title_row {
                row_offset_pt(sheet, area_r0, row) * scale
            } else {
                title_h + row_offset_pt(sheet, page.row0, row) * scale
            };
        let w = col_pt(sheet, col) * scale;
        let h = row_pt(sheet, row) * scale;
        let pdf_y = media_h - top - y_top - h;
        out.push(LinkAnn {
            rect: Rect::new(x as f32, pdf_y as f32, (x + w) as f32, (pdf_y + h) as f32),
            uri: link.target.clone(),
        });
    }
    out
}

/// Count `/Type /Page` objects, ignoring `/Type /Pages` (golden tests).
#[must_use]
pub fn pdf_page_count(bytes: &[u8]) -> usize {
    let text = String::from_utf8_lossy(bytes);
    let mut n = 0;
    let mut rest = text.as_ref();
    while let Some(idx) = rest.find("/Type /Page") {
        let after = &rest[idx + "/Type /Page".len()..];
        if !after.starts_with('s') {
            n += 1;
        }
        rest = after;
    }
    n
}

/// Extract literal strings from content streams.
#[must_use]
pub fn pdf_extract_text(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            i += 1;
            while i < bytes.len() && bytes[i] != b')' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                }
                if bytes[i].is_ascii_graphic() || bytes[i] == b' ' {
                    out.push(bytes[i] as char);
                }
                i += 1;
            }
            out.push(' ');
        }
        i += 1;
    }
    out
}

/// Whether a TrueType program was embedded.
#[must_use]
pub fn pdf_has_fontfile2(bytes: &[u8]) -> bool {
    String::from_utf8_lossy(bytes).contains("/FontFile2")
}

/// First `/MediaBox` width × height.
#[must_use]
pub fn pdf_media_box(bytes: &[u8]) -> Option<(f64, f64)> {
    let text = String::from_utf8_lossy(bytes);
    let idx = text.find("/MediaBox")?;
    let rest = &text[idx..];
    let start = rest.find('[')?;
    let end = rest.find(']')?;
    let nums: Vec<f64> = rest[start + 1..end]
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    if nums.len() == 4 {
        Some((nums[2] - nums[0], nums[3] - nums[1]))
    } else {
        None
    }
}
