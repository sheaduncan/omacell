//! WP-S1 measurement harness for IronCalc 0.8.3.
//!
//! Generates in-memory workbooks, times full and "incremental" recalc, and
//! probes dynamic arrays, LET/LAMBDA, whole-column SUM, and a tiny xlsx
//! round-trip. Not part of the product workspace.
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use ironcalc::base::cell::CellValue;
use ironcalc::base::types::Color;
use ironcalc::base::{Model, UserModel};
use ironcalc::export::save_to_xlsx;
use ironcalc::import::load_from_xlsx;

const N_100K: i32 = 100_000;
const N_1M: i32 = 1_000_000;

fn main() -> Result<()> {
    println!("omacell WP-S1 IronCalc spike");
    println!("ironcalc 0.8.3  host={}", host_line());
    println!();

    probe_formulas()?;
    probe_async_surface();
    probe_wp01_shape();
    probe_xlsx()?;
    measure_recalc(N_100K, "100k")?;
    measure_sum_column(N_100K)?;
    report_binary_size()?;
    // 1M last so an OOM or long run still leaves 100k numbers in the log.
    // Skip with `--skip-1m` when iterating.
    if std::env::args().any(|a| a == "--skip-1m") {
        println!("== 1M skipped (--skip-1m)");
    } else {
        match measure_recalc(N_1M, "1M") {
            Ok(()) => {}
            Err(e) => println!("== 1M skipped/failed: {e}"),
        }
    }
    Ok(())
}

fn host_line() -> String {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let rss = rss_kb()
        .map(|k| format!("{k} kB RSS"))
        .unwrap_or_else(|| "n/a".into());
    format!("linux cpus={cpus} start_rss={rss}")
}

