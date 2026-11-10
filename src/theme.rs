//! Color palette and appearance settings.

use crate::config::ThemeConfig;
use tiny_skia::Color as SkColor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn from_hex(value: &str) -> Option<Self> {
        let hex = value.strip_prefix('#').unwrap_or(value);
        if hex.len() != 6 || !hex.is_ascii() {
            return None;
        }
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        Some(Self { r, g, b })
    }

    pub fn to_tiny(self) -> SkColor {
        SkColor::from_rgba8(self.r, self.g, self.b, 255)
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
    pub normal: [Rgb; 8],
    pub bright: [Rgb; 8],
    /// Chrome palette. Fixed for now: the strip matches the terminal
    /// background so the band reads as window chrome rather than content.
    pub tab_bar: Rgb,
    pub tab_active: Rgb,
    pub tab_hover: Rgb,
    /// The square behind a close button while the pointer is over its tab.
    /// Lighter than the active tab so the button reads as pressed in, not dug
    /// out.
    pub tab_close_hover: Rgb,
    pub tab_border: Rgb,
    /// Dropdown panel. Fixed for now, in the strip's dark family; a light
    /// theme is out of scope until the whole theme can switch at once.
    pub menu_background: Rgb,
    /// The dropdown row under the pointer. One step above the panel, the same
    /// way `tab_hover` sits above `tab_bar`.
    pub menu_hover: Rgb,
}

impl Theme {
    pub fn from_config(config: &ThemeConfig) -> Self {
        let parse = |value: &str, fallback: Rgb| match Rgb::from_hex(value) {
            Some(rgb) => rgb,
            None => {
                tracing::warn!("invalid theme color {value:?}, falling back to {fallback:?}");
                fallback
            }
        };
        Self {
            background: parse(&config.background, Rgb::new(12, 12, 12)),
            foreground: parse(&config.foreground, Rgb::new(204, 204, 204)),
            cursor: parse(&config.cursor, Rgb::new(255, 255, 255)),
            selection: parse(&config.selection, Rgb::new(38, 79, 120)),
            normal: [
                parse(&config.black, Rgb::new(12, 12, 12)),
                parse(&config.red, Rgb::new(197, 15, 31)),
                parse(&config.green, Rgb::new(19, 161, 14)),
                parse(&config.yellow, Rgb::new(193, 156, 0)),
                parse(&config.blue, Rgb::new(0, 55, 218)),
                parse(&config.magenta, Rgb::new(136, 23, 152)),
                parse(&config.cyan, Rgb::new(58, 150, 221)),
                parse(&config.white, Rgb::new(204, 204, 204)),
            ],
            bright: [
                parse(&config.bright_black, Rgb::new(118, 118, 118)),
                parse(&config.bright_red, Rgb::new(231, 72, 86)),
                parse(&config.bright_green, Rgb::new(22, 198, 12)),
                parse(&config.bright_yellow, Rgb::new(249, 241, 165)),
                parse(&config.bright_blue, Rgb::new(59, 120, 255)),
                parse(&config.bright_magenta, Rgb::new(180, 0, 158)),
                parse(&config.bright_cyan, Rgb::new(97, 214, 214)),
                parse(&config.bright_white, Rgb::new(242, 242, 242)),
            ],
            tab_bar: Rgb::new(12, 12, 12),
            tab_active: Rgb::new(31, 31, 31),
            tab_hover: Rgb::new(26, 26, 26),
            tab_close_hover: Rgb::new(58, 58, 58),
            tab_border: Rgb::new(43, 43, 43),
            menu_background: Rgb::new(31, 31, 31),
            menu_hover: Rgb::new(43, 43, 43),
        }
    }

    pub fn indexed(&self, index: u8) -> Rgb {
        match index {
            0..=7 => self.normal[index as usize],
            8..=15 => self.bright[(index - 8) as usize],
            16..=231 => {
                let i = index - 16;
                let component = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
                Rgb::new(component(i / 36), component((i % 36) / 6), component(i % 6))
            }
            _ => {
                let v = 8 + (index - 232) * 10;
                Rgb::new(v, v, v)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colors() {
        assert_eq!(Rgb::from_hex("#0c0c0c"), Some(Rgb::new(12, 12, 12)));
        assert_eq!(Rgb::from_hex("ccCCcc"), Some(Rgb::new(204, 204, 204)));
        assert_eq!(Rgb::from_hex("#fff"), None);
        assert_eq!(Rgb::from_hex("zzzzzz"), None);
    }

    #[test]
    fn indexed_palette_covers_ansi_and_256_colors() {
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        assert_eq!(theme.indexed(1), Rgb::new(197, 15, 31));
        assert_eq!(theme.indexed(9), Rgb::new(231, 72, 86));
        assert_eq!(theme.indexed(16), Rgb::new(0, 0, 0));
        assert_eq!(theme.indexed(231), Rgb::new(255, 255, 255));
        assert_eq!(theme.indexed(232), Rgb::new(8, 8, 8));
        assert_eq!(theme.indexed(255), Rgb::new(238, 238, 238));
    }

    #[test]
    fn the_tab_palette_orders_the_band_the_hover_and_the_close_button() {
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        assert_eq!(theme.tab_bar, Rgb::new(12, 12, 12));
        assert_eq!(theme.tab_hover, Rgb::new(26, 26, 26));
        assert_eq!(theme.tab_active, Rgb::new(31, 31, 31));
        assert_eq!(theme.tab_close_hover, Rgb::new(58, 58, 58));
        assert!(theme.tab_bar.r < theme.tab_hover.r);
        assert!(
            theme.tab_hover.r < theme.tab_active.r,
            "a hovered inactive tab must stay darker than the active tab"
        );
        assert!(
            theme.tab_active.r < theme.tab_close_hover.r,
            "the close button's square must lift off the tab it sits on"
        );
    }

    #[test]
    fn the_menu_palette_lifts_the_panel_and_its_hovered_row() {
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        assert_eq!(theme.menu_background, Rgb::new(31, 31, 31));
        assert_eq!(theme.menu_hover, Rgb::new(43, 43, 43));
        assert!(
            theme.menu_background.r > theme.tab_bar.r,
            "the panel must stand out from the strip it opens over"
        );
        assert!(
            theme.menu_hover.r > theme.menu_background.r,
            "a hovered row must stand out from the panel"
        );
    }
}
