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
    /// Measured from WT at 100 %: the band is `#333333`, deliberately lighter
    /// than the terminal body `#0C0C0C`, because it is window chrome the active
    /// tab is cut out of rather than a continuation of the grid.
    pub tab_bar: Rgb,
    /// The body of the tab the window shows. WT paints this exactly the
    /// terminal background (`#0C0C0C`), so the tab reads as a cut-out onto the
    /// grid instead of a card pasted on the strip. Must equal
    /// [`Theme::background`]'s default; the palette test pins both to 12.
    pub tab_active: Rgb,
    /// The body of an inactive tab is never repainted: WT highlights only the
    /// control under the pointer, not the tab card (measured: hovering the tab
    /// body changes no pixel). This is the fill behind `+`, `⌄` and the
    /// minimize/maximize controls while they are hovered, one step above the
    /// band.
    pub control_hover: Rgb,
    /// The `+`/`⌄` glyph while its control is hovered. WT brightens it from the
    /// resting `#CCCCCC` to `#E3E3E3` rather than to pure white.
    pub control_hover_ink: Rgb,
    /// The 32x24 pill behind the tab's `✕` while the pointer is over the glyph
    /// itself. WT paints it as a faint white scrim, so it lands on `#1A1A1A`
    /// over the tab body: one step lighter than the tab it sits on, and still
    /// darker than the band.
    pub tab_close_hover: Rgb,
    /// Ink of the tab's `✕` when at rest. The close glyph now shares the
    /// title's ink (see [`Theme::ink_on`]) so it stays readable on a colored
    /// tab, so this grey is no longer painted; it is kept as the measured
    /// reference value.
    #[allow(dead_code)]
    pub tab_close: Rgb,
    /// The `✕` ink while the pill behind it is shown.
    pub tab_close_hover_ink: Rgb,
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
    /// it off the `#333333` band. The divider before `⌄` uses
    /// [`Theme::tab_border`] instead, three units darker and visually
    /// indistinguishable.
    pub tab_separator: Rgb,
    /// Dropdown panel, measured `#2C2C2C` on Windows Terminal 1.24. It sits a
    /// step *below* the `#333333` band, unlike a first guess would suggest:
    /// the panel is a recessed surface, not a raised one.
    pub menu_background: Rgb,
    /// The dropdown row under the pointer. One step above the panel, the same
    /// way [`Theme::control_hover`] sits above [`Theme::tab_active`].
    pub menu_hover: Rgb,
    /// Rule between the dropdown's groups, measured `#3D3D3D`.
    pub menu_separator: Rgb,
    /// A row's right-aligned key hint, measured `#CFCFCF`: dimmer than the
    /// label so the eye follows the names first.
    pub menu_shortcut: Rgb,
    /// Ink of a context-menu row that cannot act, measured `#8A8A8A`: dimmer
    /// than the resting label so the name still reads but takes the hint.
    pub menu_disabled: Rgb,
    /// The band behind a `Find` match, measured `#484822`.
    pub find_match: Rgb,
    /// The band behind the `Find` match under the caret, measured `#807022`:
    /// a step brighter than [`Theme::find_match`].
    pub find_current: Rgb,
    /// Tooltip panel. Shares the flyout's recessed value so the two floating
    /// surfaces read as one family.
    pub tooltip_background: Rgb,
    /// One-pixel edge around a tooltip. Without it the panel would merge into
    /// a tab of the same colour behind it.
    pub tooltip_border: Rgb,
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
            selection: parse(&config.selection, Rgb::new(255, 255, 255)),
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
            tab_bar: Rgb::new(51, 51, 51),
            tab_active: Rgb::new(12, 12, 12),
            control_hover: Rgb::new(59, 59, 59),
            control_hover_ink: Rgb::new(227, 227, 227),
            tab_close_hover: Rgb::new(26, 26, 26),
            tab_close: Rgb::new(152, 152, 152),
            tab_close_hover_ink: Rgb::new(238, 238, 238),
            caption_close_hover: Rgb::new(196, 43, 28),
            tab_title: Rgb::new(255, 255, 255),
            tab_title_inactive: Rgb::new(204, 204, 204),
            tab_border: Rgb::new(41, 41, 41),
            tab_separator: Rgb::new(61, 61, 61),
            menu_background: Rgb::new(44, 44, 44),
            menu_hover: Rgb::new(61, 61, 61),
            menu_separator: Rgb::new(61, 61, 61),
            menu_shortcut: Rgb::new(207, 207, 207),
            menu_disabled: Rgb::new(138, 138, 138),
            find_match: Rgb::new(72, 72, 34),
            find_current: Rgb::new(128, 112, 34),
            tooltip_background: Rgb::new(44, 44, 44),
            tooltip_border: Rgb::new(61, 61, 61),
        }
    }

    /// WT inverts the glyphs against the selection band rather than tinting
    /// them, so the ink has to follow the band's brightness: white text on the
    /// default white band would be invisible.
    pub fn selection_text(&self) -> Rgb {
        let luma = 0.299 * f32::from(self.selection.r)
            + 0.587 * f32::from(self.selection.g)
            + 0.114 * f32::from(self.selection.b);
        if luma > 140.0 {
            Rgb::new(0, 0, 0)
        } else {
            Rgb::new(255, 255, 255)
        }
    }

    /// The ink that stays readable on `background`: the grid's foreground on
    /// a dark fill and the grid's own background on a light one, so a colored
    /// tab's title never drowns in the color it shows.
    pub fn ink_on(&self, background: Rgb) -> Rgb {
        let luminance = 0.299 * f32::from(background.r)
            + 0.587 * f32::from(background.g)
            + 0.114 * f32::from(background.b);
        if luminance > 140.0 {
            self.background
        } else {
            self.foreground
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
        // Measured from Windows Terminal 1.24 at 100 %: `#333333` band,
        // `#0C0C0C` active tab, `#292929` outline, grey `✕`, `#1A1A1A` hover
        // pill, `#3B3B3B` control hover, black-on-white selection.
        assert_eq!(theme.tab_bar, Rgb::new(51, 51, 51));
        assert_eq!(theme.tab_active, Rgb::new(12, 12, 12));
        assert_eq!(theme.control_hover, Rgb::new(59, 59, 59));
        assert_eq!(theme.control_hover_ink, Rgb::new(227, 227, 227));
        assert_eq!(theme.tab_close_hover, Rgb::new(26, 26, 26));
        assert_eq!(theme.tab_close, Rgb::new(152, 152, 152));
        assert_eq!(theme.tab_close_hover_ink, Rgb::new(238, 238, 238));
        assert_eq!(theme.tab_border, Rgb::new(41, 41, 41));
        assert_eq!(theme.tab_separator, Rgb::new(61, 61, 61));
        assert_eq!(theme.caption_close_hover, Rgb::new(196, 43, 28));
        assert_eq!(theme.tab_title, Rgb::new(255, 255, 255));
        assert_eq!(theme.tab_title_inactive, Rgb::new(204, 204, 204));
        assert_eq!(theme.selection, Rgb::new(255, 255, 255));

        assert_eq!(
            theme.tab_active, theme.background,
            "the active tab is the terminal background, so it reads as a cut-out"
        );
        assert!(
            theme.control_hover.r > theme.tab_bar.r,
            "a hovered control must lift off the band it sits on"
        );
        assert!(
            theme.tab_bar.r > theme.tab_close_hover.r,
            "the close pill must stay darker than the band around the tab"
        );
        assert!(
            theme.tab_close_hover.r > theme.tab_active.r,
            "the close pill must still show on the darker active tab"
        );
        assert!(
            theme.tab_border.r > theme.tab_active.r && theme.tab_border.r < theme.tab_bar.r,
            "the outline is a seam between the tab and the band"
        );
    }

    #[test]
    fn selection_ink_contrasts_with_the_selection_band() {
        let mut theme = Theme::from_config(&crate::config::ThemeConfig::default());
        // WT paints the default white band with black glyphs.
        assert_eq!(theme.selection_text(), Rgb::new(0, 0, 0));

        theme.selection = Rgb::new(38, 79, 120);
        assert_eq!(
            theme.selection_text(),
            Rgb::new(255, 255, 255),
            "a dark custom band must flip the ink to stay readable"
        );
    }

    #[test]
    fn the_menu_palette_matches_the_measured_flyout() {
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        assert_eq!(
            theme.menu_background,
            Rgb::new(44, 44, 44),
            "the flyout panel is #2C2C2C"
        );
        assert_eq!(theme.menu_hover, Rgb::new(61, 61, 61));
        assert_eq!(
            theme.menu_separator,
            Rgb::new(61, 61, 61),
            "the group rule is #3D3D3D"
        );
        assert_eq!(
            theme.menu_shortcut,
            Rgb::new(207, 207, 207),
            "shortcuts are dimmer than the labels"
        );
        assert!(
            theme.menu_hover.r > theme.menu_background.r,
            "a hovered row must stand out from the panel"
        );
        assert!(
            theme.menu_shortcut.r < theme.tab_title.r,
            "the key hint must not compete with the label"
        );
    }

    #[test]
    fn the_context_menu_and_find_tokens_pin_their_measured_values() {
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        assert_eq!(
            theme.menu_disabled,
            Rgb::new(138, 138, 138),
            "a row that cannot act dims to #8A8A8A, below the resting label"
        );
        assert!(
            theme.menu_disabled.r < theme.foreground.r,
            "the disabled ink must read dimmer than the enabled label's"
        );
        assert_eq!(
            theme.find_match,
            Rgb::new(72, 72, 34),
            "a find match bands #484822 behind the text"
        );
        assert_eq!(
            theme.find_current,
            Rgb::new(128, 112, 34),
            "the current match bands #807022, a step brighter"
        );
    }

    #[test]
    fn ink_on_flips_with_the_background_luma() {
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        assert_eq!(
            theme.ink_on(Rgb::new(0, 0, 0)),
            theme.foreground,
            "ink on a dark fill is the grid's light foreground"
        );
        assert_eq!(
            theme.ink_on(Rgb::new(255, 255, 255)),
            theme.background,
            "ink on a light fill flips to the grid's own background"
        );
        assert_eq!(
            theme.ink_on(Rgb::new(150, 150, 150)),
            theme.background,
            "a fill past the luma threshold counts as light"
        );
        assert_eq!(
            theme.ink_on(Rgb::new(130, 130, 130)),
            theme.foreground,
            "a fill under the luma threshold counts as dark"
        );
    }
}
