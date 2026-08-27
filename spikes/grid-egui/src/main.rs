//! Throwaway WP-S2 spike: eframe/egui on wgpu, 1,048,576 × 50 virtualized grid.

mod cache;
mod fonts;
mod grid;
mod theme;

use std::time::{Duration, Instant};

use eframe::egui::{self, Key, ViewportBuilder};
use eframe::{NativeOptions, Renderer};

use cache::ShapeCache;
use grid::{GridState, N_COLS, N_ROWS, ROW_H};
use theme::Palette;

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let started = Instant::now();

    let native = NativeOptions {
        viewport: ViewportBuilder::default()
            .with_app_id("omacell-grid-egui-spike")
            .with_title("WP-S2 grid-egui spike")
            .with_inner_size([1280.0, 800.0]),
        renderer: Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "WP-S2 grid-egui spike",
        native,
        Box::new(move |cc| Ok(Box::new(SpikeApp::new(cc, args, started)))),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))
}

struct Args {
    measure: bool,
    frames: u32,
}

impl Args {
    fn parse() -> Self {
        let mut measure = false;
        let mut frames = 240u32;
        let mut it = std::env::args().skip(1);
        while let Some(a) = it.next() {
            match a.as_str() {
                "--measure" => measure = true,
                "--frames" => {
                    frames = it.next().and_then(|s| s.parse().ok()).unwrap_or(frames);
                }
                "-h" | "--help" => {
                    eprintln!(
                        "grid-egui [--measure] [--frames N]\n  T theme  click cell  type IME  Q quit"
                    );
                    std::process::exit(0);
                }
                other => {
                    eprintln!("unknown arg: {other}");
                    std::process::exit(2);
                }
            }
        }
        Self { measure, frames }
    }
}

struct SpikeApp {
    args: Args,
    started: Instant,
    first_frame_at: Option<Duration>,
    adapter: String,
    fonts: fonts::LoadedFonts,
    theme_dark: bool,
    palette: Palette,
    cache: ShapeCache,
    grid: GridState,
    ime: String,
    ime_focused: bool,
    last_update: Instant,
    frame_n: u32,
    dts: Vec<f32>,
    cpu_ms: Vec<f32>,
    theme_swap_started: Option<Instant>,
    theme_swap_ms: Option<f32>,
    jumped: bool,
    reported: bool,
}

impl SpikeApp {
    fn new(cc: &eframe::CreationContext<'_>, args: Args, started: Instant) -> Self {
        let fonts = fonts::install(&cc.egui_ctx);
        Palette::DARK.apply(&cc.egui_ctx);

        let adapter = cc
            .wgpu_render_state
            .as_ref()
            .map(|rs| {
                let i = rs.adapter.get_info();
                format!(
                    "{} backend={:?} type={:?} driver={}",
                    i.name, i.backend, i.device_type, i.driver
                )
            })
            .unwrap_or_else(|| "(no wgpu render state)".into());

        Self {
            args,
            started,
            first_frame_at: None,
            adapter,
            fonts,
            theme_dark: true,
            palette: Palette::DARK,
            cache: ShapeCache::new(),
            grid: GridState::default(),
            ime: String::new(),
            ime_focused: false,
            last_update: Instant::now(),
            frame_n: 0,
            dts: Vec::new(),
            cpu_ms: Vec::new(),
            theme_swap_started: None,
            theme_swap_ms: None,
            jumped: false,
            reported: false,
        }
    }

    fn swap_theme(&mut self, ctx: &egui::Context) {
        self.theme_dark = !self.theme_dark;
        self.palette = if self.theme_dark {
            Palette::DARK
        } else {
            Palette::LIGHT
        };
        self.palette.apply(ctx);
        self.cache.clear();
        self.theme_swap_started = Some(Instant::now());
    }

