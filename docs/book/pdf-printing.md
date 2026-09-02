# PDF export and printing

`omacell convert book.xlsx book.pdf` uses Omacell's deterministic page-layout
renderer. `file.print` can write a PDF path or send the same rendered document
to a selected system printer. Ctrl+P opens a keyboard-accessible printer chooser
fed by CUPS, and remembers the last successful printer; no print operation runs
on workbook open.

Page setup preserves paper size, orientation, margins, scaling, print area,
manual breaks, headers/footers, and explicit row/column title bands. A title
band may begin away from row or column one. It repeats on every applicable page
and round-trips through XLSX and OMC.

Arch packages depend on Carlito and Liberation fonts. Fontconfig resolves
Calibri and Arial to their metric-compatible installed faces, and PDF export
embeds the resolved face. If no usable face can be resolved, export uses a
Standard-14 Helvetica fallback and emits a warning. Aptos has no free
metric-compatible clone and is therefore a documented layout difference.

The preview and printed document share pagination. Always inspect a preview
when exact page count matters, particularly for workbooks produced with fonts
not installed on the current machine.
