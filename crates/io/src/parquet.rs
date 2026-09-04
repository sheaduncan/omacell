//! Parquet/Arrow read (F-9.5). Write is out of scope.

use std::fs::File;
use std::path::Path;

use arrow::array::{
    Array, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array,
    StringArray, UInt8Array, UInt16Array, UInt32Array, UInt64Array,
};
use arrow::datatypes::DataType;
use bytes::Bytes;
use omacell_core::error::CoreError;
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::workbook::Workbook;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::reader::ChunkReader;

use crate::error;
use crate::xlsx::peer_lock_blocks;

const MAX_PARQUET_BYTES: u64 = 512 * 1024 * 1024;
const MAX_BATCH_ROWS: usize = 1024;
const MAX_EXACT_INTEGER: u64 = 1u64 << 53;

/// Open a Parquet file as a table (header row + values).
pub fn open(path: &Path) -> Result<Workbook, CoreError> {
    peer_lock_blocks(path)?;
    let len = std::fs::metadata(path)
        .map_err(|e| error::parquet_format(e.to_string()))?
        .len();
    if len > MAX_PARQUET_BYTES {
        return Err(error::xlsx_limit(format!(
            "Parquet file is {len} bytes; maximum is {MAX_PARQUET_BYTES}"
        )));
    }
    let file = File::open(path).map_err(|e| error::parquet_format(e.to_string()))?;
    open_reader(file)
}

/// Open Parquet bytes from memory.
pub fn open_bytes(bytes: &[u8]) -> Result<Workbook, CoreError> {
    if bytes.len() as u64 > MAX_PARQUET_BYTES {
        return Err(error::xlsx_limit(format!(
            "Parquet payload is {} bytes; maximum is {MAX_PARQUET_BYTES}",
            bytes.len()
        )));
    }
    open_reader(Bytes::copy_from_slice(bytes))
}

fn open_reader<T: ChunkReader + 'static>(reader: T) -> Result<Workbook, CoreError> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(reader)
        .map_err(|e| error::parquet_format(e.to_string()))?;
    let schema = builder.schema().clone();
    if schema.fields().len() > usize::from(MAX_COLS) {
        return Err(error::xlsx_limit("Parquet has more columns than the grid"));
    }
    for field in schema.fields() {
        if !supported_type(field.data_type()) {
            return Err(error::parquet_format(format!(
                "unsupported Parquet column {:?} with type {}",
                field.name(),
                field.data_type()
            )));
        }
    }
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    let undo = wb.undo_log().is_enabled();
    wb.undo_log_mut().set_enabled(false);
    for (c, field) in schema.fields().iter().enumerate() {
        wb.set_text(sheet, 0, c as u16, field.name())?;
    }
    let reader = builder
        .with_batch_size(MAX_BATCH_ROWS)
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
        return write_float(wb, sheet, row, col, a.value(idx));
    }
    if let Some(a) = array.as_any().downcast_ref::<Float32Array>() {
        return write_float(wb, sheet, row, col, f64::from(a.value(idx)));
    }
    if let Some(a) = array.as_any().downcast_ref::<Int64Array>() {
        return write_signed(wb, sheet, row, col, a.value(idx));
    }
    if let Some(a) = array.as_any().downcast_ref::<Int32Array>() {
        return write_signed(wb, sheet, row, col, i64::from(a.value(idx)));
    }
    if let Some(a) = array.as_any().downcast_ref::<Int16Array>() {
        return write_signed(wb, sheet, row, col, i64::from(a.value(idx)));
    }
    if let Some(a) = array.as_any().downcast_ref::<Int8Array>() {
        return write_signed(wb, sheet, row, col, i64::from(a.value(idx)));
    }
    if let Some(a) = array.as_any().downcast_ref::<UInt64Array>() {
        return write_unsigned(wb, sheet, row, col, a.value(idx));
    }
    if let Some(a) = array.as_any().downcast_ref::<UInt32Array>() {
        return write_unsigned(wb, sheet, row, col, u64::from(a.value(idx)));
    }
    if let Some(a) = array.as_any().downcast_ref::<UInt16Array>() {
        return write_unsigned(wb, sheet, row, col, u64::from(a.value(idx)));
    }
    if let Some(a) = array.as_any().downcast_ref::<UInt8Array>() {
        return write_unsigned(wb, sheet, row, col, u64::from(a.value(idx)));
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
    Err(error::parquet_format(format!(
        "unsupported Arrow array type {}",
        array.data_type()
    )))
}

