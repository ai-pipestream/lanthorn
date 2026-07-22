//! v6 layout classification: split the engine's flat window list into the
//! single scrolling story window (a primary `Buffer`) and everything else
//! (chrome — frame graphics, status grids, etc.). Pure classification, no
//! rendering (Phase 1a).

use crate::engine::{PositionedWindow, WinNode};

/// The v6 window list split into the one story window and the rest (chrome),
/// in input order.
pub struct V6Layout<'a> {
    pub story: Option<&'a PositionedWindow>,
    pub chrome: Vec<&'a PositionedWindow>,
}

/// Classify `items`: the first primary `Buffer` becomes `story`; every other
/// entry (in input order) goes into `chrome`. With no primary `Buffer`,
/// `story` is `None` and all entries are chrome.
pub fn classify_windows(items: &[PositionedWindow]) -> V6Layout<'_> {
    let mut story = None;
    let mut chrome = Vec::new();
    for pw in items {
        if story.is_none() && matches!(&pw.node, WinNode::Buffer(b) if b.primary) {
            story = Some(pw);
        } else {
            chrome.push(pw);
        }
    }
    V6Layout { story, chrome }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{BorderPref, BufferWindow, GraphicsWindow, GridWindow};
    use std::sync::Arc;

    fn grid_item(x_px: u16) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                cols: 1, rows: 1, cells: vec![], active_rows: 1, cursor: (0, 0), cursor_active: false,
                border: BorderPref::Unspecified, bg: None, fg: None, reverse: false,
            }),
        }
    }

    fn graphics_item(x_px: u16) -> PositionedWindow {
        let canvas = Arc::new(image::RgbaImage::new(1, 1));
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow { win: 0, canvas, version: 0, upscale: false }),
        }
    }

    fn buffer_item(x_px: u16, primary: bool) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary, ..Default::default() }),
        }
    }

    #[test]
    fn story_is_the_primary_buffer_and_chrome_preserves_order() {
        let items = vec![graphics_item(1), grid_item(2), buffer_item(3, true)];
        let layout = classify_windows(&items);
        let story = layout.story.expect("primary buffer found");
        assert!(matches!(&story.node, WinNode::Buffer(b) if b.primary));
        assert_eq!(story.x_px, 3);
        assert_eq!(layout.chrome.len(), 2);
        assert_eq!(layout.chrome[0].x_px, 1);
        assert_eq!(layout.chrome[1].x_px, 2);
    }

    #[test]
    fn no_primary_buffer_means_no_story_and_all_chrome() {
        let items = vec![grid_item(1), graphics_item(2), buffer_item(3, false)];
        let layout = classify_windows(&items);
        assert!(layout.story.is_none());
        assert_eq!(layout.chrome.len(), items.len());
    }
}
