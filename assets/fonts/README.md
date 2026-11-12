# Bundled fonts

Termyx bundles **Cascadia Mono** and **Kawkab Mono 0.501**, both licensed under
the SIL Open Font License 1.1.

- `CascadiaMono-Regular.ttf` — upstream `CascadiaMono.ttf` from
  <https://github.com/microsoft/cascadia-code/releases>. It is the default
  `font.family`, and Windows does not ship it, so the bundle is what keeps the
  grid identical on every PC.
- `CascadiaMono-OFL.txt` — license shipped with that release.
- `KawkabMono-Regular.ttf` — upstream `kawkab-mono-0.501.zip` from
  <https://github.com/aiaf/kawkab-mono/releases/tag/v0.501>.
- `OFL.txt` — Kawkab's license.

`src/fonts.rs` embeds both fonts with `include_bytes!` and registers them with
`fontdb` **before** loading system fonts, so they are always available. A
system copy of the same family is therefore ignored: the bundled file wins, and
the metrics stay stable across machines.

Kawkab is an opt-in Arabic face: the default is Arial because Kawkab's narrow
metrics render disconnected at 16 px with the current rasterizer. Set
`[font] arabic_family = "Kawkab Mono"` to use it.

Kawkab's advance is narrower than the primary monospace cell (about 5.7 px vs
8.8 px at 16 px). Snapping each shaped cluster to its own cell left side-bearing
gaps and broke cursive joins, so the renderer positions an Arabic run
continuously from the shaper's advances and centers the whole run inside its
slot span. That keeps adjacent letters connected when the face's ink is wide
enough for the font size.

The default fallback chain is Arial, Tahoma, Noto Naskh Arabic, Noto Sans
Arabic, DejaVu Sans, FreeSerif, Segoe UI, Kawkab Mono. A candidate is accepted
only when it:

1. covers Arabic letters,
2. shapes `لا` as a single lam-alef ligature,
3. applies Arabic joining substitutions, and
4. has an advance that fits within the primary cell width.