fn rss_kb() -> Option<u64> {
    let text = fs::read_to_string("/proc/self/status").ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

fn fmt_dur(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1_000.0;
    if ms < 1.0 {
        format!("{:.3} ms", ms)
    } else if ms < 10_000.0 {
        format!("{:.1} ms", ms)
    } else {
        format!("{:.2} s", d.as_secs_f64())
    }
}

fn cell_num(model: &Model<'_>, sheet: u32, row: i32, col: i32) -> Result<f64> {
    match model.get_cell_value_by_index(sheet, row, col) {
        Ok(CellValue::Number(n)) => Ok(n),
        other => Err(anyhow!("expected number at r{row}c{col}, got {other:?}")),
    }
}

fn cell_fmt(model: &Model<'_>, sheet: u32, row: i32, col: i32) -> String {
    model
        .get_formatted_cell_value(sheet, row, col)
        .unwrap_or_else(|e| format!("<err {e}>"))
}

fn probe_formulas() -> Result<()> {
    println!("== formula / array / LET / LAMBDA probes ==");
    let mut model = Model::new_empty("probe", "en", "UTC", "en").map_err(|e| anyhow!(e))?;

    // Independent inputs for spill / LET / LAMBDA.
    model
        .update_cell_with_number(0, 1, 1, 10.0)
        .map_err(|e| anyhow!(e))?;
    model
        .update_cell_with_number(0, 2, 1, 20.0)
        .map_err(|e| anyhow!(e))?;
    model
        .update_cell_with_number(0, 3, 1, 30.0)
        .map_err(|e| anyhow!(e))?;

    let cases: &[(&str, i32, i32, &str)] = &[
        ("LET", 1, 2, "=LET(x,2,x+40)"),
        ("LAMBDA invoke", 2, 2, "=LAMBDA(x,x+1)(41)"),
        ("LET+LAMBDA", 3, 2, "=LET(inc,LAMBDA(n,n+1),inc(41))"),
        ("SEQUENCE spill", 1, 3, "=SEQUENCE(3)"),
        ("spill read", 2, 3, ""), // filled by SEQUENCE
        ("UNIQUE", 1, 4, "=UNIQUE({1;1;2})"),
        ("SUM whole col", 1, 5, "=SUM(A:A)"),
        ("SUM explicit", 2, 5, "=SUM(A1:A3)"),
        ("MAP LAMBDA", 1, 6, "=MAP(A1:A3,LAMBDA(x,x*2))"),
        ("blocked spill", 10, 8, "=SEQUENCE(3)"),
    ];

    // Occupied cells under the blocked SEQUENCE so we can see #SPILL!.
    // Keep this off column A so SUM(A:A) is not polluted by the error.
    model
        .update_cell_with_number(0, 11, 8, 1.0)
        .map_err(|e| anyhow!(e))?;

    for (label, row, col, formula) in cases {
        if formula.is_empty() {
            continue;
        }
        match model.update_cell_with_formula(0, *row, *col, (*formula).to_string()) {
            Ok(()) => {}
            Err(e) => println!("  {label}: set failed: {e}"),
        }
    }
    model.evaluate();

    for (label, row, col, formula) in cases {
        let shown = cell_fmt(&model, 0, *row, *col);
        let raw = model.get_cell_value_by_index(0, *row, *col);
        let formula_txt = model
            .get_cell_formula(0, *row, *col)
            .unwrap_or(None)
            .unwrap_or_default();
        println!("  {label:16} {formula:40} => fmt={shown:?} raw={raw:?} formula={formula_txt:?}");
    }

    // Named-lambda in a defined name, if the API exists.
    match model.new_defined_name("IncOne", None, "=LAMBDA(x,x+1)") {
        Ok(()) => {
            model
                .update_cell_with_formula(0, 1, 7, "=IncOne(41)".to_string())
                .ok();
            model.evaluate();
            println!(
                "  named LAMBDA     =IncOne(41)                              => fmt={:?}",
                cell_fmt(&model, 0, 1, 7)
            );
        }
        Err(e) => println!("  named LAMBDA: new_defined_name failed: {e}"),
    }

    println!();
    Ok(())
}

fn probe_async_surface() {
    println!("== async-node surface ==");
    println!("  public Model/UserModel API has no Pending/Ready/Failed cell state,");
    println!("  no AsyncNodeProvider-shaped hook, and evaluate() is synchronous.");
    println!("  UserModel evaluates on every user action (bindings README).");
    println!("  Issue ironcalc/IronCalc#849 (open, v1.0): full-sheet invalidate, no DAG.");
    println!();
}

fn probe_wp01_shape() {
    println!("== WP-01 / §11.3 shape ==");
    println!("  CellValue: None | String(String) | Number(f64) | Boolean(bool)");
    println!("  — no Error, no Array handle; errors only via formatted text / CellType.");
    println!("  Addressing: sheet u32, row i32, column i32 (not CellRef {{row:u32, col:u16}}).");
    println!("  Storage (source): HashMap<row, HashMap<col, Cell>>, not 256×256 blocks.");
    println!("  support: HashMap<CellReferenceIndex, Vec<CellOrRange>> is pub(crate);");
    println!("  CellOrRange::Range exists, so whole-column refs can be stored as one edge,");
    println!("  but evaluate() still walks every formula cell (no incremental use).");
    println!("  No rayon; single-threaded evaluate().");
    println!("  UserModel has undo; not Changeset {{forward, inverse, origin, status}}.");
    println!();
}

fn probe_xlsx() -> Result<()> {
    println!("== xlsx L1/L2 probe ==");
    let dir = temp_dir()?;
    let path = dir.join("l1l2.xlsx");

    let mut model = Model::new_empty("l1l2.xlsx", "en", "UTC", "en").map_err(|e| anyhow!(e))?;
    model
        .set_user_input(0, 1, 1, "42".to_string())
        .map_err(|e| anyhow!(e))?;
    model
        .set_user_input(0, 2, 1, "=A1*2".to_string())
        .map_err(|e| anyhow!(e))?;
    model
        .set_user_input(0, 1, 2, "100$".to_string())
        .map_err(|e| anyhow!(e))?;

    // L2-ish: fill color + bold on A1.
    let mut style = model.get_style_for_cell(0, 1, 1).map_err(|e| anyhow!(e))?;
    style.fill.color = Color::Rgb("#FF9011".to_string());
    style.font.b = true;
    model
        .set_cell_style(0, 1, 1, &style)
        .map_err(|e| anyhow!(e))?;

    model.evaluate();
    save_to_xlsx(&model, path.to_str().context("xlsx path utf-8")?)
        .map_err(|e| anyhow!("save_to_xlsx: {e}"))?;

    let mut loaded = load_from_xlsx(path.to_str().context("xlsx path utf-8")?, "en", "UTC", "en")
        .map_err(|e| anyhow!("load_from_xlsx: {e}"))?;
    loaded.evaluate();

    let a1 = cell_num(&loaded, 0, 1, 1)?;
    let a2 = cell_num(&loaded, 0, 2, 1)?;
    let b1_fmt = cell_fmt(&loaded, 0, 1, 2);
    let formula = loaded
        .get_cell_formula(0, 2, 1)
        .map_err(|e| anyhow!(e))?
        .unwrap_or_default();
    let style_back = loaded.get_style_for_cell(0, 1, 1).map_err(|e| anyhow!(e))?;
    println!("  L1 values: A1={a1} A2={a2} (expect 42, 84) formula={formula:?} B1_fmt={b1_fmt:?}");
    println!(
        "  L2 style: bold={} fill={:?}",
        style_back.font.b, style_back.fill.color
    );

    // Merge / names / comments / tables / CF / freeze — record API presence.
    println!(
        "  defined names after load: {:?}",
        loaded.get_defined_name_list()
    );
    println!("  L2 comments: no public comment API in 0.8.3 (upstream #295, v2.0).");
    println!("  L2 charts/pivots: roadmap v2.0, not in engine.");
    println!("  L3 unknown parts: import rebuilds a Model; no part-preserve API.");

    let _ = fs::remove_dir_all(&dir);
    println!();
    Ok(())
}

fn temp_dir() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("omacell-wp-s1");
    if dir.exists() {
        fs::remove_dir_all(&dir).ok();
    }
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Column A: numbers 1..=n. Column B: `=An*2`. Edit A1, re-evaluate.
fn measure_recalc(n: i32, label: &str) -> Result<()> {
    println!("== recalc {label} ({n} formulas, B=A*2 independent) ==");
    let rss0 = rss_kb();
    let t0 = Instant::now();
    let mut model = Model::new_empty("recalc", "en", "UTC", "en").map_err(|e| anyhow!(e))?;

    for row in 1..=n {
        model
            .update_cell_with_number(0, row, 1, f64::from(row))
            .map_err(|e| anyhow!("set A{row}: {e}"))?;
        model
            .update_cell_with_formula(0, row, 2, format!("=A{row}*2"))
            .map_err(|e| anyhow!("set B{row}: {e}"))?;
        if row % 200_000 == 0 {
            println!("  built {row} rows in {}", fmt_dur(t0.elapsed()));
        }
    }
    let build = t0.elapsed();
    let rss_built = rss_kb();
    println!(
        "  build {n} inputs + {n} formulas: {}  rss={:?}",
        fmt_dur(build),
        rss_built
    );

    let t1 = Instant::now();
    model.evaluate();
    let full1 = t1.elapsed();
    let b1 = cell_num(&model, 0, 1, 2)?;
    let blast = cell_num(&model, 0, n, 2)?;
    println!(
        "  full evaluate #1: {}  B1={b1} B{n}={blast} (expect 2, {})",
        fmt_dur(full1),
        f64::from(n) * 2.0
    );
    if (b1 - 2.0).abs() > 0.0 {
        bail!("B1 after full eval expected 2, got {b1}");
    }

    // Edit one input that only B1 depends on.
    model
        .update_cell_with_number(0, 1, 1, 100.0)
        .map_err(|e| anyhow!(e))?;
    let t2 = Instant::now();
    model.evaluate();
    let incr = t2.elapsed();
    let b1_after = cell_num(&model, 0, 1, 2)?;
    let blast_after = cell_num(&model, 0, n, 2)?;
    println!(
        "  evaluate after editing A1 (100): {}  B1={b1_after} B{n}={blast_after}",
        fmt_dur(incr)
    );
    if (b1_after - 200.0).abs() > 0.0 {
        bail!("B1 after edit expected 200, got {b1_after}");
    }

    let t3 = Instant::now();
    model.evaluate();
    let full2 = t3.elapsed();
    println!(
        "  full evaluate #2 (no edits): {}  ratio incr/full1={:.2}",
        fmt_dur(full2),
        incr.as_secs_f64() / full1.as_secs_f64().max(1e-9)
    );
    println!(
        "  rss start={rss0:?} built={rss_built:?} end={:?}",
        rss_kb()
    );
    println!(
        "  §12.1 gates: incremental 100k < 50 ms; full 1M < 5 s / 8 threads (engine is 1 thread)"
    );
    println!();
    Ok(())
}

fn measure_sum_column(n: i32) -> Result<()> {
    println!("== SUM(A:A) vs SUM(A1:An) at {n} numeric cells ==");
    let mut model = Model::new_empty("sumcol", "en", "UTC", "en").map_err(|e| anyhow!(e))?;
    for row in 1..=n {
        model
            .update_cell_with_number(0, row, 1, 1.0)
            .map_err(|e| anyhow!(e))?;
    }
    model
        .update_cell_with_formula(0, 1, 3, "=SUM(A:A)".to_string())
        .map_err(|e| anyhow!(e))?;
    let t0 = Instant::now();
    model.evaluate();
    let whole = t0.elapsed();
    let c1 = cell_num(&model, 0, 1, 3)?;
    println!(
        "  SUM(A:A) evaluate: {}  C1={c1} (expect {n})",
        fmt_dur(whole)
    );

    model
        .update_cell_with_formula(0, 1, 3, format!("=SUM(A1:A{n})"))
        .map_err(|e| anyhow!(e))?;
    let t1 = Instant::now();
    model.evaluate();
    let explicit = t1.elapsed();
    let c1b = cell_num(&model, 0, 1, 3)?;
    println!("  SUM(A1:A{n}) evaluate: {}  C1={c1b}", fmt_dur(explicit));

    // Whole-column formula present while 1M empty rows theoretically exist.
    // We cannot count support-map edges (pub(crate)); times + source are the evidence.
    println!(
        "  graph: CellOrRange::Range is one stored edge; evaluate still visits every formula."
    );
    println!();
    Ok(())
}

fn report_binary_size() -> Result<()> {
    println!("== binary size ==");
    // This running binary, plus the rustc crate metadata if we can find the release artifact.
    let exe = std::env::current_exe()?;
    let meta = fs::metadata(&exe)?;
    println!(
        "  current exe: {}  {} bytes ({:.1} MiB)",
        exe.display(),
        meta.len(),
        meta.len() as f64 / (1024.0 * 1024.0)
    );
    if let Some(release) = find_release_bin(&exe) {
        if let Ok(m) = fs::metadata(&release) {
            println!(
                "  release artifact: {}  {} bytes ({:.1} MiB)",
                release.display(),
                m.len(),
                m.len() as f64 / (1024.0 * 1024.0)
            );
        }
    }
    println!("  product binary is omacell-cli; this is a lower bound for adding ironcalc.");
    println!();
    Ok(())
}

fn find_release_bin(current: &Path) -> Option<PathBuf> {
    let mut p = current.to_path_buf();
    for _ in 0..6 {
        if p.file_name()?.to_str()? == "release" {
            return Some(p.join("omacell-spike-ironcalc"));
        }
        p = p.parent()?.to_path_buf();
    }
    None
}

// UserModel is referenced so the crate's high-level API is part of the compile.
#[allow(dead_code)]
fn _touch_user_model() {
    let _ = std::any::type_name::<UserModel<'_>>();
}
