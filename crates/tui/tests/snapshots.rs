//! TestBackend snapshots: 80×24, 120×40, 200×60 × three fixture themes.

mod common;

use common::{draw_text, harness_theme, seed_demo};
use insta::assert_snapshot;

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
fn first_frame_completes() {
    let start = std::time::Instant::now();
    let mut h = harness_theme("nord");
    seed_demo(&mut h.tui);
    let _ = draw_text(&h.tui, 80, 24);
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    eprintln!("tui_first_frame_ms={ms:.2}");
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
