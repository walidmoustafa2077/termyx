//! Painting for the inline tab rename editor.

use super::{ui_text, Renderer};
use crate::fonts::SEMIBOLD_FONT_ID;
use crate::ui::rename::RenameState;
use crate::ui::tab_bar::{Rect, TAB_TITLE_SIZE};

impl Renderer {
    /// Paints the rename editor over the tab body, replacing the tab's title
    /// with the text typed so far and a caret after it. An empty edit shows the
    /// current title as a ghost (the shortcut grey) so the field reads as
    /// editing-in-place rather than blank.
    pub fn draw_rename(&mut self, tab: Rect, current: &str, rename: &RenameState, scale: f32) {
        let field = Rect {
            x: tab.x + 6.0 * scale,
            y: tab.y + 5.0 * scale,
            w: (tab.w - 12.0 * scale).max(1.0),
            h: (tab.h - 10.0 * scale).max(1.0),
        };
        self.fill_round_rect(field, 4.0 * scale, self.theme.control_hover);
        let px = TAB_TITLE_SIZE * scale;
        let text = if rename.text.is_empty() {
            current
        } else {
            &rename.text
        };
        let color = if rename.text.is_empty() {
            self.theme.menu_shortcut
        } else {
            self.theme.tab_title
        };
        let style = ui_text::TextStyle {
            face: SEMIBOLD_FONT_ID,
            px,
            color,
        };
        let text_x = field.x + 6.0 * scale;
        let max_width = (field.w - 12.0 * scale).max(1.0);
        ui_text::draw_text_ellipsis_in(
            self,
            text,
            text_x,
            field.y + (field.h - px) * 0.5,
            style,
            max_width,
        );
        if !rename.text.is_empty() {
            let caret_x = text_x
                + self
                    .fonts
                    .width_in(style.face, &rename.text, style.px)
                    .min(max_width - 2.0 * scale);
            self.fill_rect(
                caret_x,
                field.y + 4.0 * scale,
                1.0,
                field.h - 8.0 * scale,
                self.theme.cursor,
            );
        }
    }
}
