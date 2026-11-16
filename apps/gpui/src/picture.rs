//! The picture preview's zoom, pan and marked regions, without a window.
//!
//! It follows the WebView's image preview (`ZoomableFilePreview` in
//! `ui/src/app_support/previews.rs`): 100% draws the picture as wide as the
//! preview, and the zoom goes from 25% to 400% in steps of 25; the wheel
//! zooms; a drag moves the picture when it is larger than the preview; and
//! a region is held in fractions of the picture (`wisp_dto::image_pins`), so
//! it stays on the same part of it at any zoom.
use std::collections::HashMap;

use wisp_dto::image_pins::{ImagePin, ImageRegion};

pub const ZOOM_MIN: u16 = 25;
pub const ZOOM_MAX: u16 = 400;
pub const ZOOM_STEP: u16 = 25;
/// As wide as the preview.
pub const ZOOM_FIT: u16 = 100;
/// Scrolling on a trackpad, in pixels, that makes one step of zoom. The
/// WebView steps once per wheel event, which a trackpad sends dozens of a
/// second; a mouse wheel's notch is one step in both.
pub const WHEEL_STEP_PX: f32 = 40.0;

/// A rectangle in the preview, in pixels from its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// How a picture is shown: its zoom and how far it is scrolled when it is
/// larger than the preview.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    zoom: u16,
    /// How far the picture is moved left and up, as a scrolled page is.
    scroll: (f32, f32),
    /// Trackpad scrolling not yet turned into a step.
    wheel: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            zoom: ZOOM_FIT,
            scroll: (0.0, 0.0),
            wheel: 0.0,
        }
    }
}

/// The size a picture of `picture` pixels is drawn at, zoomed, in a
/// preview of `viewport`: the zoom is a share of the preview's width.
fn drawn(zoom: u16, picture: (f32, f32), viewport: (f32, f32)) -> Option<(f32, f32)> {
    let usable = |value: f32| value.is_finite() && value > 0.0;
    if !(usable(picture.0) && usable(picture.1) && usable(viewport.0) && usable(viewport.1)) {
        return None;
    }
    let width = viewport.0 * f32::from(zoom) / 100.0;
    Some((width, width * picture.1 / picture.0))
}

/// How far a picture `drawn` long can be scrolled in a preview `room` long.
fn reach(drawn: f32, room: f32) -> f32 {
    (drawn - room).max(0.0)
}

impl View {
    pub fn zoom(&self) -> u16 {
        self.zoom
    }

    pub fn can_zoom_in(&self) -> bool {
        self.zoom < ZOOM_MAX
    }

    pub fn can_zoom_out(&self) -> bool {
        self.zoom > ZOOM_MIN
    }

    /// Where the picture is drawn in the preview. Narrower or shorter than
    /// the preview, it is centred that way; larger, it is where it was
    /// scrolled to, never past its edge.
    pub fn frame(&self, picture: (f32, f32), viewport: (f32, f32)) -> Option<Rect> {
        let (width, height) = drawn(self.zoom, picture, viewport)?;
        let place = |drawn: f32, room: f32, scrolled: f32| {
            if drawn <= room {
                (room - drawn) / 2.0
            } else {
                -scrolled.clamp(0.0, reach(drawn, room))
            }
        };
        Some(Rect {
            x: place(width, viewport.0, self.scroll.0),
            y: place(height, viewport.1, self.scroll.1),
            width,
            height,
        })
    }

    /// Whether a drag can move the picture: it is larger than the preview
    /// one way or the other.
    pub fn can_pan(&self, picture: (f32, f32), viewport: (f32, f32)) -> bool {
        drawn(self.zoom, picture, viewport).is_some_and(|(width, height)| {
            reach(width, viewport.0) > 0.0 || reach(height, viewport.1) > 0.0
        })
    }

    /// Moves the picture with the pointer by `dx`, `dy` pixels, as far as
    /// it goes.
    pub fn pan_by(&mut self, dx: f32, dy: f32, picture: (f32, f32), viewport: (f32, f32)) {
        let Some((width, height)) = drawn(self.zoom, picture, viewport) else {
            return;
        };
        self.scroll = (
            (self.scroll.0 - dx).clamp(0.0, reach(width, viewport.0)),
            (self.scroll.1 - dy).clamp(0.0, reach(height, viewport.1)),
        );
    }

