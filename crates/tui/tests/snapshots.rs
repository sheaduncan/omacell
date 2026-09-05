//! TestBackend snapshots: 80×24, 120×40, 200×60 × three fixture themes.

mod common;

use common::{draw_text, harness_theme, harness_workbook, seed_demo};
use insta::assert_snapshot;
use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::chart::{Axis, Chart, ChartAnchor, ChartId, ChartKind, LegendPos, Series};
use omacell_core::sheet::SplitView;
use omacell_core::workbook::Workbook;

const THEMES: [&str; 3] = ["tokyo-night", "catppuccin-latte", "nord"];
const SIZES: [(u16, u16); 3] = [(80, 24), (120, 40), (200, 60)];

#[test]
fn frame_snapshots_across_sizes_and_themes() {
    for theme in THEMES {
        let mut h = harness_theme(theme);
        seed_demo(&mut h.tui);
        for (w, ht) in SIZES {
            let text = draw_text(&h.tui, w, ht);
            assert!(
                text.contains("Hello") || text.contains("fx"),
                "{theme} {w}x{ht} missing grid/formula chrome:\n{text}"
            );
            assert_snapshot!(format!("{theme}_{w}x{ht}"), text);
        }
    }
}

#[test]
#[ignore = "nightly wall-clock performance gate"]
fn first_frame_completes() {
    let start = std::time::Instant::now();
    let mut h = harness_theme("nord");
    seed_demo(&mut h.tui);
    let _ = draw_text(&h.tui, 80, 24);
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    eprintln!("tui_first_frame_ms={ms:.2}");
    eprintln!(
        "OMACELL_PERF_RESULT {}",
        serde_json::json!({"id": "tui_cold_start_ms", "value": ms})
    );
    assert!(
        ms < 5_000.0,
        "first TestBackend frame took {ms:.2} ms (CI-safe bound; §12.1 is 100 ms in release)"
    );
}

#[test]
fn formula_bar_shows_active_cell() {
    let mut h = harness_theme("nord");
    seed_demo(&mut h.tui);
    let text = draw_text(&h.tui, 80, 24);
    assert!(text.contains("fx"), "{text}");
    assert!(text.contains("Hello"), "{text}");
}

#[test]
fn split_view_renders_leading_and_scrolled_panes_with_dividers() {
    let mut h = harness_theme("nord");
    seed_demo(&mut h.tui);
    h.tui
        .execute_cmd(
            "cell.set",
            serde_json::json!({"ref": "F8", "input": "SCROLLED"}),
        )
        .unwrap();
    common::wait_tasks(&mut h.tui);
    let mut viewport = h.tui.ui().viewport();
    viewport.first_row = 7;
    viewport.first_col = 5;
    viewport.split = Some(SplitView {
        x_px: 128,
        y_px: 40,
    });
    h.tui.ui().set_viewport(viewport);

    let text = draw_text(&h.tui, 80, 24);

    assert!(text.contains("Hello"), "leading pane missing:\n{text}");
    assert!(text.contains("SCROLLED"), "scrolled pane missing:\n{text}");
    assert!(
        text.contains('┃'),
        "vertical split divider missing:\n{text}"
    );
    assert!(
        text.contains('─'),
        "horizontal split divider missing:\n{text}"
    );
}

#[test]
fn formula_references_use_rgb_when_truecolor_on() {
    use omacell_ui::EditSurface;
    use ratatui::style::Color;

    let h = common::harness_opts(Some("tokyo-night"), "keys/classic.toml", "on");
    h.tui.ui().begin_edit(EditSurface::FormulaBar, "=A1+B2");
    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    h.tui.draw(&mut terminal).unwrap();
    let buf = terminal.backend().buffer();
    let area = buf.area();
    let mut saw_rgb = false;
    for y in area.y..area.y.saturating_add(area.height) {
        for x in area.x..area.x.saturating_add(area.width) {
            if matches!(buf[(x, y)].fg, Color::Rgb(_, _, _)) {
                saw_rgb = true;
            }
        }
    }
    assert!(saw_rgb, "expected file-origin RGB on formula references");
}

#[test]
fn workbook_text_cannot_emit_terminal_control_sequences() {
    let mut h = harness_theme("nord");
    let payload = "safe\u{1b}]52;c;dGVzdA==\u{7}text\u{202e}tail";
    let outcome = h
        .tui
        .execute_cmd(
            "cell.set",
            serde_json::json!({"ref": "A1", "input": payload}),
        )
        .unwrap();
    assert!(outcome.ok, "{:?}", outcome.error);
    common::wait_tasks(&mut h.tui);
    let text = draw_text(&h.tui, 80, 24);
    assert!(!text.contains('\u{1b}'));
    assert!(!text.contains('\u{7}'));
    assert!(!text.contains('\u{202e}'));
    assert!(text.contains('�'));
}

#[test]
fn modeled_chart_has_a_unicode_terminal_consumer() {
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    for (row, value) in [2.0, 5.0, 3.0, 8.0].into_iter().enumerate() {
        workbook.set_number(sheet, row as u32, 0, value).unwrap();
    }
    let values = RangeRef::from_corners(CellRef::new(0, 0).unwrap(), CellRef::new(3, 0).unwrap());
    workbook
        .add_chart(Chart {
            id: ChartId::new(0),
            kind: ChartKind::Line,
            title: Some("Quarterly Revenue".into()),
            categories: None,
            series: vec![Series {
                name: "Revenue".into(),
                values,
                x: None,
                size: None,
                color: None,
                secondary_axis: false,
                trendline: None,
            }],
            category_axis: Axis::default(),
            value_axis: Axis::default(),
            secondary_axis: None,
            legend: LegendPos::None,
            data_labels: false,
            anchor: ChartAnchor {
                from_row: 2,
                from_col: 1,
                to_row: 12,
                to_col: 6,
            },
            sheet,
        })
        .unwrap();

    let h = harness_workbook(workbook);
    let text = draw_text(&h.tui, 80, 24);
    assert!(text.contains("Quarterly Revenue"), "{text}");
    assert!(
        text.chars()
            .any(|ch| ('\u{2800}'..='\u{28ff}').contains(&ch)),
        "expected braille chart marks: {text}"
    );
}
