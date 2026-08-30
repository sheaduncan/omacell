//! Serialize a [`Scene`](super::scene::Scene) to SVG.

use super::scene::{Op, Scene};

/// Render `scene` as an SVG document.
#[must_use]
pub fn to_svg(scene: &Scene) -> String {
    let mut out = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">"#,
        w = scene.width,
        h = scene.height
    );
    for op in &scene.ops {
        match op {
            Op::FillRect { x, y, w, h, color } => {
                let color = svg_color(color);
                out.push_str(&format!(
                    r#"<rect x="{x:.2}" y="{y:.2}" width="{w:.2}" height="{h:.2}" fill="{color}"/>"#
                ));
            }
            Op::Polyline {
                points,
                color,
                width,
            } => {
                if points.is_empty() {
                    continue;
                }
                let color = svg_color(color);
                out.push_str(&format!(
                    r#"<polyline fill="none" stroke="{color}" stroke-width="{width:.2}" points=""#
                ));
                for (i, (x, y)) in points.iter().enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    out.push_str(&format!("{x:.2},{y:.2}"));
                }
                out.push_str("\"/>");
            }
            Op::Polygon { points, color } => {
                if points.is_empty() {
                    continue;
                }
                let color = svg_color(color);
                out.push_str(&format!(r#"<polygon fill="{color}" points=""#));
                for (i, (x, y)) in points.iter().enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    out.push_str(&format!("{x:.2},{y:.2}"));
                }
                out.push_str("\"/>");
            }
            Op::Circle { x, y, r, color } => {
                let color = svg_color(color);
                out.push_str(&format!(
                    r#"<circle cx="{x:.2}" cy="{y:.2}" r="{r:.2}" fill="{color}"/>"#
                ));
            }
            Op::Text {
                x,
                y,
                text,
                color,
                size,
            } => {
                let color = svg_color(color);
                out.push_str(&format!(
                    r#"<text x="{x:.2}" y="{y:.2}" fill="{color}" font-size="{size:.2}" font-family="sans-serif">{}</text>"#,
                    xml_escape(text)
                ));
            }
        }
    }
    out.push_str("</svg>");
    out
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if !is_xml10_char(c) {
            out.push('\u{FFFD}');
            continue;
        }
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn svg_color(color: &str) -> &str {
    let Some(hex) = color.strip_prefix('#') else {
        return "#000000";
    };
    if hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        color
    } else {
        "#000000"
    }
}

fn is_xml10_char(ch: char) -> bool {
    matches!(ch, '\u{9}' | '\u{A}' | '\u{D}')
        || ('\u{20}'..='\u{D7FF}').contains(&ch)
        || ('\u{E000}'..='\u{FFFD}').contains(&ch)
        || ('\u{10000}'..='\u{10FFFF}').contains(&ch)
}