    /// Zooms to `zoom` (kept within 25% and 400%), keeping the part of the
    /// picture under `anchor` (a point in the preview; its middle when
    /// `None`) where it is. Whether the zoom changed.
    pub fn zoom_to(
        &mut self,
        zoom: u16,
        anchor: Option<(f32, f32)>,
        picture: (f32, f32),
        viewport: (f32, f32),
    ) -> bool {
        let zoom = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        if zoom == self.zoom {
            return false;
        }
        let before = self.frame(picture, viewport);
        self.zoom = zoom;
        let (Some(before), Some((width, height))) = (before, drawn(zoom, picture, viewport)) else {
            return true;
        };
        let anchor = anchor.unwrap_or((viewport.0 / 2.0, viewport.1 / 2.0));
        // The point of the picture under the anchor, in fractions.
        let held = (
            (anchor.0 - before.x) / before.width,
            (anchor.1 - before.y) / before.height,
        );
        self.scroll = (
            (held.0 * width - anchor.0).clamp(0.0, reach(width, viewport.0)),
            (held.1 * height - anchor.1).clamp(0.0, reach(height, viewport.1)),
        );
        true
    }

    /// Zooms `steps` steps of 25% in (or out, when negative).
    pub fn zoom_by(
        &mut self,
        steps: i32,
        anchor: Option<(f32, f32)>,
        picture: (f32, f32),
        viewport: (f32, f32),
    ) -> bool {
        let zoom = (i32::from(self.zoom) + steps * i32::from(ZOOM_STEP))
            .clamp(i32::from(ZOOM_MIN), i32::from(ZOOM_MAX));
        self.zoom_to(zoom as u16, anchor, picture, viewport)
    }

    /// The wheel over the picture: up zooms in, down zooms out, at the
    /// pointer. `delta_y` is the window's (positive when the wheel turns
    /// up); `precise` is a trackpad's, in pixels, rather than a wheel's
    /// notches. Whether the zoom changed.
    pub fn wheel(
        &mut self,
        delta_y: f32,
        precise: bool,
        anchor: (f32, f32),
        picture: (f32, f32),
        viewport: (f32, f32),
    ) -> bool {
        if !delta_y.is_finite() || delta_y == 0.0 {
            return false;
        }
        let steps = if precise {
            // Turning back starts counting again.
            if self.wheel.signum() != delta_y.signum() {
                self.wheel = 0.0;
            }
            self.wheel += delta_y;
            let steps = (self.wheel / WHEEL_STEP_PX).trunc();
            self.wheel -= steps * WHEEL_STEP_PX;
            steps as i32
        } else {
            delta_y.signum() as i32
        };
        steps != 0 && self.zoom_by(steps, Some(anchor), picture, viewport)
    }
}

