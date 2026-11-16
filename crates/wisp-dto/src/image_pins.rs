//! Regions marked on a picture in its preview, and the numbered notes
//! ("pins") written on them. Shared by the WebView's image preview (`ui/`)
//! and the GPUI desktop's, so both measure a region the same way, cut the
//! same pixels out of the picture for it, and put the same words in front of
//! the model.
//!
//! A region is held in fractions of the picture (every value in 0..=1,
//! measured from its top-left corner). That keeps it on the same part of the
//! picture at any zoom or pan, and maps it onto the picture's own pixels
//! whatever size the picture is drawn at.

/// The smallest region, in pixels on the screen along each side, that a
/// drag marks; anything smaller is taken for a stray click.
pub const MIN_REGION_PX: f64 = 8.0;

/// A rectangle of a picture in fractions of its width and height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageRegion {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl ImageRegion {
    /// The region a drag from `start` to `end` covers, the two corners in
    /// either order, kept inside the picture.
    pub fn between(start: (f64, f64), end: (f64, f64)) -> Self {
        let clamp = |value: f64| value.clamp(0.0, 1.0);
        let (left, right) = (clamp(start.0.min(end.0)), clamp(start.0.max(end.0)));
        let (top, bottom) = (clamp(start.1.min(end.1)), clamp(start.1.max(end.1)));
        Self {
            x: left,
            y: top,
            w: right - left,
            h: bottom - top,
        }
    }

    /// Whether the region is large enough to keep on a picture drawn
    /// `shown_width` by `shown_height` pixels on the screen.
    pub fn is_marked(&self, shown_width: f64, shown_height: f64) -> bool {
        self.w * shown_width >= MIN_REGION_PX && self.h * shown_height >= MIN_REGION_PX
    }

    /// The middle of the region, where its numbered marker stands.
    pub fn centre(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// The pixels of a picture `width` by `height` the region covers, as
    /// (left, top, width, height): at least one pixel, all inside the
    /// picture, the way the WebView's canvas crop clamps its source rect.
    pub fn pixels(&self, width: u32, height: u32) -> (u32, u32, u32, u32) {
        let span = |start: f64, length: f64, total: u32| {
            let total = total.max(1);
            let from = ((start * total as f64).round().max(0.0) as u32).min(total - 1);
            let size = ((length * total as f64).round().max(1.0) as u32).min(total - from);
            (from, size)
        };
        let (left, cut_width) = span(self.x, self.w, width);
        let (top, cut_height) = span(self.y, self.h, height);
        (left, top, cut_width, cut_height)
    }
}

/// A note on a region of a picture: the region in fractions of the picture
/// plus what was written about it.
#[derive(Clone, Debug, PartialEq)]
pub struct ImagePin {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub note: String,
}

impl ImagePin {
    pub fn new(region: ImageRegion, note: impl Into<String>) -> Self {
        Self {
            x: region.x,
            y: region.y,
            w: region.w,
            h: region.h,
            note: note.into(),
        }
    }

    pub fn region(&self) -> ImageRegion {
        ImageRegion {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
        }
    }
}

/// The "send for AI revision" text: the localized head line naming the
/// file, then one numbered line per pin with its region in fractions. It
/// reaches the model as a quote in front of the message.
pub fn pin_review_message(head: &str, pins: &[ImagePin]) -> String {
    let mut out = String::from(head);
    for (index, pin) in pins.iter().enumerate() {
        out.push_str(&format!(
            "\n{}. (x={:.3}, y={:.3}, w={:.3}, h={:.3}) {}",
            index + 1,
            pin.x,
            pin.y,
            pin.w,
            pin.h,
            pin.note
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_marks_the_region_between_its_corners_inside_the_picture() {
        let region = ImageRegion::between((0.6, 0.8), (0.2, 0.3));
        assert_eq!((region.x, region.y), (0.2, 0.3));
        assert!((region.w - 0.4).abs() < 1e-9 && (region.h - 0.5).abs() < 1e-9);
        // Dragged past the picture's edge, it stops at the edge.
        let past = ImageRegion::between((0.5, -0.2), (1.4, 0.5));
        assert_eq!(
            past,
            ImageRegion {
                x: 0.5,
                y: 0.0,
                w: 0.5,
                h: 0.5
            }
        );
        assert_eq!(past.centre(), (0.75, 0.25));
    }

    #[test]
    fn a_stray_click_is_not_a_region() {
        let small = ImageRegion::between((0.1, 0.1), (0.11, 0.3));
        // One hundredth of a picture drawn 400 px wide is 4 px.
        assert!(!small.is_marked(400.0, 400.0));
        // Zoomed in to 1000 px, the same region is 10 px across.
        assert!(small.is_marked(1000.0, 1000.0));
        assert!(!ImageRegion::between((0.5, 0.5), (0.5, 0.5)).is_marked(4000.0, 4000.0));
    }

    #[test]
    fn a_region_maps_onto_the_pictures_own_pixels() {
        let region = ImageRegion {
            x: 0.5,
            y: 0.0,
            w: 0.5,
            h: 1.0 / 3.0,
        };
        assert_eq!(region.pixels(4, 3), (2, 0, 2, 1));
        // Never empty and never past the picture's edge.
        let sliver = ImageRegion {
            x: 1.0,
            y: 1.0,
            w: 0.0,
            h: 0.0,
        };
        assert_eq!(sliver.pixels(4, 3), (3, 2, 1, 1));
        let whole = ImageRegion::between((0.0, 0.0), (1.0, 1.0));
        assert_eq!(whole.pixels(640, 480), (0, 0, 640, 480));
    }

    #[test]
    fn pin_review_message_numbers_fractional_regions() {
        let pins = vec![
            ImagePin::new(
                ImageRegion {
                    x: 0.5,
                    y: 0.25,
                    w: 0.1,
                    h: 0.05,
                },
                "对比度太低",
            ),
            ImagePin {
                x: 0.0,
                y: 0.75,
                w: 0.25,
                h: 0.25,
                note: "crop this corner".into(),
            },
        ];
        let (x, y) = pins[0].region().centre();
        assert!((x - 0.55).abs() < 1e-9 && (y - 0.275).abs() < 1e-9);
        assert_eq!(
            pin_review_message("Revision pins on image `results/plot.png`:", &pins),
            "Revision pins on image `results/plot.png`:\n\
             1. (x=0.500, y=0.250, w=0.100, h=0.050) 对比度太低\n\
             2. (x=0.000, y=0.750, w=0.250, h=0.250) crop this corner"
        );
        assert_eq!(pin_review_message("head", &[]), "head");
    }
}
