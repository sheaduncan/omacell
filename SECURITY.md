# Security policy

Please **do not** open a public GitHub issue for a vulnerability.

## How to report

Use GitHub's private vulnerability reporting on this repository:

**Security → Report a vulnerability**  
https://github.com/sheaduncan/omacell/security/advisories/new

Include a short description, impact, and steps to reproduce. I will acknowledge the report and follow up there.

## Scope

In scope: the engine, parsers (formula, number format, CSV, later zip/XML/IPC), command bus, and file I/O.

Out of scope: Excel-compatible sheet/workbook "passwords". Those exist for interchange only and are not a confidentiality control.

## Supported versions

This project is pre-1.0 (`0.0.0`). Reports against the default branch (`main`) are the ones that matter.