/// The point of the picture under `at` (a point in the preview), in
/// fractions of the picture, kept on the picture.
pub fn to_picture(frame: Rect, at: (f32, f32)) -> (f64, f64) {
    let along = |offset: f32, length: f32| {
        if length > 0.0 {
            f64::from(offset / length).clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    (
        along(at.0 - frame.x, frame.width),
        along(at.1 - frame.y, frame.height),
    )
}

/// Where a point of the picture, in fractions of it, is in the preview.
pub fn to_viewport(frame: Rect, at: (f64, f64)) -> (f32, f32) {
    (
        frame.x + at.0 as f32 * frame.width,
        frame.y + at.1 as f32 * frame.height,
    )
}

/// Where a region of the picture is in the preview.
pub fn region_in_viewport(frame: Rect, region: &ImageRegion) -> Rect {
    let (x, y) = to_viewport(frame, (region.x, region.y));
    Rect {
        x,
        y,
        width: region.w as f32 * frame.width,
        height: region.h as f32 * frame.height,
    }
}

/// Where the note field of a marked region goes in a preview of `viewport`:
/// under the region when there is room, else over it, else as low as it
/// fits; across, centred on the region and kept inside the preview.
pub fn popup_at(region: Rect, viewport: (f32, f32), size: (f32, f32)) -> (f32, f32) {
    const GAP: f32 = 6.0;
    const EDGE: f32 = 4.0;
    let x = (region.x + region.width / 2.0 - size.0 / 2.0)
        .min(viewport.0 - size.0 - EDGE)
        .max(EDGE);
    let below = region.y + region.height + GAP;
    let above = region.y - GAP - size.1;
    let y = if below + size.1 <= viewport.1 - EDGE {
        below.max(EDGE)
    } else if above >= EDGE {
        above
    } else {
        (viewport.1 - size.1 - EDGE).max(EDGE)
    };
    (x, y)
}

/// A region being marked, or marked and waiting for a note: where the drag
/// began and where it is now, in fractions of the picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub start: (f64, f64),
    pub end: (f64, f64),
    /// The drag is over and the region kept: its note and actions show.
    pub done: bool,
}

impl Mark {
    pub fn at(point: (f64, f64)) -> Self {
        Self {
            start: point,
            end: point,
            done: false,
        }
    }

    pub fn region(&self) -> ImageRegion {
        ImageRegion::between(self.start, self.end)
    }
}

/// The notes on regions of pictures while the window is open, by project
/// and path, as the WebView keeps them for its session: they are not saved.
#[derive(Default)]
pub struct Pins(HashMap<(String, String), Vec<ImagePin>>);

impl Pins {
    pub fn of(&self, project: &str, path: &str) -> &[ImagePin] {
        self.0
            .get(&(project.to_owned(), path.to_owned()))
            .map_or(&[], Vec::as_slice)
    }

    pub fn add(&mut self, project: &str, path: &str, pin: ImagePin) {
        self.0
            .entry((project.to_owned(), path.to_owned()))
            .or_default()
            .push(pin);
    }

    /// Removes the pin numbered `index + 1`; those after it move up.
    pub fn remove(&mut self, project: &str, path: &str, index: usize) {
        let key = (project.to_owned(), path.to_owned());
        if let Some(list) = self.0.get_mut(&key) {
            if index < list.len() {
                list.remove(index);
            }
            if list.is_empty() {
                self.0.remove(&key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PICTURE: (f32, f32) = (800.0, 600.0);
    const PREVIEW: (f32, f32) = (400.0, 500.0);

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn at_first_the_picture_is_as_wide_as_the_preview_and_centred() {
        let view = View::default();
        assert_eq!(view.zoom(), 100);
        let frame = view.frame(PICTURE, PREVIEW).unwrap();
        // 400 wide keeps the picture's shape: 300 high, centred in 500.
        assert_eq!(
            frame,
            Rect {
                x: 0.0,
                y: 100.0,
                width: 400.0,
                height: 300.0
            }
        );
        assert!(!view.can_pan(PICTURE, PREVIEW));
        // A picture smaller than the preview is drawn as wide, as the
        // WebView draws it.
        let tiny = view.frame((40.0, 10.0), PREVIEW).unwrap();
        assert_eq!((tiny.width, tiny.height), (400.0, 100.0));
        // Nothing to draw into, or nothing to draw.
        assert_eq!(view.frame(PICTURE, (0.0, 500.0)), None);
        assert_eq!(view.frame((0.0, 10.0), PREVIEW), None);
    }

    #[test]
    fn zoom_steps_by_a_quarter_between_25_and_400_percent() {
        let mut view = View::default();
        assert!(view.zoom_by(1, None, PICTURE, PREVIEW));
        assert_eq!(view.zoom(), 125);
        assert!(view.zoom_by(100, None, PICTURE, PREVIEW));
        assert_eq!(view.zoom(), 400);
        assert!(!view.can_zoom_in());
        assert!(!view.zoom_by(1, None, PICTURE, PREVIEW));
        assert!(view.zoom_by(-100, None, PICTURE, PREVIEW));
        assert_eq!(view.zoom(), 25);
        assert!(!view.can_zoom_out());
        // Zoomed out, the picture is a quarter as wide, in the middle.
        let frame = view.frame(PICTURE, PREVIEW).unwrap();
        assert_eq!((frame.x, frame.width), (150.0, 100.0));
        // Reset is back to the preview's width.
        assert!(view.zoom_to(ZOOM_FIT, None, PICTURE, PREVIEW));
        assert_eq!(view.zoom(), 100);
        assert!(!view.zoom_to(ZOOM_FIT, None, PICTURE, PREVIEW));
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer_where_it_is() {
        let mut view = View::default();
        let at = (100.0, 250.0);
        let before = to_picture(view.frame(PICTURE, PREVIEW).unwrap(), at);
        view.zoom_by(4, Some(at), PICTURE, PREVIEW);
        assert_eq!(view.zoom(), 200);
        let frame = view.frame(PICTURE, PREVIEW).unwrap();
        let after = to_picture(frame, at);
        assert!((before.0 - after.0).abs() < 1e-4 && (before.1 - after.1).abs() < 1e-4);
        // The buttons zoom about the middle of the preview.
        let mut middle = View::default();
        let centre = (PREVIEW.0 / 2.0, PREVIEW.1 / 2.0);
        middle.zoom_by(4, None, PICTURE, PREVIEW);
        let frame = middle.frame(PICTURE, PREVIEW).unwrap();
        let held = to_picture(frame, centre);
        assert!((held.0 - 0.5).abs() < 1e-4 && (held.1 - 0.5).abs() < 1e-4);
    }

    #[test]
    fn a_drag_moves_a_large_picture_but_never_out_of_view() {
        let mut view = View::default();
        view.zoom_to(200, Some((0.0, 0.0)), PICTURE, PREVIEW);
        // 800 by 600 in a preview of 400 by 500.
        assert!(view.can_pan(PICTURE, PREVIEW));
        let frame = view.frame(PICTURE, PREVIEW).unwrap();
        assert_eq!((frame.x, frame.y), (0.0, 0.0));
        view.pan_by(-150.0, -40.0, PICTURE, PREVIEW);
        let frame = view.frame(PICTURE, PREVIEW).unwrap();
        assert_eq!((frame.x, frame.y), (-150.0, -40.0));
        // As far as its far edge, and no farther.
        view.pan_by(-10_000.0, -10_000.0, PICTURE, PREVIEW);
        let frame = view.frame(PICTURE, PREVIEW).unwrap();
        assert_eq!((frame.x, frame.y), (-400.0, -100.0));
        view.pan_by(10_000.0, 10_000.0, PICTURE, PREVIEW);
        let frame = view.frame(PICTURE, PREVIEW).unwrap();
        assert_eq!((frame.x, frame.y), (0.0, 0.0));
        // Zoomed back out, where it was scrolled no longer counts.
        view.pan_by(-10_000.0, -10_000.0, PICTURE, PREVIEW);
        view.zoom_to(100, None, PICTURE, PREVIEW);
        let frame = view.frame(PICTURE, PREVIEW).unwrap();
        assert_eq!((frame.x, frame.y), (0.0, 100.0));
        // A preview that grows taller takes the scroll back within reach.
        view.zoom_to(200, Some((400.0, 500.0)), PICTURE, PREVIEW);
        let frame = view.frame(PICTURE, PREVIEW).unwrap();
        assert_eq!((frame.x, frame.y), (-400.0, -100.0));
        let taller = view.frame(PICTURE, (400.0, 550.0)).unwrap();
        assert_eq!((taller.x, taller.y), (-400.0, -50.0));
    }

    #[test]
    fn the_wheel_zooms_one_step_a_notch_and_a_step_per_stretch_of_trackpad() {
        let mut view = View::default();
        let at = (200.0, 250.0);
        // A wheel's notch up zooms in, down zooms out.
        assert!(view.wheel(1.0, false, at, PICTURE, PREVIEW));
        assert_eq!(view.zoom(), 125);
        assert!(view.wheel(-3.0, false, at, PICTURE, PREVIEW));
        assert_eq!(view.zoom(), 100);
        // A trackpad's small movements add up to a step.
        assert!(!view.wheel(15.0, true, at, PICTURE, PREVIEW));
        assert!(!view.wheel(15.0, true, at, PICTURE, PREVIEW));
        assert!(view.wheel(15.0, true, at, PICTURE, PREVIEW));
        assert_eq!(view.zoom(), 125);
        // Turning back counts again from nothing.
        assert!(!view.wheel(-30.0, true, at, PICTURE, PREVIEW));
        // 120 px down is three steps out.
        assert!(view.wheel(-90.0, true, at, PICTURE, PREVIEW));
        assert_eq!(view.zoom(), 50);
        assert!(!view.wheel(0.0, false, at, PICTURE, PREVIEW));
    }

    #[test]
    fn points_go_between_the_preview_and_the_picture() {
        let frame = Rect {
            x: -100.0,
            y: 20.0,
            width: 800.0,
            height: 600.0,
        };
        assert_eq!(to_picture(frame, (300.0, 320.0)), (0.5, 0.5));
        // A point off the picture is on its edge.
        assert_eq!(to_picture(frame, (-500.0, 2000.0)), (0.0, 1.0));
        let back = to_viewport(frame, (0.25, 0.75));
        assert!(close(back.0, 100.0) && close(back.1, 470.0));
        let region = ImageRegion::between((0.25, 0.5), (0.5, 0.75));
        let drawn = region_in_viewport(frame, &region);
        assert!(close(drawn.x, 100.0) && close(drawn.y, 320.0));
        assert!(close(drawn.width, 200.0) && close(drawn.height, 150.0));
        // The same region at another zoom covers the same part of it.
        let zoomed = Rect {
            x: 0.0,
            y: 0.0,
            width: 1600.0,
            height: 1200.0,
        };
        let drawn = region_in_viewport(zoomed, &region);
        assert_eq!(to_picture(zoomed, (drawn.x, drawn.y)), (0.25, 0.5));
    }

    #[test]
    fn the_note_field_stands_by_its_region_inside_the_preview() {
        let size = (200.0, 60.0);
        let region = |x, y| Rect {
            x,
            y,
            width: 40.0,
            height: 20.0,
        };
        // Under the region, centred on it.
        assert_eq!(
            popup_at(region(180.0, 100.0), PREVIEW, size),
            (100.0, 126.0)
        );
        // Near the bottom it goes over the region instead.
        assert_eq!(
            popup_at(region(180.0, 460.0), PREVIEW, size),
            (100.0, 394.0)
        );
        // Near a side it stays inside the preview.
        assert_eq!(popup_at(region(-30.0, 100.0), PREVIEW, size).0, 4.0);
        assert_eq!(popup_at(region(390.0, 100.0), PREVIEW, size).0, 196.0);
        // A region as tall as the preview leaves it at the bottom.
        let tall = Rect {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 500.0,
        };
        assert_eq!(popup_at(tall, PREVIEW, size).1, 436.0);
    }

    #[test]
    fn a_mark_covers_its_drag_in_either_direction() {
        let mut mark = Mark::at((0.75, 0.5));
        mark.end = (0.25, 0.25);
        assert_eq!(
            mark.region(),
            ImageRegion::between((0.25, 0.25), (0.75, 0.5))
        );
        assert!(!mark.done);
    }

    #[test]
    fn pins_are_kept_by_project_and_path_and_renumber_when_one_goes() {
        let mut pins = Pins::default();
        let region = ImageRegion::between((0.0, 0.0), (0.5, 0.5));
        pins.add("p1", "figs/a.png", ImagePin::new(region, "first"));
        pins.add("p1", "figs/a.png", ImagePin::new(region, "second"));
        pins.add("p2", "figs/a.png", ImagePin::new(region, "elsewhere"));
        assert_eq!(pins.of("p1", "figs/a.png").len(), 2);
        assert_eq!(pins.of("p2", "figs/a.png")[0].note, "elsewhere");
        assert!(pins.of("p1", "figs/b.png").is_empty());
        pins.remove("p1", "figs/a.png", 0);
        assert_eq!(pins.of("p1", "figs/a.png")[0].note, "second");
        pins.remove("p1", "figs/a.png", 5);
        pins.remove("p1", "figs/a.png", 0);
        assert!(pins.of("p1", "figs/a.png").is_empty());
        assert_eq!(pins.of("p2", "figs/a.png").len(), 1);
    }
}
