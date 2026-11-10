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
    /// Chrome palette. Fixed for now, in Windows Terminal 1.24's dark chrome
    /// family; a light theme is out of scope until the whole theme can switch
    /// at once.
    ///
    /// Measured from WT at 100 %: the band is `#2E2E2E`, deliberately lighter
    /// than the terminal body `#0C0C0C`, because it is window chrome the active
    /// tab is cut out of rather than a continuation of the grid.
    pub tab_bar: Rgb,
    /// The body of the tab the window shows. WT paints this exactly the
    /// terminal background (`#0C0C0C`), so the tab reads as a cut-out onto the
    /// grid instead of a card pasted on the strip. Must equal
    /// [`Theme::background`]'s default; the palette test pins both to 12.
    pub tab_active: Rgb,
    /// The body of an inactive tab under the pointer, one step above
    /// [`Theme::tab_active`] so the target stays visible on the `#2E2E2E` band.
    pub tab_hover: Rgb,
    /// The square behind a close button while the pointer is over its tab.
    /// Lighter than the active tab so the button reads as pressed in, not dug
    /// out; at least 51 keeps it visible on `#0C0C0C`.
    pub tab_close_hover: Rgb,
    /// Ink of the tab's `✕`. WT draws it grey (`#989898` in the capture),
    /// never white: closing a tab is secondary to the title.
    pub tab_close: Rgb,
    /// Fill behind the close control at the strip's right end while it is
    /// hovered; the Windows accent red.
    pub caption_close_hover: Rgb,
    /// The title on the tab the window shows. Its own token rather than the
    /// grid's `foreground`, so a user remapping ANSI grey cannot dim the tab
    /// they are looking at.
    pub tab_title: Rgb,
    /// The title on a tab that is not the window's. One step below
    /// [`Theme::tab_title`] so the active tab still reads first.
    pub tab_title_inactive: Rgb,
    /// The active tab's 1 px outline on top and both sides. WT's `#292929`,
    /// one step below the band, so the edge reads as a seam rather than a
    /// highlight.
    pub tab_border: Rgb,
    /// The `#3D3D3D` divider WT draws at the `+` button's left edge, lifting
    /// it off the `#2E2E2E` band. The divider before `⌄` uses
    /// [`Theme::tab_border`] instead, three units darker and visually
    /// indistinguishable.
    pub tab_separator: Rgb,
    /// Dropdown panel. Fixed for now, in the strip's dark family; the panel
    /// opens under the `#2E2E2E` band, so it sits above it in value or the
    /// menu would read as a hole cut into the strip.
    pub menu_background: Rgb,
    /// The dropdown row under the pointer. One step above the panel, the same
    /// way `tab_hover` sits above `tab_active`.
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
            tab_bar: Rgb::new(46, 46, 46),
            tab_active: Rgb::new(12, 12, 12),
            tab_hover: Rgb::new(31, 31, 31),
            tab_close_hover: Rgb::new(51, 51, 51),
            tab_close: Rgb::new(152, 152, 152),
            caption_close_hover: Rgb::new(196, 43, 28),
            tab_title: Rgb::new(255, 255, 255),
            tab_title_inactive: Rgb::new(204, 204, 204),
            tab_border: Rgb::new(41, 41, 41),
            tab_separator: Rgb::new(61, 61, 61),
            menu_background: Rgb::new(51, 51, 51),
            menu_hover: Rgb::new(61, 61, 61),
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
    fn the_tab_palette_puts_the_tab_below_the_band_it_sits_on() {
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        // Measured from Windows Terminal 1.24 at 100 %: `#2E2E2E` band,
        // `#0C0C0C` active tab, `#292929` outline, grey `✕`.
        assert_eq!(theme.tab_bar, Rgb::new(46, 46, 46));
        assert_eq!(theme.tab_active, Rgb::new(12, 12, 12));
        assert_eq!(theme.tab_hover, Rgb::new(31, 31, 31));
        assert_eq!(theme.tab_close_hover, Rgb::new(51, 51, 51));
        assert_eq!(theme.tab_close, Rgb::new(152, 152, 152));
        assert_eq!(theme.tab_border, Rgb::new(41, 41, 41));
        assert_eq!(theme.tab_separator, Rgb::new(61, 61, 61));
        assert_eq!(theme.caption_close_hover, Rgb::new(196, 43, 28));
        assert_eq!(theme.tab_title, Rgb::new(255, 255, 255));
        assert_eq!(theme.tab_title_inactive, Rgb::new(204, 204, 204));

        assert_eq!(
            theme.tab_active, theme.background,
            "the active tab is the terminal background, so it reads as a cut-out"
        );
        assert!(
            theme.tab_hover.r > theme.tab_active.r,
            "a hovered tab must lift off the active tab's cut-out"
        );
        assert!(
            theme.tab_bar.r > theme.tab_hover.r,
            "the band must stay lighter than the tab on it, as WT paints it"
        );
        assert!(
            theme.tab_close_hover.r >= 51,
            "the close square must stay clearly visible on the #0C0C0C tab"
        );
        assert!(
            theme.tab_close_hover.r > theme.tab_bar.r,
            "the close square must lift off the band as well"
        );
        assert!(
            theme.tab_border.r > theme.tab_active.r && theme.tab_border.r < theme.tab_bar.r,
            "the outline is a seam between the tab and the band"
        );
    }

    #[test]
    fn the_menu_palette_lifts_the_panel_and_its_hovered_row() {
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        assert_eq!(theme.menu_background, Rgb::new(51, 51, 51));
        assert_eq!(theme.menu_hover, Rgb::new(61, 61, 61));
        assert!(
            theme.menu_background.r > theme.tab_bar.r,
            "the panel opens under the #2E2E2E band, so it must sit above it"
        );
        assert!(
            theme.menu_hover.r > theme.menu_background.r,
            "a hovered row must stand out from the panel"
        );
    }
}