    fn finish_measure(&mut self, ctx: &egui::Context) {
        if self.reported {
            return;
        }
        self.reported = true;
        let rss_kib = rss_kib();
        let startup_ms = self
            .first_frame_at
            .unwrap_or_else(|| self.started.elapsed())
            .as_secs_f64()
            * 1000.0;
        let stats = summarize(&self.dts);
        let cpu = summarize(&self.cpu_ms);
        let swap = self.theme_swap_ms.unwrap_or(-1.0);
        println!("{{");
        println!("  \"adapter\": {},", json_str(&self.adapter));
        println!("  \"pixels_per_point\": {},", ctx.pixels_per_point());
        println!("  \"startup_to_first_frame_ms\": {startup_ms:.3},");
        println!("  \"scroll_frames\": {},", self.dts.len());
        println!("  \"frame_ms_median\": {:.3},", stats.median);
        println!("  \"frame_ms_p95\": {:.3},", stats.p95);
        println!("  \"frame_ms_max\": {:.3},", stats.max);
        println!("  \"frame_ms_mean\": {:.3},", stats.mean);
        println!("  \"cpu_update_ms_median\": {:.3},", cpu.median);
        println!("  \"cpu_update_ms_p95\": {:.3},", cpu.p95);
        println!("  \"theme_swap_ms\": {swap:.3},");
        println!("  \"rss_kib\": {rss_kib},");
        println!("  \"rss_mib\": {:.1},", rss_kib as f64 / 1024.0);
        println!("  \"shaping_cache_entries\": {},", self.cache.len());
        println!("  \"rows\": {},", N_ROWS);
        println!("  \"cols\": {},", N_COLS);
        println!(
            "  \"monospace_family\": {},",
            json_str(&self.fonts.monospace_family)
        );
        println!(
            "  \"monospace_path\": {},",
            json_str(&self.fonts.monospace_path.display().to_string())
        );
        println!(
            "  \"cjk_family\": {},",
            json_str(self.fonts.cjk_family.as_deref().unwrap_or(""))
        );
        println!(
            "  \"focused_cell\": {}",
            json_str(&grid::a1(self.grid.focused.0, self.grid.focused.1))
        );
        println!("}}");
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for SpikeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let cpu_t0 = Instant::now();
        let dt = self.last_update.elapsed().as_secs_f32() * 1000.0;
        self.last_update = Instant::now();
        self.frame_n += 1;
        if self.first_frame_at.is_none() {
            self.first_frame_at = Some(self.started.elapsed());
        }
        // Crisp 1-device-px gridlines (§11.4). Text stays hinted via the font.
        ctx.tessellation_options_mut(|t| {
            t.feathering = false;
        });

        if let Some(t0) = self.theme_swap_started.take() {
            self.theme_swap_ms = Some(t0.elapsed().as_secs_f32() * 1000.0);
        }

        if !self.ime_focused && ctx.input(|i| i.key_pressed(Key::T)) {
            self.swap_theme(&ctx);
        }
        if ctx.input(|i| i.key_pressed(Key::Q) || i.key_pressed(Key::Escape)) && !self.ime_focused {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if !self.ime_focused {
            let (mut r, mut c) = self.grid.focused;
            if ctx.input(|i| i.key_pressed(Key::ArrowDown)) {
                r = (r + 1).min(N_ROWS - 1);
            }
            if ctx.input(|i| i.key_pressed(Key::ArrowUp)) {
                r = r.saturating_sub(1);
            }
            if ctx.input(|i| i.key_pressed(Key::ArrowRight)) {
                c = (c + 1).min(N_COLS - 1);
            }
            if ctx.input(|i| i.key_pressed(Key::ArrowLeft)) {
                c = c.saturating_sub(1);
            }
            if (r, c) != self.grid.focused {
                self.grid.focused = (r, c);
            }
        }

        if self.args.measure {
            measure_tick(self);
            if self.theme_swap_started.is_some() {
                self.palette.apply(&ctx);
            }
            ctx.request_repaint();
        }

        egui::Panel::top("chrome").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.strong("WP-S2 grid-egui");
                ui.separator();
                ui.label(format!(
                    "{}×{}  theme={}  ppp={:.2}  fps={:.0}  cache={}",
                    N_ROWS,
                    N_COLS,
                    self.palette.name,
                    ui.ctx().pixels_per_point(),
                    ui.input(|i| 1.0 / i.stable_dt.max(1e-4)),
                    self.cache.len(),
                ));
            });
            ui.horizontal(|ui| {
                ui.label("IME:");
                let edit = egui::TextEdit::singleline(&mut self.ime)
                    .hint_text("type CJK here")
                    .desired_width(320.0);
                let resp = ui.add(edit);
                self.ime_focused = resp.has_focus();
                ui.separator();
                ui.label(format!(
                    "focus {}  font {}  adapter {}",
                    grid::a1(self.grid.focused.0, self.grid.focused.1),
                    self.fonts.monospace_family,
                    short_adapter(&self.adapter),
                ));
            });
            ui.colored_label(
                self.palette.muted,
                "T theme · click cell · arrows · Q quit · --measure",
            );
            ui.add_space(2.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            let stats = grid::show(
                ui,
                &mut self.grid,
                &self.palette,
                &mut self.cache,
                !self.ime_focused,
            );
            ui.ctx().debug_painter().text(
                ui.clip_rect().left_bottom() + egui::vec2(8.0, -18.0),
                egui::Align2::LEFT_BOTTOM,
                format!(
                    "viewport {}×{} cells  cache {}",
                    stats.vis_rows, stats.vis_cols, stats.shaped
                ),
                egui::FontId::monospace(11.0),
                self.palette.muted,
            );
        });

        let cpu = cpu_t0.elapsed().as_secs_f32() * 1000.0;
        if self.args.measure && self.frame_n > 30 && self.dts.len() < self.args.frames as usize {
            self.dts.push(dt);
            self.cpu_ms.push(cpu);
        }

        if self.args.measure
            && self.jumped
            && self.theme_swap_ms.is_some()
            && self.dts.len() >= self.args.frames as usize
        {
            self.finish_measure(&ctx);
        }
    }
}

