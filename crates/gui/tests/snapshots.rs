//! egui_kittest snapshots at 1×, 1.5×, 2× × three fixture themes.

mod common;

use common::launch_theme;
use egui_kittest::Harness;
use omacell_gui::Gui;

const THEMES: [&str; 3] = ["tokyo-night", "catppuccin-latte", "nord"];
const SCALES: [f32; 3] = [1.0, 1.5, 2.0];

#[test]
fn grid_snapshots_across_scales_and_themes() {
    for theme in THEMES {
        for scale in SCALES {
            let parts = launch_theme(Some(theme));
            let mut harness = Harness::builder()
                .with_size(egui::vec2(640.0, 400.0))
                .with_pixels_per_point(scale)
                .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
            harness.run();
            let name = format!("grid_{theme}_{scale}x");
            harness.snapshot(&name);
            if (scale - 1.0).abs() < f32::EPSILON {
                let image = harness.render().expect("render snapshot");
                assert_crisp_gridlines(
                    image.width() as usize,
                    image.height() as usize,
                    |x, y| image.get_pixel(x as u32, y as u32).0,
                    theme,
                );
            }
        }
    }
}

fn assert_crisp_gridlines(
    w: usize,
    h: usize,
    pixel: impl Fn(usize, usize) -> [u8; 4],
    theme: &str,
) {
    assert!(w > 80 && h > 80, "{theme} snapshot too small: {w}x{h}");
    let mut best = usize::MAX;
    let mut saw_lines = false;
    for y in 120..h.saturating_sub(60) {
        let mut counts = std::collections::BTreeMap::<[u8; 3], usize>::new();
        for x in 80..w.saturating_sub(40) {
            let px = pixel(x, y);
            *counts.entry([px[0], px[1], px[2]]).or_default() += 1;
        }
        let Some((bg, _)) = counts.into_iter().max_by_key(|(_, n)| *n) else {
            continue;
        };
        let mut max_run = 0usize;
        let mut run = 0usize;
        let mut line_runs = 0usize;
        for x in 80..w.saturating_sub(40) {
            let px = pixel(x, y);
            if color_dist([px[0], px[1], px[2]], bg) > 12 {
                run += 1;
            } else {
                if run > 0 {
                    max_run = max_run.max(run);
                    line_runs += 1;
                }
                run = 0;
            }
        }
        max_run = max_run.max(run);
        if line_runs >= 4 {
            saw_lines = true;
            best = best.min(max_run);
        }
    }
    assert!(saw_lines, "{theme} expected vertical gridlines in the body");
    assert!(
        best <= 2,
        "{theme} expected ~1 px gridlines, saw run {best}"
    );
}

fn color_dist(a: [u8; 3], b: [u8; 3]) -> u16 {
    a.iter().zip(b).map(|(l, r)| u16::from(l.abs_diff(r))).sum()
}
