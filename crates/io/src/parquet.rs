//! Parquet/Arrow read (F-9.5). Write is out of scope.

use std::fs::File;
use std::path::Path;

use arrow::array::{
    Array, BooleanArray, Float32Array, Float64Array, Int32Array, Int64Array, StringArray,
    UInt64Array,
};
use omacell_core::error::CoreError;
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::workbook::Workbook;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use crate::error;
use crate::xlsx::peer_lock_blocks;

/// Open a Parquet file as a table (header row + values).
pub fn open(path: &Path) -> Result<Workbook, CoreError> {
    peer_lock_blocks(path)?;
    let file = File::open(path).map_err(|e| error::parquet_format(e.to_string()))?;
    open_file(file)
}

/// Open Parquet bytes (written to a private temp file so the Arrow reader can seek).
pub fn open_bytes(bytes: &[u8]) -> Result<Workbook, CoreError> {
    let dir = std::env::temp_dir().join(format!(
        "omacell-parquet-{}-{}",
        std::process::id(),
        bytes.len()
    ));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("in.parquet");
    std::fs::write(&path, bytes).map_err(|e| error::parquet_format(e.to_string()))?;
    let result = open(&path);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);
    result
}

fn open_file(file: File) -> Result<Workbook, CoreError> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| error::parquet_format(e.to_string()))?;
    let schema = builder.schema().clone();
    if schema.fields().len() > usize::from(MAX_COLS) {
        return Err(error::xlsx_limit("Parquet has more columns than the grid"));
    }
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    let undo = wb.undo_log().is_enabled();
    wb.undo_log_mut().set_enabled(false);
    for (c, field) in schema.fields().iter().enumerate() {
        wb.set_text(sheet, 0, c as u16, field.name())?;
    }
    let reader = builder
        .build()
        .map_err(|e| error::parquet_format(e.to_string()))?;
    let mut row = 1u32;
    for batch in reader {
        let batch = batch.map_err(|e| error::parquet_format(e.to_string()))?;
        for r in 0..batch.num_rows() {
            if row >= MAX_ROWS {
                wb.undo_log_mut().set_enabled(undo);
                return Err(error::xlsx_limit("Parquet exceeds the row grid"));
            }
            for c in 0..batch.num_columns() {
                write_arrow_cell(&mut wb, sheet, row, c as u16, batch.column(c).as_ref(), r)?;
            }
            row += 1;
        }
    }
    wb.undo_log_mut().set_enabled(undo);
    Ok(wb)
}

fn write_arrow_cell(
    wb: &mut Workbook,
    sheet: omacell_core::addr::SheetId,
    row: u32,
    col: u16,
    array: &dyn Array,
    idx: usize,
) -> Result<(), CoreError> {
    if array.is_null(idx) {
        return Ok(());
    }
    if let Some(a) = array.as_any().downcast_ref::<Float64Array>() {
        return wb.set_number(sheet, row, col, a.value(idx)).map(|_| ());
    }
    if let Some(a) = array.as_any().downcast_ref::<Float32Array>() {
        return wb
            .set_number(sheet, row, col, f64::from(a.value(idx)))
            .map(|_| ());
    }
    if let Some(a) = array.as_any().downcast_ref::<Int64Array>() {
        return wb
            .set_number(sheet, row, col, a.value(idx) as f64)
            .map(|_| ());
    }
    if let Some(a) = array.as_any().downcast_ref::<Int32Array>() {
        return wb
            .set_number(sheet, row, col, f64::from(a.value(idx)))
            .map(|_| ());
    }
    if let Some(a) = array.as_any().downcast_ref::<UInt64Array>() {
        return wb
            .set_number(sheet, row, col, a.value(idx) as f64)
            .map(|_| ());
    }
    if let Some(a) = array.as_any().downcast_ref::<BooleanArray>() {
        let t = if a.value(idx) { "TRUE" } else { "FALSE" };
        wb.set_cell_contents(sheet, row, col, t)?;
        return Ok(());
    }
    if let Some(a) = array.as_any().downcast_ref::<StringArray>() {
        wb.set_text(sheet, row, col, a.value(idx))?;
        return Ok(());
    }
    wb.set_text(sheet, row, col, &format!("{array:?}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Int64Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use parquet::arrow::arrow_writer::ArrowWriter;
    use std::sync::Arc;

    #[test]
    fn reads_utf8_and_int_columns() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", DataType::Utf8, false),
            Field::new("n", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec!["Ada"])),
                Arc::new(Int64Array::from(vec![7i64])),
            ],
        )
        .unwrap();
        let mut buf = Vec::new();
        let mut writer = ArrowWriter::try_new(&mut buf, schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
        let wb = open_bytes(&buf).unwrap();
        let sheet = wb.active_sheet();
        let name = wb.get(sheet, 1, 0).unwrap().unwrap();
        match name.value {
            omacell_core::value::Value::Text(id) => {
                assert_eq!(wb.intern().strings.get(id).unwrap(), "Ada");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            wb.get(sheet, 1, 1).unwrap().unwrap().value,
            omacell_core::value::Value::Number(7.0)
        );
    }
}
