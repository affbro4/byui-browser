//! Empty toolbar layout only; controls and browser content are not implemented.

const HEIGHT: f64 = 48.0; // Logical pixels, including the bottom divider.
const BACKGROUND: u32 = 0x00efefef;
const DIVIDER: u32 = 0x00cccccc;
const CONTENT: u32 = 0x00ffffff;

/// Paints a full-width toolbar and blank content into an RGB window buffer.
/// `width` is in device pixels; `scale_factor` converts logical to device pixels.
pub(super) fn draw(pixels: &mut [u32], width: usize, scale_factor: f64) {
    pixels.fill(CONTENT);
    if width == 0 {
        return;
    }
    let toolbar_height = (HEIGHT * scale_factor).round() as usize;
    let divider_height = (scale_factor.round() as usize).max(1);
    for (y, row) in pixels.chunks_mut(width).take(toolbar_height).enumerate() {
        row.fill(if y >= toolbar_height.saturating_sub(divider_height) {
            DIVIDER
        } else {
            BACKGROUND
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolbar_spans_width_and_reserves_content_at_each_scale() {
        for scale in [1.0, 1.5, 2.0] {
            for width in [1, 17, 640] {
                let height = (HEIGHT * scale).round() as usize;
                let divider = (scale.round() as usize).max(1);
                let mut pixels = vec![0; width * (height + 10)];
                draw(&mut pixels, width, scale);
                assert!(
                    pixels[..width * (height - divider)]
                        .iter()
                        .all(|p| *p == BACKGROUND)
                );
                assert!(
                    pixels[width * (height - divider)..width * height]
                        .iter()
                        .all(|p| *p == DIVIDER)
                );
                assert!(pixels[width * height..].iter().all(|p| *p == CONTENT));
            }
        }
    }

    #[test]
    fn tiny_and_empty_windows_are_safe() {
        let mut pixels = vec![0; 6];
        draw(&mut pixels, 3, 2.0);
        assert_eq!(pixels, vec![BACKGROUND; 6]);
        draw(&mut [], 0, 1.0);
    }
}
