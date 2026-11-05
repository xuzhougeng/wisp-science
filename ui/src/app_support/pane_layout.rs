//! Pane-layout domain: the signals that own the sidebar / right pane / center
//! split / composer / terminal dock geometry, and the pointer-drag behaviour
//! that resizes them. `App` constructs `PaneLayoutState` and wires the
//! handlers; persistence goes through the prefs helpers.

use super::*;
use crate::bindings::schedule_chat_follow;
use crate::{
    max_right_pane_width, CENTER_CHAT_MIN_WIDTH, CENTER_DOCUMENT_MIN_WIDTH, PANE_RESIZER_WIDTH,
    RIGHT_PANE_MIN_WIDTH,
};

/// All reactive geometry of the resizable panes. `RwSignal` is `Copy`, so the
/// whole struct is `Copy` and can be captured by every drag closure.
#[derive(Clone, Copy)]
pub(crate) struct PaneLayoutState {
    pub(crate) sidebar_w: RwSignal<f64>,
    pub(crate) sidebar_dragging: RwSignal<bool>,
    pub(crate) sidebar_drag_start_x: RwSignal<f64>,
    pub(crate) sidebar_drag_start_w: RwSignal<f64>,
    pub(crate) right_w: RwSignal<f64>,
    pub(crate) right_dragging: RwSignal<bool>,
    pub(crate) right_drag_start_x: RwSignal<f64>,
    pub(crate) right_drag_start_w: RwSignal<f64>,
    pub(crate) center_chat_w: RwSignal<Option<f64>>,
    pub(crate) center_split_dragging: RwSignal<bool>,
    pub(crate) center_split_drag_start_x: RwSignal<f64>,
    pub(crate) center_split_drag_start_w: RwSignal<f64>,
    pub(crate) composer_h: RwSignal<f64>,
    pub(crate) composer_h_custom: RwSignal<bool>,
    pub(crate) composer_dragging: RwSignal<bool>,
    pub(crate) composer_drag_start_y: RwSignal<f64>,
    pub(crate) composer_drag_start_h: RwSignal<f64>,
    pub(crate) terminal_h: RwSignal<f64>,
    pub(crate) terminal_dragging: RwSignal<bool>,
    pub(crate) terminal_drag_start_y: RwSignal<f64>,
    pub(crate) terminal_drag_start_h: RwSignal<f64>,
}

impl PaneLayoutState {
    pub(crate) fn new() -> Self {
        Self {
            sidebar_w: create_rw_signal(load_sidebar_w()),
            sidebar_dragging: create_rw_signal(false),
            sidebar_drag_start_x: create_rw_signal(0.0_f64),
            sidebar_drag_start_w: create_rw_signal(0.0_f64),
            right_w: create_rw_signal(load_right_pane_w()),
            right_dragging: create_rw_signal(false),
            right_drag_start_x: create_rw_signal(0.0_f64),
            right_drag_start_w: create_rw_signal(0.0_f64),
            center_chat_w: create_rw_signal(None::<f64>),
            center_split_dragging: create_rw_signal(false),
            center_split_drag_start_x: create_rw_signal(0.0_f64),
            center_split_drag_start_w: create_rw_signal(0.0_f64),
            composer_h: create_rw_signal(load_composer_h()),
            composer_h_custom: create_rw_signal(composer_h_custom()),
            composer_dragging: create_rw_signal(false),
            composer_drag_start_y: create_rw_signal(0.0_f64),
            composer_drag_start_h: create_rw_signal(0.0_f64),
            terminal_h: create_rw_signal(320.0_f64),
            terminal_dragging: create_rw_signal(false),
            terminal_drag_start_y: create_rw_signal(0.0_f64),
            terminal_drag_start_h: create_rw_signal(0.0_f64),
        }
    }

    pub(crate) fn sidebar_resize_start(self, ev: web_sys::MouseEvent) {
        ev.prevent_default();
        self.sidebar_dragging.set(true);
        self.sidebar_drag_start_x.set(ev.client_x() as f64);
        self.sidebar_drag_start_w.set(self.sidebar_w.get());
    }

    pub(crate) fn sidebar_resize_move(self, ev: web_sys::MouseEvent) {
        if self.sidebar_dragging.get() {
            let dx = ev.client_x() as f64 - self.sidebar_drag_start_x.get();
            self.sidebar_w
                .set((self.sidebar_drag_start_w.get() + dx).clamp(SIDEBAR_W_MIN, SIDEBAR_W_MAX));
        }
    }