fn measure_tick(app: &mut SpikeApp) {
    // Auto-scroll one-plus rows per frame so virtualization is doing work.
    if app.frame_n > 10 && app.dts.len() < app.args.frames as usize {
        app.grid.scroll.y += ROW_H * 2.0;
        app.grid.focused.0 = (app.grid.scroll.y / ROW_H) as u32;
        app.grid.focused.0 = app.grid.focused.0.min(N_ROWS - 1);
    }
    // Seek near the bottom of the million-row sheet, then swap the theme.
    if !app.jumped && app.dts.len() >= app.args.frames as usize {
        grid::jump_to(&mut app.grid, N_ROWS - 40, 0);
        app.jumped = true;
    }
    if app.jumped
        && app.theme_swap_ms.is_none()
        && app.theme_swap_started.is_none()
        && app.frame_n > 40
    {
        // Swap on the next iteration via a flag: apply here.
        let ctx_swap = true;
        if ctx_swap {
            app.theme_dark = !app.theme_dark;
            app.palette = if app.theme_dark {
                Palette::DARK
            } else {
                Palette::LIGHT
            };
            app.cache.clear();
            app.theme_swap_started = Some(Instant::now());
        }
    }
}

fn rss_kib() -> u64 {
    let Ok(s) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest
                .split_whitespace()
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
        }
    }
    0
}

struct Num {
    median: f32,
    p95: f32,
    max: f32,
    mean: f32,
}

fn summarize(xs: &[f32]) -> Num {
    if xs.is_empty() {
        return Num {
            median: 0.0,
            p95: 0.0,
            max: 0.0,
            mean: 0.0,
        };
    }
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let median = v[v.len() / 2];
    let p95 = v[((v.len() as f32) * 0.95) as usize];
    let max = *v.last().unwrap();
    let mean = v.iter().sum::<f32>() / v.len() as f32;
    Num {
        median,
        p95,
        max,
        mean,
    }
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn short_adapter(s: &str) -> String {
    s.split(" backend=").next().unwrap_or(s).to_owned()
}
