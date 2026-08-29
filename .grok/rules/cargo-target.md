# Cargo artifacts stay off /tmp

`/tmp` on this host is a **16 GiB tmpfs**. `rust-lld` mmaps the linker output. When tmpfs is full that mmap becomes **SIGBUS**, not a clean `ENOSPC`. Do not put Cargo `target/` on `/tmp`.

## Required

1. **Always** set `CARGO_TARGET_DIR` to a path on `/home` (btrfs, hundreds of GiB free) before `cargo` / `just`:

   ```bash
   export CARGO_TARGET_DIR="${HOME}/.cache/omacell/target"
   mkdir -p "$CARGO_TARGET_DIR"
   ```

   The justfile and `.envrc` already export this. If you invoke `cargo` directly, export it in that command.

2. **Never** clone, `cp -a`, or `git worktree add` this repo onto `/tmp` (no `/tmp/omacell-pr*`, no `/tmp/omacell-build`). Isolated work: `spawn_subagent` `isolation: "worktree"` (Grok places those under `~/.grok/worktrees` on disk) or a git worktree under `$HOME`.

3. **Leave existing `/tmp/omacell-pr*` trees alone** unless the user explicitly asks to delete them.

## If a build is already on tmpfs

Prefer moving `CARGO_TARGET_DIR` onto `/home` rather than fighting tmpfs. Only if you cannot:

- `export CARGO_BUILD_JOBS=2` (or `1`)
- `export RUSTFLAGS='-C link-arg=--no-mmap-output-file'` so LLD uses ordinary writes (those fail with `ENOSPC` instead of SIGBUS)

Do not treat SIGBUS from `rust-lld` as a compiler bug. Check `df -h /tmp` first.

## Documented writes outside the repo

Allowed: `$HOME/.cache/omacell/` (Cargo artifacts) and small test fixtures via `std::env::temp_dir()`. Not allowed: repo checkouts or `target/` trees on `/tmp`.
