//! Chart sampling, scene/SVG parity, and golden SVGs.

use omacell_core::addr::{CellRef, RangeRef, SheetId};
use omacell_core::chart::{
    ChartKind, ChartTheme, Sparkline, SparklineKind, Trendline, TrendlineKind, chart_from_range,
    layout, layout_sparkline, sample, to_svg,
};
use omacell_core::workbook::Workbook;

fn seed() -> Workbook {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Cat").unwrap();
    wb.set_text(s, 0, 1, "East").unwrap();
    wb.set_text(s, 0, 2, "West").unwrap();
    wb.set_text(s, 1, 0, "A").unwrap();
    wb.set_number(s, 1, 1, 10.0).unwrap();
    wb.set_number(s, 1, 2, 5.0).unwrap();
    wb.set_text(s, 2, 0, "B").unwrap();
    wb.set_number(s, 2, 1, 20.0).unwrap();
    wb.set_number(s, 2, 2, 8.0).unwrap();
    wb.set_text(s, 3, 0, "C").unwrap();
    wb.set_number(s, 3, 1, 12.0).unwrap();
    wb.set_number(s, 3, 2, 15.0).unwrap();
    wb
}

fn range(r0: u32, c0: u16, r1: u32, c1: u16) -> RangeRef {
    RangeRef::from_corners(
        CellRef::new(r0, c0).unwrap().on_sheet(SheetId::new(0)),
        CellRef::new(r1, c1).unwrap().on_sheet(SheetId::new(0)),
    )
}

fn latte() -> ChartTheme {
    ChartTheme {
        background: "#eff1f5".into(),
        foreground: "#4c4f69".into(),
        axis: "#6c6f85".into(),
        gridline: "#ccd0da".into(),
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
    }
}

fn nord() -> ChartTheme {
    ChartTheme {
        background: "#2e3440".into(),
        foreground: "#eceff4".into(),
        axis: "#d8dee9".into(),
        gridline: "#4c566a".into(),
        palette: [
            "#88c0d0".into(),
            "#a3be8c".into(),
            "#ebcb8b".into(),
            "#bf616a".into(),
            "#b48ead".into(),
            "#81a1c1".into(),
            "#d08770".into(),
            "#eceff4".into(),
        ],
    }
}

const KINDS: [ChartKind; 14] = [
    ChartKind::Line,
    ChartKind::Column,
    ChartKind::Bar,
    ChartKind::ColumnStacked,
    ChartKind::BarStacked,
    ChartKind::ColumnPct,
    ChartKind::BarPct,
    ChartKind::Area,
    ChartKind::Pie,
    ChartKind::Donut,
    ChartKind::Scatter,
    ChartKind::Bubble,
    ChartKind::Combo,
    ChartKind::Histogram,
];

#[test]
fn sample_reads_values_and_updates_after_edit() {
    let mut wb = seed();
    let chart = chart_from_range(
        &wb,
        wb.active_sheet(),
        range(0, 0, 3, 2),
        ChartKind::Column,
        Some("Sales".into()),
    )
    .unwrap();
    let first = sample(&wb, &chart).unwrap();
    assert_eq!(first.series.len(), 2);
    assert_eq!(first.series[0].y[0], 10.0);
    wb.set_number(wb.active_sheet(), 1, 1, 99.0).unwrap();
    let second = sample(&wb, &chart).unwrap();
    assert_eq!(second.series[0].y[0], 99.0);
}

#[test]
fn scatter_and_bubble_use_numeric_x_y_columns() {
    let wb = seed();
    for kind in [ChartKind::Scatter, ChartKind::Bubble] {
        let chart =
            chart_from_range(&wb, wb.active_sheet(), range(0, 0, 3, 2), kind, None).unwrap();
        let sampled = sample(&wb, &chart).unwrap();
        assert_eq!(sampled.series.len(), 1);
        assert_eq!(sampled.series[0].x, vec![10.0, 20.0, 12.0]);
        assert_eq!(sampled.series[0].y, vec![5.0, 8.0, 15.0]);
        let scene = layout(&sampled, &chart, &ChartTheme::neutral(), 320.0, 200.0);
        assert!(
            scene
                .ops
                .iter()
                .any(|op| matches!(op, omacell_core::chart::Op::Circle { .. })),
            "{kind:?} must contain visible data points"
        );
    }
}

