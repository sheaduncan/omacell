//! Workbook file I/O for Omacell (`.xlsx`, CSV, `.omc`, and later formats).
//!
//! Depends on `omacell-core`. Filled in by WP-08, WP-09, WP-10, WP-11, and WP-27
//! (ODS, JSON, Parquet/Arrow, HTML/Markdown, `.xls` bridge).
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

pub mod bridge;
pub mod chart_export;
pub mod csv;
pub mod error;
pub mod html;
pub mod json;
pub mod ods;
pub mod omc;
pub mod parquet;
pub mod pdf;
mod temp;
pub mod xlsx;