    pub(crate) fn sidebar_resize_end(self) {
        if self.sidebar_dragging.get() {
            save_sidebar_w(self.sidebar_w.get());
            self.sidebar_dragging.set(false);
        }
    }

    pub(crate) fn right_resize_start(self, ev: web_sys::MouseEvent) {
        ev.prevent_default();
        self.right_dragging.set(true);
        self.right_drag_start_x.set(ev.client_x() as f64);
        self.right_drag_start_w.set(self.right_w.get());
    }

    pub(crate) fn right_resize_move(self, ev: web_sys::MouseEvent, sidebar_open: bool) {
        if self.right_dragging.get() {
            let dx = self.right_drag_start_x.get() - ev.client_x() as f64;
            let max_width = max_right_pane_width(sidebar_open, self.sidebar_w.get());
            self.right_w
                .set((self.right_drag_start_w.get() + dx).clamp(RIGHT_PANE_MIN_WIDTH, max_width));
        }
    }

    pub(crate) fn right_resize_end(self) {
        if self.right_dragging.get_untracked() {
            save_right_pane_w(self.right_w.get_untracked());
            self.right_dragging.set(false);
        }
    }

    pub(crate) fn center_split_resize_start(self, ev: web_sys::MouseEvent) {
        ev.prevent_default();
        let width = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| {
                document
                    .query_selector(".center.split > .chat-stage")
                    .ok()
                    .flatten()
            })
            .map(|element| element.get_bounding_client_rect().width())
            .unwrap_or(CENTER_CHAT_MIN_WIDTH);
        self.center_chat_w.set(Some(width));
        self.center_split_drag_start_w.set(width);
        self.center_split_drag_start_x.set(ev.client_x() as f64);
        self.center_split_dragging.set(true);
    }

    pub(crate) fn center_split_resize_move(self, ev: web_sys::MouseEvent) {
        if !self.center_split_dragging.get() {
            return;
        }
        let center_width = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.query_selector(".center.split").ok().flatten())
            .map(|element| element.get_bounding_client_rect().width())
            .unwrap_or(CENTER_CHAT_MIN_WIDTH + CENTER_DOCUMENT_MIN_WIDTH + PANE_RESIZER_WIDTH);
        let max_width = (center_width - CENTER_DOCUMENT_MIN_WIDTH - PANE_RESIZER_WIDTH)
            .max(CENTER_CHAT_MIN_WIDTH);
        let dx = self.center_split_drag_start_x.get() - ev.client_x() as f64;
        self.center_chat_w.set(Some(
            (self.center_split_drag_start_w.get() + dx).clamp(CENTER_CHAT_MIN_WIDTH, max_width),
        ));
    }

    pub(crate) fn composer_resize_start(self, ev: web_sys::MouseEvent) {
        ev.prevent_default();
        self.composer_dragging.set(true);
        self.composer_drag_start_y.set(ev.client_y() as f64);
        self.composer_drag_start_h.set(self.composer_h.get());
    }

    pub(crate) fn composer_resize_move(self, ev: web_sys::MouseEvent) {
        if self.composer_dragging.get() {
            let dy = self.composer_drag_start_y.get() - ev.client_y() as f64;
            self.composer_h
                .set((self.composer_drag_start_h.get() + dy).clamp(COMPOSER_H_MIN, COMPOSER_H_MAX));
            self.composer_h_custom.set(true);
        }
    }

    pub(crate) fn composer_resize_end(self) {
        if self.composer_dragging.get() {
            self.composer_dragging.set(false);
            save_composer_h(self.composer_h.get());
            schedule_chat_follow();
        }
    }

    pub(crate) fn terminal_resize_start(self, ev: web_sys::MouseEvent) {
        ev.prevent_default();
        self.terminal_dragging.set(true);
        self.terminal_drag_start_y.set(ev.client_y() as f64);
        self.terminal_drag_start_h.set(self.terminal_h.get());
    }

    pub(crate) fn terminal_resize_move(self, ev: web_sys::MouseEvent) {
        if self.terminal_dragging.get() {
            let dy = self.terminal_drag_start_y.get() - ev.client_y() as f64;
            let max_h = web_sys::window()
                .and_then(|window| window.inner_height().ok())
                .and_then(|height| height.as_f64())
                .map(|height| (height - 180.0).max(220.0))
                .unwrap_or(720.0);
            self.terminal_h
                .set((self.terminal_drag_start_h.get() + dy).clamp(150.0, max_h));
        }
    }
}
