# Legacy `.xls` corpus

These committed Excel 97–2003 BIFF workbooks mirror selected files in
`tests/corpus/xlsx/` and exercise values, dates, formulas, merges, defined
names, and sheet visibility.

Tests read these files directly with Omacell's native reader. They must not
generate them at test time or require LibreOffice, Excel, or another external
converter.
