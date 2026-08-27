//! Load fontconfig `monospace` (and a CJK fallback) into egui.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

#[derive(Clone, Debug)]
pub struct LoadedFonts {
    pub monospace_family: String,
    pub monospace_path: PathBuf,
    pub cjk_family: Option<String>,
    pub cjk_path: Option<PathBuf>,
}

pub fn install(ctx: &egui::Context) -> LoadedFonts {
    let mut fonts = FontDefinitions::default();
    let mut loaded = LoadedFonts {
        monospace_family: "egui-default".into(),
        monospace_path: PathBuf::from("(bundled)"),
        cjk_family: None,
        cjk_path: None,
    };

    if let Some((path, family, index)) = fc_match("monospace") {
        if let Some(data) = read_font(&path, index) {
            fonts.font_data.insert("fc_mono".into(), Arc::new(data));
            if let Some(mono) = fonts.families.get_mut(&FontFamily::Monospace) {
                mono.insert(0, "fc_mono".into());
            }
            if let Some(prop) = fonts.families.get_mut(&FontFamily::Proportional) {
                prop.insert(0, "fc_mono".into());
            }
            loaded.monospace_family = family;
            loaded.monospace_path = path;
        }
    }

    // CJK fallback so IME composition is visible, not tofu.
    // `fc-match :lang=zh` often returns a Latin font that merely lists zh in
    // its lang set; prefer an actual CJK family, then a charset match.
    let cjk = fc_match("Noto Sans CJK SC")
        .or_else(|| fc_match("Noto Sans CJK JP"))
        .or_else(|| fc_match(":charset=4e00"));
    if let Some((path, family, index)) = cjk {
        if path != loaded.monospace_path {
            if let Some(data) = read_font(&path, index) {
                fonts.font_data.insert("fc_cjk".into(), Arc::new(data));
                for family_key in [FontFamily::Monospace, FontFamily::Proportional] {
                    if let Some(list) = fonts.families.get_mut(&family_key) {
                        list.push("fc_cjk".into());
                    }
                }
                loaded.cjk_family = Some(family);
                loaded.cjk_path = Some(path);
            }
        }
    }

    ctx.set_fonts(fonts);
    loaded
}

fn fc_match(query: &str) -> Option<(PathBuf, String, u32)> {
    let output = Command::new("fc-match")
        .args(["-f", "%{file}\t%{family}\t%{index}\n", query])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&output.stdout);
    let mut parts = line.trim().split('\t');
    let file = parts.next()?.trim();
    if file.is_empty() {
        return None;
    }
    let family = parts.next().unwrap_or("unknown").trim().to_owned();
    let index = parts
        .next()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0);
    Some((PathBuf::from(file), family, index))
}

fn read_font(path: &Path, index: u32) -> Option<FontData> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    let mut data = FontData::from_owned(bytes);
    data.index = index;
    Some(data)
}
