# Parser and resource limits

Omacell treats file, workbook, configuration, IPC, MCP, Lua, theme, and keymap
input as untrusted. It bounds input before allocation and rejects excessive
nested structure. Error codes ending in `.limit` identify intentional resource
limits rather than malformed syntax.

Important public limits include:

| Surface | Limit |
|---|---:|
| IPC frame | configurable 1–16 MiB; hard ceiling 16 MiB |
| IPC JSON nesting | 32 levels |
| IPC clients | 32 |
| MCP arguments/result page | 1 MiB |
| MCP JSON nesting | 32 levels |
| MCP range rows per page | 1,024 |
| Command/Lua resolved range | 100,000 cells |
| User Lua source | 1 MiB |
| Embedded Lua | 32 MiB memory and 10,000,000 VM instructions |
| Lua macro recording | 10,000 steps or 16 MiB |
| Trust store | 1 MiB |
| Legacy `.xls` input / worker output | 256 MiB each |

Legacy BIFF parsing runs in the private `omacell-xls-worker` companion with a
1 GiB address-space ceiling, 10 seconds of CPU time, a 15-second host wall-time
limit, no inherited environment, no core dumps, and a 64 KiB diagnostic cap.
The host validates the OLE DIFAT chain before starting it and accepts only a
bounded XLSX reply that passes Omacell's normal package reader. A parser panic,
allocator abort, loop, or malformed reply therefore becomes an `xls.bridge`
error instead of terminating the GUI, TUI, or CLI. This helper is part of
Omacell; LibreOffice is not invoked or required.

Archive readers also cap entry counts, individual expanded entries, aggregate
expanded bytes, compression ratios, XML nesting/text, shared strings, styles,
formula text, workbook dimensions, and relationships before materializing a
workbook. Exact format-specific ceilings are part of the versioned error and
test corpus rather than configuration knobs; this avoids making a dangerous
file acceptable through a workbook-supplied setting.

For large legitimate data, prefer CSV streaming and query pagination. Raising
`ipc.max_frame_bytes` cannot exceed the protocol ceiling and does not raise MCP,
file parser, or command-range limits.
