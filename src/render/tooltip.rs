//! Painting for a control's tooltip.

use super::{ui_text, Renderer};
use crate::ui::tab_bar::Rect;
use crate::ui::tooltip::{
    Tooltip, TOOLTIP_PADDING_X, TOOLTIP_PADDING_Y, TOOLTIP_RADIUS, TOOLTIP_TEXT_PX,
};

impl Renderer {
    /// Paints `tooltip` over the surface.
    ///
    /// The caller paints this after the menu, so a tooltip can never end up
    /// behind a panel that is already open.
    pub fn draw_tooltip(&mut self, tooltip: &Tooltip) {
        let rect = tooltip.rect;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let k = tooltip.scale;
        // The frame is a rounded rect one pixel larger than the panel: the
        // renderer has no stroke primitive, and the inset fill hides all but
        // the edge.
        self.fill_round_rect(rect, TOOLTIP_RADIUS * k, self.theme.tooltip_border);
        let inset = k;
        self.fill_round_rect(
            Rect {
                x: rect.x + inset,
                y: rect.y + inset,
                w: (rect.w - 2.0 * inset).max(1.0),
                h: (rect.h - 2.0 * inset).max(1.0),
            },
            (TOOLTIP_RADIUS * k - inset).max(0.0),
            self.theme.tooltip_background,
        );
        let text_x = rect.x + (TOOLTIP_PADDING_X + 1.0) * k;
        ui_text::draw_text(
            self,
            &tooltip.text,
            text_x,
            rect.y + (TOOLTIP_PADDING_Y + 1.0) * k,
            TOOLTIP_TEXT_PX * k,
            self.theme.foreground,
            (rect.x + rect.w - text_x - (TOOLTIP_PADDING_X + 1.0) * k).max(1.0),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::UI_FONT_ID;
    use crate::ui::tooltip::TOOLTIP_TEXT_PX;

    fn tooltip(text: &str) -> Tooltip {
        let renderer = crate::render::tests::default_renderer();
        let fonts = renderer.fonts();
        let width = fonts.width_in(UI_FONT_ID, text, TOOLTIP_TEXT_PX);
        let line = fonts.ui_line_height(TOOLTIP_TEXT_PX);
        let anchor = Rect {
            x: 100.0,
            y: 0.0,
            w: 32.0,
            h: crate::ui::tab_bar::TAB_STRIP_HEIGHT,
        };
        Tooltip::new(text, anchor, width, line, 600.0, 400.0).expect("a tooltip must fit")
    }

    #[test]
    fn the_panel_paints_its_fill_and_its_edge() {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(600, 400);
        let tip = tooltip("New tab");
        renderer.draw_tooltip(&tip);
        assert_eq!(
            pixel_at(&renderer, tip.rect.x + tip.rect.w / 2.0, tip.rect.y),
            renderer.theme.tooltip_border,
            "the top edge is the frame colour"
        );
        assert_eq!(
            pixel_at(&renderer, tip.rect.x + tip.rect.w / 2.0, tip.rect.y + 5.0),
            renderer.theme.tooltip_background,
            "the panel body is the tooltip fill"
        );
    }

    #[test]
    fn the_label_paints_inside_the_panel() {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(600, 400);
        let tip = tooltip("New tab");
        renderer.draw_tooltip(&tip);
        let background = renderer.theme.tooltip_background;
        let ink = (tip.rect.y as u32..(tip.rect.y + tip.rect.h) as u32)
            .flat_map(|y| {
                (tip.rect.x as u32..(tip.rect.x + tip.rect.w) as u32).map(move |x| (x, y))
            })
            .filter(|(x, y)| pixel_at(&renderer, *x as f32, *y as f32) != background)
            .count();
        assert!(ink > 0, "the tooltip must paint its label");
    }

    fn pixel_at(renderer: &Renderer, x: f32, y: f32) -> crate::theme::Rgb {
        let width = renderer.pixmap.width() as usize;
        let index = (y as usize * width + x as usize) * 4;
        let data = renderer.pixmap.data();
        crate::theme::Rgb::new(data[index], data[index + 1], data[index + 2])
    }
}
