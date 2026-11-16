//! A picture in the Files panel's preview, zoomed and moved as the
//! WebView's image preview is, with regions marked on it: a numbered note
//! on a region goes to the message as a quote, and the region itself can be
//! cut out and attached. The arithmetic is in `crate::picture`.
use crate::i18n::{fill, tr};
use std::sync::Arc;

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
    tooltip::Tooltip,
};
use wisp_dto::image_pins::{ImagePin, pin_review_message};

use super::{
    Workspace,
    shell::{glyph, icon_button},
};
use crate::{
    backend::receive,
    compose::{self, Chip},
    layers::Layer,
    picture::{self, Mark},
};

/// The note field and the actions under a marked region, for placing it.
const POPUP_SIZE: (f32, f32) = (300.0, 96.0);
/// One wheel notch, in pixels, for a wheel that reports lines.
const WHEEL_LINE: f32 = 20.0;

impl Workspace {
    /// Opening a file stops marking; a picture other than the one shown
    /// starts as wide as the preview, as the WebView opens every picture,
    /// while the same one opened again keeps its zoom.
    pub(super) fn reset_picture(&mut self, same_file: bool) {
        if !same_file {
            self.picture = picture::View::default();
        }
        self.picture_pan = None;
        self.picture_mark = None;
        self.layers.close(Layer::PictureSelect);
    }

    /// Regions are marked, and their notes sent, only into a conversation
    /// that can be written to, as files are attached.
    fn can_mark_picture(&self) -> bool {
        self.state.session.is_some() && self.read_only().is_none()
    }

    /// The previewed picture's path, when the picture is what is shown.
    fn shown_picture(&self) -> Option<&str> {
        self.file_image
            .as_ref()
            .map(|(path, _)| path.as_str())
            .filter(|path| self.files.preview_path.as_deref() == Some(*path))
    }

    /// Where the picture preview is and how large the picture is, with a
    /// point in the window as a point in the preview.
    fn picture_place(&self, at: Point<Pixels>) -> Option<((f32, f32), (f32, f32), (f32, f32))> {
        let (bounds, natural) = self.picture_geometry.get()?;
        let viewport = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        let local = (
            f32::from(at.x - bounds.origin.x),
            f32::from(at.y - bounds.origin.y),
        );
        Some((natural, viewport, local))
    }

    /// Where the picture is drawn in the preview, as last measured.
    fn picture_frame(&self) -> Option<picture::Rect> {
        let (bounds, natural) = self.picture_geometry.get()?;
        let viewport = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        self.picture.frame(natural, viewport)
    }

    fn zoom_picture(&mut self, steps: i32, cx: &mut Context<Self>) {
        match self.picture_geometry.get() {
            Some((bounds, natural)) => {
                let viewport = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                self.picture.zoom_by(steps, None, natural, viewport);
            }
            None => {
                self.picture.zoom_by(steps, None, (0.0, 0.0), (0.0, 0.0));
            }
        }
        cx.notify();
    }

    fn reset_picture_zoom(&mut self, cx: &mut Context<Self>) {
        let (natural, viewport) = self
            .picture_geometry
            .get()
            .map(|(bounds, natural)| {
                (
                    natural,
                    (f32::from(bounds.size.width), f32::from(bounds.size.height)),
                )
            })
            .unwrap_or_default();
        self.picture
            .zoom_to(picture::ZOOM_FIT, None, natural, viewport);
        cx.notify();
    }

    fn toggle_picture_marking(&mut self, cx: &mut Context<Self>) {
        self.picture_mark = None;
        if !self.layers.close(Layer::PictureSelect) && self.can_mark_picture() {
            self.layers.open(Layer::PictureSelect);
        }
        cx.notify();
    }

    fn picture_down(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((natural, viewport, local)) = self.picture_place(at) else {
            return;
        };
        if self.layers.contains(Layer::PictureSelect) {
            // A region waiting for its note keeps the picture until it is
            // noted, attached or given up.
            if self.layers.contains(Layer::PictureRegion) || self.picture_cutting {
                return;
            }
            if let Some(frame) = self.picture.frame(natural, viewport) {
                self.picture_mark = Some(Mark::at(picture::to_picture(frame, local)));
                cx.notify();
            }
        } else if self.picture.can_pan(natural, viewport) {
            self.picture_pan = Some(at);
        }
    }

