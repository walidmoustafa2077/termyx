# Bundled fonts

Termyx prefers a cell-compatible Arabic face. The configured default is
`Kawkab Mono`, which is OFL 1.1.

This directory intentionally contains no font binary yet. Until the asset task
lands, `src/fonts.rs` falls back to an installed Arabic-capable system face
(Arial, Tahoma, Noto Naskh Arabic, Noto Sans Arabic, DejaVu Sans, FreeSerif, or
Segoe UI) and logs a warning naming the requested family. A face is only chosen
when it shapes `لا` as a single lam-alef ligature, so a face that covers Arabic
letters but cannot shape them is skipped.

When the asset is added:

- Place `KawkabMono-Regular.ttf` here.
- Add the OFL 1.1 license as `assets/fonts/KawkabMono-OFL.txt`.
- Load it with `fontdb::Database::load_font_data` before querying system fonts.
- Keep the fallback chain for machines where the file is missing.
