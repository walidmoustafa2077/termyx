# Bundled fonts

Termyx bundles **Kawkab Mono 0.501**, licensed under the SIL Open Font
License 1.1. The full license text is `OFL.txt` in this directory.

- `KawkabMono-Regular.ttf` — upstream `kawkab-mono-0.501.zip` from
  <https://github.com/aiaf/kawkab-mono/releases/tag/v0.501>.
- `OFL.txt` — license shipped with that release.

`src/fonts.rs` embeds the font with `include_bytes!` and registers it with
`fontdb` **before** loading system fonts, so it is always available. It is the
default Arabic face.

Kawkab's advance is narrower than the primary monospace cell (about 5.7 px vs
8.8 px at 16 px). Snapping each shaped cluster to its own cell left side-bearing
gaps and broke cursive joins, so the renderer positions an Arabic run
continuously from the shaper's advances and centers the whole run inside its
slot span. That keeps adjacent letters connected.

The fallback chain is Kawkab Mono, Arial, Tahoma, Noto Naskh Arabic, Noto Sans
Arabic, DejaVu Sans, FreeSerif, Segoe UI. A candidate is accepted only when it:

1. covers Arabic letters,
2. shapes `لا` as a single lam-alef ligature,
3. applies Arabic joining substitutions, and
4. has an advance that fits within the primary cell width.