    fn picture_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.pressed_button != Some(MouseButton::Left) {
            self.picture_up(window, cx);
            return;
        }
        let Some((natural, viewport, local)) = self.picture_place(event.position) else {
            return;
        };
        if let Some(last) = self.picture_pan {
            self.picture.pan_by(
                f32::from(event.position.x - last.x),
                f32::from(event.position.y - last.y),
                natural,
                viewport,
            );
            self.picture_pan = Some(event.position);
            cx.notify();
        } else if let Some(mark) = self.picture_mark.as_mut().filter(|mark| !mark.done) {
            if let Some(frame) = self.picture.frame(natural, viewport) {
                mark.end = picture::to_picture(frame, local);
                cx.notify();
            }
        }
    }

    /// The end of a drag: a region large enough is kept and asks for its
    /// note; a stray click is forgotten.
    fn picture_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picture_pan = None;
        let Some(mut mark) = self.picture_mark.filter(|mark| !mark.done) else {
            return;
        };
        let marked = self.picture_frame().is_some_and(|frame| {
            mark.region()
                .is_marked(f64::from(frame.width), f64::from(frame.height))
        });
        if marked && self.layers.contains(Layer::PictureSelect) {
            mark.done = true;
            self.picture_mark = Some(mark);
            self.layers.open(Layer::PictureRegion);
            self.pin_note.update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
        } else {
            self.picture_mark = None;
        }
        cx.notify();
    }

    fn picture_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let Some((natural, viewport, local)) = self.picture_place(event.position) else {
            return;
        };
        let delta = event.delta.pixel_delta(px(WHEEL_LINE));
        if self.picture.wheel(
            f32::from(delta.y),
            event.delta.precise(),
            local,
            natural,
            viewport,
        ) {
            cx.notify();
        }
        cx.stop_propagation();
    }

    /// Gives up the marked region and what was written for it.
    pub(super) fn drop_picture_mark(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picture_mark = None;
        self.pin_note
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    /// Keeps the note written for the marked region as the picture's next
    /// numbered pin; marking goes on for the next region.
    pub(super) fn add_picture_pin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.contains(Layer::PictureRegion) {
            return;
        }
        let note = self.pin_note.read(cx).value().trim().to_owned();
        let (Some(project), Some(path), Some(mark)) = (
            self.state.project.clone(),
            self.shown_picture().map(str::to_owned),
            self.picture_mark.filter(|mark| mark.done),
        ) else {
            return;
        };
        if note.is_empty() {
            return;
        }
        self.picture_pins
            .add(&project, &path, ImagePin::new(mark.region(), note));
        self.layers.close(Layer::PictureRegion);
        self.drop_picture_mark(window, cx);
        cx.notify();
    }

    fn remove_picture_pin(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.layers.contains(Layer::PictureSelect) {
            return;
        }
        if let (Some(project), Some(path)) = (
            self.state.project.clone(),
            self.shown_picture().map(str::to_owned),
        ) {
            self.picture_pins.remove(&project, &path, index);
            cx.notify();
        }
    }

    /// The picture's notes, numbered with their regions, as a quote in
    /// front of the message being written, as the WebView's Send for AI
    /// revision puts them. The notes stay on the picture.
    fn send_picture_pins(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_mark_picture() {
            return;
        }
        let (Some(project), Some(path)) = (
            self.state.project.clone(),
            self.shown_picture().map(str::to_owned),
        ) else {
            return;
        };
        let pins = self.picture_pins.of(&project, &path);
        if pins.is_empty() {
            return;
        }
        let head = fill(
            tr(
                "Revision notes on image `{}` (x/y/w/h are fractional region coordinates from the top-left):",
            ),
            &[&path],
        );
        let text = pin_review_message(&head, pins);
        let filled = compose::with_quote_before(&self.composer.read(cx).value(), &text);
        self.layers.close(Layer::PictureSelect);
        self.picture_mark = None;
        self.state.save_draft(filled.clone());
        self.fill_composer(filled, window, cx);
        cx.notify();
    }

    /// Cuts the marked region out of the picture into `uploads` and puts it
    /// on the message; `jump` also leaves the preview for the composer.
    fn attach_picture_region(&mut self, jump: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.picture_cutting || !self.can_mark_picture() {
            return;
        }
        let selection = self.state.selection();
        let (Some(project), Some(path), Some(mark)) = (
            selection.project.clone(),
            self.shown_picture().map(str::to_owned),
            self.picture_mark.filter(|mark| mark.done),
        ) else {
            return;
        };
        self.picture_cutting = true;
        let name = compose::region_name(chrono::Utc::now().naive_utc());
        let reply = self
            .backend
            .import_image_region(project, path, mark.region(), name);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.picture_cutting = false;
                // The region is given up either way, as in the WebView.
                this.layers.close(Layer::PictureRegion);
                this.drop_picture_mark(window, cx);
                match result {
                    // It belongs to the conversation it was cut for.
                    Ok(kept) if this.state.current(&selection) => {
                        this.attach(Chip::File(kept), cx);
                        this.layers.close(Layer::PictureSelect);
                        if jump {
                            this.close_preview(window, cx);
                            this.focus_composer(window, cx);
                        } else {
                            this.file_notice = Some(tr("Attached to the message.").to_owned());
                        }
                    }
                    Ok(_) => {}
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The picture's controls, under the preview's header: zoom out, the
    /// zoom (a click goes back to the preview's width), zoom in, marking
    /// regions and sending their notes.
    pub(super) fn render_picture_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        let path = self.shown_picture()?;
        let pins = self
            .state
            .project
            .as_deref()
            .map_or(0, |project| self.picture_pins.of(project, path).len());
        let marking = self.layers.contains(Layer::PictureSelect);
        let mut bar = div()
            .h_flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                icon_button("picture-zoom-out", "minus", "Zoom out")
                    .xsmall()
                    .disabled(!self.picture.can_zoom_out())
                    .on_click(cx.listener(|this, _, _, cx| this.zoom_picture(-1, cx))),
            )
            .child(
                Button::new("picture-zoom-reset")
                    .label(format!("{}%", self.picture.zoom()))
                    .xsmall()
                    .ghost()
                    .tooltip(tr("Reset zoom"))
                    .on_click(cx.listener(|this, _, _, cx| this.reset_picture_zoom(cx))),
            )
            .child(
                icon_button("picture-zoom-in", "plus", "Zoom in")
                    .xsmall()
                    .disabled(!self.picture.can_zoom_in())
                    .on_click(cx.listener(|this, _, _, cx| this.zoom_picture(1, cx))),
            );
        if self.can_mark_picture() {
            bar = bar.child(
                icon_button("picture-select", "crop", "Select a region to ask about")
                    .xsmall()
                    .selected(marking)
                    .disabled(self.picture_cutting)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_picture_marking(cx))),
            );
            if pins > 0 {
                bar = bar.child(
                    Button::new("picture-pins-send")
                        .icon(glyph("chat"))
                        .label(fill(tr("Send for AI revision ({})"), &[&pins]))
                        .xsmall()
                        .ghost()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.send_picture_pins(window, cx)),
                        ),
                );
            }
        }
        Some(bar)
    }

    /// The picture, drawn at its zoom from its own pixels (never from a
    /// smaller copy), with the region being marked, the numbered notes and
    /// the note field over it.
    pub(super) fn render_picture(
        &self,
        path: &str,
        image: Arc<Image>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // The picture's own size, once it is decoded; until then it is
        // drawn whole, as it was before it could be zoomed.
        let Some(natural) = image.clone().use_render_image(window, cx).map(|data| {
            let size = data.size(0);
            (size.width.0 as f32, size.height.0 as f32)
        }) else {
            return div()
                .flex_1()
                .min_h_0()
                .p_3()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    img(image)
                        .max_w_full()
                        .max_h_full()
                        .object_fit(ObjectFit::Contain),
                )
                .into_any_element();
        };
        let theme = cx.theme();
        let (primary, ink, border, card) = (
            theme.primary,
            theme.primary_foreground,
            theme.border,
            theme.popover,
        );
        let radius = theme.radius;
        let marking = self.layers.contains(Layer::PictureSelect);
        let viewport = self
            .picture_geometry
            .get()
            .map(|(bounds, _)| (f32::from(bounds.size.width), f32::from(bounds.size.height)));
        let frame = viewport.and_then(|viewport| self.picture.frame(natural, viewport));
        let can_pan = viewport.is_some_and(|viewport| self.picture.can_pan(natural, viewport));

        // Measures where the preview is, for the pointer, and draws again
        // when that changes.
        let geometry = self.picture_geometry.clone();
        let weak = cx.entity().downgrade();
        let measure = canvas(
            move |bounds, window, cx| {
                let next = Some((bounds, natural));
                if geometry.get() != next {
                    geometry.set(next);
                    window.defer(cx, move |_, cx| {
                        let _ = weak.update(cx, |_, cx| cx.notify());
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        let mut shown = div()
            .id("picture-viewport")
            .relative()
            .flex_1()
            .min_h_0()
            .m_3()
            .overflow_hidden()
            .when(marking, |shown| shown.cursor_crosshair())
            .when(!marking && can_pan, |shown| shown.cursor_grab())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.picture_down(event.position, cx)
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                this.picture_move(event, window, cx)
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, window, cx| this.picture_up(window, cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, window, cx| this.picture_up(window, cx)),
            )
            .on_scroll_wheel(
                cx.listener(|this, event: &ScrollWheelEvent, _, cx| this.picture_wheel(event, cx)),
            )
            .child(measure);
        let (Some(frame), Some(viewport)) = (frame, viewport) else {
            return shown.into_any_element();
        };
        shown = shown.child(
            img(image)
                .absolute()
                .left(px(frame.x))
                .top(px(frame.y))
                .w(px(frame.width))
                .h(px(frame.height)),
        );

        // The region being marked, or marked and waiting for its note.
        let mark = self.picture_mark.filter(|_| marking);
        if let Some(mark) = mark {
            let rect = picture::region_in_viewport(frame, &mark.region());
            shown = shown.child(
                div()
                    .absolute()
                    .left(px(rect.x))
                    .top(px(rect.y))
                    .w(px(rect.width))
                    .h(px(rect.height))
                    .border_1()
                    .border_color(primary)
                    .bg(primary.opacity(0.12))
                    .when(mark.done, |rect| {
                        rect.child(
                            div()
                                .px_1()
                                .text_xs()
                                .whitespace_nowrap()
                                .text_color(ink)
                                .bg(primary)
                                .child(tr("Selected region")),
                        )
                    }),
            );
        }

        // The numbered notes, at the middle of their regions; in marking,
        // a click on one removes it.
        let pins = self
            .state
            .project
            .as_deref()
            .map(|project| self.picture_pins.of(project, path))
            .unwrap_or_default();
        for (index, pin) in pins.iter().enumerate() {
            let (x, y) = picture::to_viewport(frame, pin.region().centre());
            let tip = if marking {
                fill(tr("Click to remove: {}"), &[&pin.note])
            } else {
                pin.note.clone()
            };
            shown = shown.child(
                div()
                    .id(("picture-pin", index))
                    .absolute()
                    .left(px(x - 9.0))
                    .top(px(y - 9.0))
                    .min_w(px(18.0))
                    .h(px(18.0))
                    .px_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(radius)
                    .border_1()
                    .border_color(card)
                    .bg(primary)
                    .text_color(ink)
                    .text_xs()
                    .when(marking, |marker| marker.cursor_pointer())
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    // The picture under it does not start a region or a drag.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| this.remove_picture_pin(index, cx)))
                    .child((index + 1).to_string()),
            );
        }

        // The note field and the actions for the marked region.
        if let Some(mark) =
            mark.filter(|mark| mark.done && self.layers.contains(Layer::PictureRegion))
        {
            let rect = picture::region_in_viewport(frame, &mark.region());
            let (left, top) = picture::popup_at(rect, viewport, POPUP_SIZE);
            // The WebView cuts an SVG on its canvas; the host here has no
            // renderer for one, so its regions are only noted.
            let cuttable = !path.to_ascii_lowercase().ends_with(".svg");
            let busy = self.picture_cutting;
            let mut actions = div().h_flex().flex_wrap().gap_1();
            if cuttable {
                actions = actions
                    .child(
                        Button::new("picture-region-attach")
                            .icon(glyph("attach"))
                            .label(tr("Add to chat"))
                            .xsmall()
                            .ghost()
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.attach_picture_region(false, window, cx)
                            })),
                    )
                    .child(
                        Button::new("picture-region-attach-jump")
                            .icon(glyph("chat"))
                            .label(tr("Add to chat and jump back to chat"))
                            .xsmall()
                            .ghost()
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.attach_picture_region(true, window, cx)
                            })),
                    );
            }
            shown = shown.child(
                div()
                    .id("picture-region-note")
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(POPUP_SIZE.0))
                    .v_flex()
                    .gap_1()
                    .p_1()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(border)
                    .bg(card)
                    .cursor_default()
                    // Clicks here are the field's and the buttons', not the
                    // picture's.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .h_flex()
                            .gap_1()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&self.pin_note).xsmall()),
                            )
                            .child(
                                icon_button("picture-pin-add", "pin", "Add note")
                                    .xsmall()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.add_picture_pin(window, cx)
                                    })),
                            ),
                    )
                    .child(actions),
            );
        }
        shown.into_any_element()
    }
}
