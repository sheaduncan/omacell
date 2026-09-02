# Omarchy integration

The package does not modify Omarchy or user configuration. After installation,
run `omacell setup omarchy` as your user to install the theme template, the
theme-change hook, and links to the shipped agent skill. Existing non-symlink
paths are preserved. Menu rows require the explicit `--menu` option or an
interactive confirmation.

The skill is linked into the generic Agent Skills location and the personal
locations recognized by Claude Code, Codex, OpenCode, Pi/Oh My Pi, Gemini,
Grok, GitHub Copilot, and Crush. Re-running setup is idempotent.

Run `omacell setup omarchy --show-hyprland` to print the current launch-table
snippet, then choose a chord that does not conflict with your own bindings.
`omacell keys check` can compare the classic Omacell keymap with
`~/.config/hypr/bindings.lua`.

```lua
o.bind("SUPER + ALT + X", "Spreadsheet", { launch = "omacell" })
```

The theme hook sends `theme.reload` to every live owned Omacell instance over
the local user-only IPC socket. If Omacell is not running, the hook exits
without changing configuration. The generated theme file remains under the
user's Omarchy configuration and follows subsequent theme changes.

Stable, release-candidate, and edge Omarchy images are release gates. Those
live VM checks complement the software-render smoke lane; they are not replaced
by it.