fn write_float(
    wb: &mut Workbook,
    sheet: omacell_core::addr::SheetId,
    row: u32,
    col: u16,
    value: f64,
) -> Result<(), CoreError> {
    if value.is_finite() {
        wb.set_number(sheet, row, col, value)?;
    } else {
        let text = if value.is_nan() {
            "NaN"
        } else if value.is_sign_negative() {
            "-Infinity"
        } else {
            "Infinity"
        };
        wb.set_text(sheet, row, col, text)?;
    }
    Ok(())
}

fn supported_type(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Utf8
            | DataType::Boolean
            | DataType::Float32
            | DataType::Float64
            | DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
    )
}

fn write_signed(
    wb: &mut Workbook,
    sheet: omacell_core::addr::SheetId,
    row: u32,
    col: u16,
    value: i64,
) -> Result<(), CoreError> {
    if value.unsigned_abs() <= MAX_EXACT_INTEGER {
        wb.set_number(sheet, row, col, value as f64)?;
    } else {
        wb.set_text(sheet, row, col, &value.to_string())?;
    }
    Ok(())
}

fn write_unsigned(
    wb: &mut Workbook,
    sheet: omacell_core::addr::SheetId,
    row: u32,
    col: u16,
    value: u64,
) -> Result<(), CoreError> {
    if value <= MAX_EXACT_INTEGER {
        wb.set_number(sheet, row, col, value as f64)?;
    } else {
        wb.set_text(sheet, row, col, &value.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Float64Array, Int64Array, StringArray};
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

    #[test]
    fn preserves_large_integers_as_text_and_rejects_unsupported_columns() {
        use arrow::array::{BinaryArray, UInt64Array};

        let schema = Arc::new(Schema::new(vec![Field::new(
            "large",
            DataType::UInt64,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(UInt64Array::from(vec![u64::MAX]))],
        )
        .unwrap();
        let mut buf = Vec::new();
        let mut writer = ArrowWriter::try_new(&mut buf, schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
        let wb = open_bytes(&buf).unwrap();
        let slot = wb.get(wb.active_sheet(), 1, 0).unwrap().unwrap();
        let omacell_core::value::Value::Text(id) = slot.value else {
            panic!("large integer was not preserved as text");
        };
        assert_eq!(wb.intern().strings.get(id), Some("18446744073709551615"));

        let schema = Arc::new(Schema::new(vec![Field::new(
            "binary",
            DataType::Binary,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(BinaryArray::from(vec![b"x".as_slice()]))],
        )
        .unwrap();
        let mut buf = Vec::new();
        let mut writer = ArrowWriter::try_new(&mut buf, schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
        assert_eq!(
            open_bytes(&buf).unwrap_err().code,
            crate::error::codes::PARQUET_FORMAT
        );
    }

    #[test]
    fn preserves_non_finite_floats_as_text() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            DataType::Float64,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Float64Array::from(vec![
                f64::NAN,
                f64::INFINITY,
                f64::NEG_INFINITY,
                1.25,
            ]))],
        )
        .unwrap();
        let mut buf = Vec::new();
        let mut writer = ArrowWriter::try_new(&mut buf, schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();

        let wb = open_bytes(&buf).unwrap();
        let sheet = wb.active_sheet();
        for (row, expected) in [(1, "NaN"), (2, "Infinity"), (3, "-Infinity")] {
            let slot = wb.get(sheet, row, 0).unwrap().unwrap();
            let omacell_core::value::Value::Text(id) = slot.value else {
                panic!("non-finite Parquet value at row {row} was stored as a number");
            };
            assert_eq!(wb.intern().strings.get(id), Some(expected));
        }
        assert_eq!(
            wb.get(sheet, 4, 0).unwrap().unwrap().value,
            omacell_core::value::Value::Number(1.25)
        );
    }
}
