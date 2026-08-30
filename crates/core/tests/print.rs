//! Pagination corpus (spec F-11.1). Each case cites the geometry it encodes.

use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::geometry::DEFAULT_ROW_PX;
use omacell_core::print::{Orientation, PX_TO_PT, PageSetup, PaperSize, expand_header, paginate};
use omacell_core::workbook::Workbook;

fn fill_grid(wb: &mut Workbook, rows: u32, cols: u16) {
    let sheet = wb.active_sheet();
    for r in 0..rows {
        for c in 0..cols {
            wb.set_number(sheet, r, c, f64::from(r * 100 + u32::from(c)))
                .unwrap();
        }
    }
}

fn letter_rows_per_page() -> u32 {
    // Letter 792pt, default 0.75in margins each side → 684pt usable.
    // DEFAULT_ROW_PX=20 → 15pt. 684/15 = 45.6 → 45 rows at 100% scale.
    let usable_h = 792.0 - 0.75 * 72.0 * 2.0;
    let row_pt = f64::from(DEFAULT_ROW_PX) * PX_TO_PT;
    (usable_h / row_pt).floor() as u32
}

#[test]
fn letter_portrait_page_count_matches_usable_height() {
    // Documented: Letter 8.5×11in, Excel default 0.75in margins, 15pt rows.
    let mut wb = Workbook::new();
    let per = letter_rows_per_page();
    fill_grid(&mut wb, per + 5, 1);
    let pages = paginate(wb.sheet(wb.active_sheet()).unwrap(), &PageSetup::default());
    assert_eq!(
        pages.len(),
        2,
        "50-ish default rows spill onto a second page"
    );
    assert_eq!(pages[0].row0, 0);
    assert_eq!(pages[0].row1, per - 1);
    assert_eq!(pages[1].row0, per);
    assert_eq!(pages[0].pages, 2);
    assert!((pages[0].scale - 1.0).abs() < f64::EPSILON);
}

#[test]
fn a4_is_taller_than_letter_so_same_grid_stays_on_one_page() {
    // A4 portrait 842pt vs Letter 792pt; 45 Letter rows fit on A4.
    let mut wb = Workbook::new();
    let per = letter_rows_per_page();
    fill_grid(&mut wb, per, 1);
    let setup = PageSetup {
        paper: PaperSize::A4,
        ..PageSetup::default()
    };
    let pages = paginate(wb.sheet(wb.active_sheet()).unwrap(), &setup);
    assert_eq!(pages.len(), 1);
    assert_eq!(setup.media_pt(), (595.0, 842.0));
}

#[test]
fn landscape_swaps_media_box() {
    let setup = PageSetup {
        orientation: Orientation::Landscape,
        ..PageSetup::default()
    };
    assert_eq!(setup.media_pt(), (792.0, 612.0));
}

#[test]
fn fit_to_one_by_one_uses_excel_floor_percent() {
    // floor(100 * min(usable/content)) clamped 10–400 (WP-26 plan).
    let mut wb = Workbook::new();
    fill_grid(&mut wb, 80, 4);
    let setup = PageSetup {
        fit_to_width: Some(1),
        fit_to_height: Some(1),
        ..PageSetup::default()
    };
    let pages = paginate(wb.sheet(wb.active_sheet()).unwrap(), &setup);
    assert_eq!(pages.len(), 1, "fit-to 1×1 must produce a single page");
    let content_h = f64::from(80 * DEFAULT_ROW_PX) * PX_TO_PT;
    let usable_h = setup.usable_pt().1;
    let expected = ((usable_h / content_h * 100.0).floor() as u32).clamp(10, 400);
    assert_eq!((pages[0].scale * 100.0).round() as u32, expected);
}

#[test]
fn manual_row_break_splits_before_the_named_row() {
    // OOXML brk id is 1-based first row of the new page; we store the 0-based
    // row *after* which that page starts (id 11 → store 9 → new page at row 10).
    let mut wb = Workbook::new();
    fill_grid(&mut wb, 20, 1);
    let setup = PageSetup {
        row_breaks: vec![9],
        ..PageSetup::default()
    };
    let pages = paginate(wb.sheet(wb.active_sheet()).unwrap(), &setup);
    assert!(pages.len() >= 2);
    assert_eq!(pages[0].row1, 9);
    assert_eq!(pages[1].row0, 10);
}

#[test]
fn print_area_clips_used_range() {
    let mut wb = Workbook::new();
    fill_grid(&mut wb, 30, 4);
    let setup = PageSetup {
        print_area: Some(RangeRef::from_corners(
            CellRef::new(2, 1).unwrap(),
            CellRef::new(4, 2).unwrap(),
        )),
        ..PageSetup::default()
    };
    let pages = paginate(wb.sheet(wb.active_sheet()).unwrap(), &setup);
    assert_eq!(pages[0].row0, 2);
    assert_eq!(pages[0].row1, 4);
    assert_eq!(pages[0].col0, 1);
    assert_eq!(pages[0].col1, 2);
}

#[test]
fn hidden_rows_contribute_zero_height() {
    let mut wb = Workbook::new();
    fill_grid(&mut wb, 10, 1);
    let sheet_id = wb.active_sheet();
    for r in 0..10 {
        wb.set_row_hidden(sheet_id, r, true).unwrap();
    }
    let pages = paginate(wb.sheet(sheet_id).unwrap(), &PageSetup::default());
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].row0, 0);
    assert_eq!(pages[0].row1, 9);
}

#[test]
fn expand_header_substitutes_excel_ampersand_fields() {
    let pages = {
        let mut wb = Workbook::new();
        fill_grid(&mut wb, 1, 1);
        paginate(wb.sheet(wb.active_sheet()).unwrap(), &PageSetup::default())
    };
    let text = expand_header("Page &P of &N — &A / &F", &pages[0], "Sheet1", "book.xlsx");
    assert_eq!(text, "Page 1 of 1 — Sheet1 / book.xlsx");
}
