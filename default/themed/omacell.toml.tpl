# Omacell color roles — rendered from the active Omarchy theme's colors.toml.
# Placeholders: {{ key }}, {{ key_strip }}, {{ key_rgb }}, {{ mix a b 20% }}.
mode = "{{ mode }}"

[surfaces]
background        = "{{ background }}"
surface           = "{{ lighter_background }}"     # panels, formula bar
header_background = "{{ dark_background }}"        # row/column headers
popup_background  = "{{ darker_background }}"

[text]
foreground        = "{{ foreground }}"
muted             = "{{ muted }}"                  # empty-cell hints, placeholders
header_foreground = "{{ dark_foreground }}"
bright            = "{{ bright_foreground }}"

[structure]
grid_line    = "{{ mix background foreground 12% }}"
pane_divider = "{{ mix background foreground 35% }}"
frozen_edge  = "{{ accent }}"

[state]
cursor           = "{{ accent }}"
selection        = "{{ selection }}"
selection_border = "{{ accent }}"
active_header    = "{{ accent }}"
hover            = "{{ mix background foreground 6% }}"
stale            = "{{ mix background muted 50% }}"  # hatching for not-yet-recalculated cells

[semantic]
error   = "{{ red }}"
warning = "{{ color3 }}"
success = "{{ color2 }}"
info    = "{{ color4 }}"
link    = "{{ blue }}"

[references]   # cycle used to colorize ranges while editing a formula
colors = ["{{ color4 }}", "{{ color2 }}", "{{ color5 }}", "{{ color3 }}",
          "{{ color6 }}", "{{ color1 }}", "{{ accent }}", "{{ color7 }}"]

[charts]
palette = ["{{ accent }}", "{{ color2 }}", "{{ color3 }}", "{{ color5 }}",
           "{{ color6 }}", "{{ color1 }}", "{{ color4 }}", "{{ color7 }}"]
axis     = "{{ dark_foreground }}"
gridline = "{{ mix background foreground 10% }}"

[conditional]  # defaults for new color scales / data bars
scale_low  = "{{ red }}"
scale_mid  = "{{ color3 }}"
scale_high = "{{ color2 }}"
data_bar   = "{{ accent }}"
