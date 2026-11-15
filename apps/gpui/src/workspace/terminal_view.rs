//! The terminal dock at the foot of the conversation, after the WebView's
//! terminal panel (xterm.js over the host's PTY): the open project's
//! terminals as tabs, on this computer or a saved SSH host, each drawn from
//! its screen and typed into while it has focus. Hiding the dock keeps them
//! running; closing a tab stops its terminal.
use std::{cell::Cell, collections::HashMap, rc::Rc};

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_dto::native_conversations::TerminalInfo;

use super::{
    Workspace,
    shell::{glyph, icon_button},
};
use crate::{
    backend::receive,
    i18n::tr,
    layers::Layer,
    terminal_screen::{self, Screen},
};

/// The type size and line height terminals are drawn at.
const FONT_SIZE: f32 = 13.;
const LINE_HEIGHT: f32 = 17.;
/// How tall the dock is, its tabs included.
const DOCK_HEIGHT: f32 = 300.;

pub(super) struct TerminalTab {
    info: TerminalInfo,
    screen: Screen,
}

pub(super) struct TerminalDock {
    tabs: Vec<TerminalTab>,
    active: usize,
    pub(super) open: bool,
    /// Whose terminals the tabs are.
    project: Option<String>,
    pub(super) focus: FocusHandle,
    /// The width of one character cell, as measured in the terminal font.
    cell: Rc<Cell<f32>>,
    /// Rows and columns the screens are, so a resize is asked for once.
    size: Rc<Cell<(u16, u16)>>,
    polls: HashMap<String, Task<()>>,
    /// Keys typed and not yet written, by terminal, and whether a write is
    /// under way: one at a time, so they arrive in order.
    typed: HashMap<String, (Vec<u8>, bool)>,
}

impl TerminalDock {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            open: false,
            project: None,
            focus: cx.focus_handle(),
            cell: Rc::new(Cell::new(FONT_SIZE * 0.6)),
            size: Rc::new(Cell::new((16, 100))),
            polls: HashMap::new(),
            typed: HashMap::new(),
        }
    }

    fn tab_mut(&mut self, id: &str) -> Option<&mut TerminalTab> {
        self.tabs.iter_mut().find(|tab| tab.info.id == id)
    }
}