#[test]
fn oversized_chart_range_is_rejected_before_sampling() {
    let wb = Workbook::new();
    let huge = range(0, 0, 100_000, 10);
    let error = chart_from_range(&wb, wb.active_sheet(), huge, ChartKind::Line, None).unwrap_err();
    assert_eq!(error.code, "chart.limit");
}

#[test]
fn every_kind_emits_svg_for_three_palettes() {
    let wb = seed();
    let themes = [
        ("tokyo-night", ChartTheme::neutral()),
        ("catppuccin-latte", latte()),
        ("nord", nord()),
    ];
    for kind in KINDS {
        let mut chart = chart_from_range(
            &wb,
            wb.active_sheet(),
            range(0, 0, 3, 2),
            kind,
            Some(kind.as_str().into()),
        )
        .unwrap();
        if kind == ChartKind::Line {
            chart.series[0].trendline = Some(Trendline {
                kind: TrendlineKind::Linear,
                period: 2,
            });
        }
        let sampled = sample(&wb, &chart).unwrap();
        for (theme_name, theme) in &themes {
            let scene = layout(&sampled, &chart, theme, 480.0, 280.0);
            assert!(!scene.ops.is_empty(), "{kind:?} {theme_name}");
            let svg = to_svg(&scene);
            assert!(svg.starts_with("<svg"), "{kind:?} {theme_name}");
            assert!(svg.contains(&theme.palette[0]), "{kind:?} {theme_name}");
            insta::assert_snapshot!(format!("chart_{}_{theme_name}", kind.as_str()), svg);
        }
    }
}

#[test]
fn scene_and_svg_are_the_same_ops() {
    let wb = seed();
    let chart = chart_from_range(
        &wb,
        wb.active_sheet(),
        range(0, 0, 3, 2),
        ChartKind::Column,
        None,
    )
    .unwrap();
    let sampled = sample(&wb, &chart).unwrap();
    let scene = layout(&sampled, &chart, &ChartTheme::neutral(), 320.0, 200.0);
    let svg = to_svg(&scene);
    let fills = scene
        .ops
        .iter()
        .filter(|op| matches!(op, omacell_core::chart::Op::FillRect { .. }))
        .count();
    assert_eq!(svg.matches("<rect ").count(), fills);
}

#[test]
fn shared_scene_renders_edited_axis_titles() {
    let wb = seed();
    let mut chart = chart_from_range(
        &wb,
        wb.active_sheet(),
        range(0, 0, 3, 2),
        ChartKind::Combo,
        Some("Sales".into()),
    )
    .unwrap();
    chart.category_axis.title = Some("Quarter".into());
    chart.value_axis.title = Some("Revenue".into());
    chart.secondary_axis.as_mut().unwrap().title = Some("Margin".into());
    let sampled = sample(&wb, &chart).unwrap();
    let scene = layout(&sampled, &chart, &ChartTheme::neutral(), 480.0, 280.0);
    let text = scene
        .ops
        .iter()
        .filter_map(|op| match op {
            omacell_core::chart::Op::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    for title in ["Sales", "Quarter", "Revenue", "Margin"] {
        assert!(text.contains(&title), "missing {title}: {text:?}");
    }
}

#[test]
fn sparkline_line_and_winloss_layout() {
    let spark = Sparkline {
        kind: SparklineKind::Line,
        data: range(1, 1, 3, 1),
        row: 1,
        col: 3,
        sheet: SheetId::new(0),
    };
    let scene = layout_sparkline(&[1.0, 3.0, 2.0], &spark, &ChartTheme::neutral(), 64.0, 18.0);
    assert!(
        scene
            .ops
            .iter()
            .any(|op| matches!(op, omacell_core::chart::Op::Polyline { .. }))
    );
    let mut win = spark;
    win.kind = SparklineKind::WinLoss;
    let scene = layout_sparkline(&[1.0, -1.0, 0.5], &win, &ChartTheme::neutral(), 64.0, 18.0);
    assert!(
        scene
            .ops
            .iter()
            .any(|op| matches!(op, omacell_core::chart::Op::FillRect { .. }))
    );
}

#[test]
fn workbook_stores_charts() {
    let mut wb = seed();
    let chart = chart_from_range(
        &wb,
        wb.active_sheet(),
        range(0, 0, 3, 2),
        ChartKind::Pie,
        Some("Share".into()),
    )
    .unwrap();
    let id = wb.add_chart(chart).unwrap();
    assert_eq!(wb.sheet(wb.active_sheet()).unwrap().charts[0].id, id);
}