impl Workspace {
    /// Shows or hides the dock. Shown with no terminal of the open project,
    /// it takes up the ones the host keeps, or opens one here.
    pub(super) fn toggle_terminal_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal.open {
            self.terminal.open = false;
            self.layers.close(Layer::TerminalHosts);
            cx.notify();
            return;
        }
        self.terminal.open = true;
        self.show_project_terminals(true, window, cx);
        cx.notify();
    }

    /// The open project's terminals in the dock, as the host keeps them;
    /// with `or_open`, a new one when it keeps none.
    fn show_project_terminals(
        &mut self,
        or_open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        if self.terminal.project.as_ref() == Some(&project) && !self.terminal.tabs.is_empty() {
            return;
        }
        self.terminal.project = Some(project.clone());
        self.terminal.tabs.clear();
        self.terminal.polls.clear();
        self.terminal.typed.clear();
        let reply = self.backend.terminals(project.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.terminal.project.as_ref() != Some(&project) {
                    return;
                }
                match result {
                    Ok(open) if open.is_empty() && or_open => {
                        this.open_terminal_in(None, None, window, cx)
                    }
                    Ok(open) => {
                        let (rows, cols) = this.terminal.size.get();
                        for info in open {
                            let id = info.id.clone();
                            this.terminal.tabs.push(TerminalTab {
                                info,
                                screen: Screen::new(rows, cols),
                            });
                            this.poll_terminal(project.clone(), id, window, cx);
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens a terminal in the dock: this computer's shell (in `folder` of
    /// the project, when given), or one on an environment such as
    /// `ssh:alias` (in `folder` there).
    pub(super) fn open_terminal_in(
        &mut self,
        context: Option<String>,
        folder: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self
            .state
            .project
            .clone()
            .or_else(|| self.projects.first().map(|project| project.id.clone()))
        else {
            self.error = Some(tr("Open a project to open a terminal in it.").into());
            cx.notify();
            return;
        };
        self.layers.close(Layer::TerminalHosts);
        self.terminal.open = true;
        if self.terminal.project.as_ref() != Some(&project) {
            self.terminal.project = Some(project.clone());
            self.terminal.tabs.clear();
            self.terminal.polls.clear();
            self.terminal.typed.clear();
        }
        let reply = self.backend.open_terminal(project.clone(), context, folder);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(info) => {
                        let id = info.id.clone();
                        let (rows, cols) = this.terminal.size.get();
                        this.terminal.tabs.push(TerminalTab {
                            info,
                            screen: Screen::new(rows, cols),
                        });
                        this.terminal.active = this.terminal.tabs.len() - 1;
                        // The host starts it at its own size.
                        let resize =
                            this.backend
                                .resize_terminal(project.clone(), id.clone(), rows, cols);
                        cx.background_executor()
                            .spawn(async move {
                                let _ = receive(resize).await;
                            })
                            .detach();
                        this.poll_terminal(project.clone(), id, window, cx);
                        this.terminal.focus.focus(window);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Reads a terminal's output as it comes, until it ends or its tab is
    /// closed.
    fn poll_terminal(
        &mut self,
        project: String,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let watched = id.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            loop {
                let Ok(Some((reply, _))) = this.update(cx, |this, _| {
                    let cursor = this.terminal.tab_mut(&id)?.screen.cursor;
                    Some((
                        this.backend
                            .read_terminal(project.clone(), id.clone(), cursor),
                        (),
                    ))
                }) else {
                    break;
                };
                let result = receive(reply).await;
                let keep = this
                    .update(cx, |this, cx| {
                        let Some(tab) = this.terminal.tab_mut(&id) else {
                            return false;
                        };
                        let going = match result {
                            Ok(output) => {
                                tab.screen.apply(&output);
                                output.exit_code.is_none()
                            }
                            Err(_) => false,
                        };
                        tab.info.running = going;
                        cx.notify();
                        going
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        });
        self.terminal.polls.insert(watched, task);
    }

    /// Sends `bytes` to the active terminal, after anything typed before.
    fn type_into_terminal(&mut self, bytes: Vec<u8>, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(tab)) = (
            self.terminal.project.clone(),
            self.terminal.tabs.get_mut(self.terminal.active),
        ) else {
            return;
        };
        if !tab.info.running {
            return;
        }
        // Typing brings the live screen back.
        tab.screen.follow();
        let id = tab.info.id.clone();
        self.terminal
            .typed
            .entry(id.clone())
            .or_default()
            .0
            .extend(bytes);
        self.flush_typed(project, id, window, cx);
    }

    fn flush_typed(
        &mut self,
        project: String,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((waiting, writing)) = self.terminal.typed.get_mut(&id) else {
            return;
        };
        if *writing || waiting.is_empty() {
            return;
        }
        *writing = true;
        let bytes = std::mem::take(waiting);
        let reply = self
            .backend
            .write_terminal(project.clone(), id.clone(), bytes);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Some((_, writing)) = this.terminal.typed.get_mut(&id) {
                    *writing = false;
                }
                if result.is_ok() {
                    this.flush_typed(project, id, window, cx);
                }
            });
        })
        .detach();
    }

    /// A key pressed while a terminal has focus.
    fn terminal_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.terminal.tabs.get(self.terminal.active) else {
            return;
        };
        let stroke = &event.keystroke;
        let held = stroke.modifiers;
        let mac = cfg!(target_os = "macos");
        // Paste and copy: Cmd on macOS, Ctrl+Shift elsewhere, as terminals
        // there do.
        let chord = |letter: &str| {
            stroke.key == letter
                && if mac {
                    held.platform && !held.control
                } else {
                    held.control && held.shift
                }
        };
        if chord("v") {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                let bytes = terminal_screen::paste_bytes(&text, tab.screen.bracketed_paste());
                self.type_into_terminal(bytes, window, cx);
            }
            cx.stop_propagation();
            return;
        }
        if chord("c") {
            cx.write_to_clipboard(ClipboardItem::new_string(tab.screen.text()));
            cx.stop_propagation();
            return;
        }
        // The window's own shortcuts.
        if held.platform {
            return;
        }
        let key = terminal_screen::Key {
            name: &stroke.key,
            typed: stroke.key_char.as_deref(),
            control: held.control,
            alt: held.alt,
            shift: held.shift,
        };
        if let Some(bytes) = terminal_screen::key_bytes(key, tab.screen.application_cursor()) {
            self.type_into_terminal(bytes, window, cx);
            cx.stop_propagation();
        }
    }

    /// Gives every terminal of the dock `rows` by `cols`.
    fn resize_terminals(&mut self, rows: u16, cols: u16, cx: &mut Context<Self>) {
        self.terminal.size.set((rows, cols));
        let Some(project) = self.terminal.project.clone() else {
            return;
        };
        for tab in &mut self.terminal.tabs {
            if tab.screen.resize(rows, cols) && tab.info.running {
                let reply =
                    self.backend
                        .resize_terminal(project.clone(), tab.info.id.clone(), rows, cols);
                cx.background_executor()
                    .spawn(async move {
                        let _ = receive(reply).await;
                    })
                    .detach();
            }
        }
        cx.notify();
    }

    /// Stops a terminal and takes its tab away.
    fn close_terminal_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.terminal.tabs.len() {
            return;
        }
        let tab = self.terminal.tabs.remove(index);
        self.terminal.polls.remove(&tab.info.id);
        self.terminal.typed.remove(&tab.info.id);
        if let Some(project) = self.terminal.project.clone() {
            let reply = self.backend.close_terminal(project, tab.info.id);
            cx.spawn_in(window, async move |this, cx| {
                if let Err(error) = receive(reply).await {
                    let _ = this.update(cx, |this, cx| {
                        this.error = Some(error);
                        cx.notify();
                    });
                }
            })
            .detach();
        }
        self.terminal.active = self
            .terminal
            .active
            .min(self.terminal.tabs.len().saturating_sub(1));
        cx.notify();
    }

    /// The saved SSH hosts a terminal can be opened on.
    fn terminal_hosts_menu(&self, cx: &mut Context<Self>) -> Div {
        let mut menu = div()
            .v_flex()
            .min_w(px(200.))
            .p_1()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_md();
        let hosts = self
            .environments
            .as_ref()
            .map(|environments| environments.hosts.as_slice())
            .unwrap_or_default();
        if hosts.is_empty() {
            menu = menu.child(
                div()
                    .p_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr(
                        "No SSH host is saved; add one in Settings, Environments.",
                    )),
            );
        }
        for (index, host) in hosts.iter().enumerate() {
            let context = format!("ssh:{}", host.alias);
            menu = menu.child(
                Button::new(("terminal-host", index))
                    .label(host.alias.clone())
                    .icon(glyph("server"))
                    .small()
                    .ghost()
                    .w_full()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_terminal_in(Some(context.clone()), None, window, cx)
                    })),
            );
        }
        menu
    }

    /// The dock, when it is shown.
    pub(super) fn render_terminal_dock(&self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.terminal.open {
            return None;
        }
        let theme = cx.theme();
        let (muted, border, foreground, background) = (
            theme.muted_foreground,
            theme.border,
            theme.foreground,
            theme.background,
        );
        let mono = theme.mono_font_family.clone();
        let showing = self.terminal.project == self.state.project;
        let mut tabs = div()
            .h_flex()
            .gap_1()
            .flex_1()
            .min_w_0()
            .overflow_x_hidden();
        if showing {
            for (index, tab) in self.terminal.tabs.iter().enumerate() {
                let label = if tab.info.running {
                    tab.info.title.clone()
                } else {
                    crate::i18n::fill(tr("{} (ended)"), &[&tab.info.title])
                };
                tabs = tabs.child(
                    div()
                        .h_flex()
                        .child(
                            Button::new(("terminal-tab", index))
                                .label(label)
                                .icon(glyph("terminal"))
                                .xsmall()
                                .ghost()
                                .selected(index == self.terminal.active)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.terminal.active = index;
                                    this.terminal.focus.focus(window);
                                    cx.notify();
                                })),
                        )
                        .child(
                            icon_button(("terminal-close", index), "close", "Close this terminal")
                                .xsmall()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.close_terminal_tab(index, window, cx)
                                })),
                        ),
                );
            }
        }
        let hosts_open = self.layers.contains(Layer::TerminalHosts);
        let mut host_button = div().relative().child(
            icon_button(
                "terminal-on-host",
                "server",
                "Open a terminal on an SSH host",
            )
            .xsmall()
            .selected(hosts_open)
            .on_click(cx.listener(|this, _, window, cx| {
                if !this.layers.close(Layer::TerminalHosts) {
                    // Escape then closes the menu, not the shell's.
                    window.blur();
                    this.load_environments(window, cx);
                    this.layers.open(Layer::TerminalHosts);
                }
                cx.notify();
            })),
        );
        if hosts_open {
            // Above the button: the dock is at the window's foot.
            host_button = host_button.child(deferred(
                self.terminal_hosts_menu(cx)
                    .absolute()
                    .bottom(px(28.))
                    .right_0(),
            ));
        }
        let header = div()
            .h_flex()
            .gap_1()
            .px_2()
            .h(px(32.))
            .flex_shrink_0()
            .border_b_1()
            .border_color(border)
            .child(tabs)
            .child(
                icon_button("terminal-new", "plus", "Open a terminal on this computer")
                    .xsmall()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_terminal_in(None, None, window, cx)
                    })),
            )
            .child(host_button)
            .child(
                icon_button(
                    "terminal-system",
                    "external-link",
                    "Open the system's terminal in the project's folder",
                )
                .xsmall()
                .on_click(cx.listener(|this, _, _, cx| this.open_terminal(cx))),
            )
            .child(
                icon_button(
                    "terminal-hide",
                    "chevron-down",
                    "Hide the terminals; they keep running",
                )
                .xsmall()
                .on_click(cx.listener(|this, _, window, cx| this.toggle_terminal_dock(window, cx))),
            );
        let tab = self
            .terminal
            .tabs
            .get(self.terminal.active)
            .filter(|_| showing);
        let mut body = div()
            .id("terminal-screen")
            .relative()
            .flex_1()
            .min_h_0()
            .px_2()
            .py_1()
            .overflow_hidden()
            .font_family(mono.clone())
            .text_size(px(FONT_SIZE))
            .line_height(px(LINE_HEIGHT))
            .track_focus(&self.terminal.focus)
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.terminal.focus.focus(window);
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.terminal_key(event, window, cx)
            }))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                let delta = event.delta.pixel_delta(px(LINE_HEIGHT));
                let lines = (f32::from(delta.y) / LINE_HEIGHT).round() as i32;
                let active = this.terminal.active;
                if let Some(tab) = this.terminal.tabs.get_mut(active) {
                    tab.screen.scroll(lines);
                    cx.notify();
                }
            }));
        // Measures the font's cell and the room there is, and asks for the
        // size that fits.
        let (cell, size) = (self.terminal.cell.clone(), self.terminal.size.clone());
        let weak = cx.entity().downgrade();
        let measure_font = font(mono);
        body = body.child(
            canvas(
                move |bounds, window, cx| {
                    let system = window.text_system();
                    let id = system.resolve_font(&measure_font);
                    if let Ok(advance) = system.advance(id, px(FONT_SIZE), 'M') {
                        cell.set(f32::from(advance.width).max(1.));
                    }
                    let cols = (f32::from(bounds.size.width) / cell.get()).floor() as u16;
                    let rows = (f32::from(bounds.size.height) / LINE_HEIGHT).floor() as u16;
                    let wanted = (rows.clamp(2, 500), cols.clamp(2, 1000));
                    if wanted != size.get() {
                        size.set(wanted);
                        window.defer(cx, move |_, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.resize_terminals(wanted.0, wanted.1, cx)
                            });
                        });
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
        match tab {
            None => {
                body = body.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(tr("No terminal is open. + opens one on this computer.")),
                );
            }
            Some(tab) => {
                let default_fg = rgb_of(foreground);
                let default_bg = rgb_of(background);
                for (number, runs) in tab.screen.rows().into_iter().enumerate() {
                    let mut text = String::new();
                    let mut highlights = Vec::new();
                    for run in runs {
                        let start = text.len();
                        text.push_str(&run.text);
                        let style = run.style;
                        let (mut fg, mut bg) = (
                            terminal_screen::rgb(style.fg, default_fg),
                            (style.bg != vt100::Color::Default)
                                .then(|| terminal_screen::rgb(style.bg, default_bg)),
                        );
                        if style.inverse {
                            let swapped = bg.unwrap_or(default_bg);
                            bg = Some(fg);
                            fg = swapped;
                        }
                        highlights.push((
                            start..text.len(),
                            HighlightStyle {
                                color: Some(colour(fg)),
                                background_color: bg.map(colour),
                                font_weight: style.bold.then_some(FontWeight::BOLD),
                                font_style: style.italic.then_some(FontStyle::Italic),
                                underline: style.underline.then(|| UnderlineStyle {
                                    thickness: px(1.),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            },
                        ));
                    }
                    body = body.child(
                        div()
                            .id(("terminal-row", number))
                            .h(px(LINE_HEIGHT))
                            .whitespace_nowrap()
                            .child(StyledText::new(text).with_highlights(highlights)),
                    );
                }
                if let Some((row, col)) = tab.screen.cursor_at() {
                    let width = self.terminal.cell.get();
                    body = body.child(
                        div()
                            .absolute()
                            // The body's padding: 8 across, 4 down.
                            .left(px(8. + f32::from(col) * width))
                            .top(px(4. + f32::from(row) * LINE_HEIGHT))
                            .w(px(width))
                            .h(px(LINE_HEIGHT))
                            .bg(foreground.opacity(0.45)),
                    );
                }
                if !tab.info.running {
                    body = body.child(div().text_xs().text_color(muted).child(crate::i18n::fill(
                        tr("The terminal ended (exit code {})."),
                        &[&tab.screen.exit.unwrap_or_default()],
                    )));
                }
            }
        }
        Some(
            div()
                .v_flex()
                .h(px(DOCK_HEIGHT))
                .flex_shrink_0()
                .border_t_1()
                .border_color(border)
                .bg(background)
                .child(header)
                .child(body),
        )
    }
}

fn rgb_of(colour: Hsla) -> (u8, u8, u8) {
    let rgba = colour.to_rgb();
    (
        (rgba.r * 255.).round() as u8,
        (rgba.g * 255.).round() as u8,
        (rgba.b * 255.).round() as u8,
    )
}

fn colour((r, g, b): (u8, u8, u8)) -> Hsla {
    Rgba {
        r: f32::from(r) / 255.,
        g: f32::from(g) / 255.,
        b: f32::from(b) / 255.,
        a: 1.,
    }
    .into()
}
