//! The example conversations that ship with Wisp: a dialog over the home
//! page that lists them, shows the chosen one read only as it was recorded,
//! and copies it into a project as a conversation to go on with.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    text::TextView,
};
use wisp_server::desktop::{Demo, DemoRole};

use super::{
    Workspace, chip,
    shell::{error_banner, glyph},
};
use crate::{backend::receive, demos, layers::Layer, present::Tone};

impl Workspace {
    /// Opens the examples and reads the first one.
    pub(super) fn open_examples(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::Palette);
        self.close_examples();
        self.layers.open(Layer::Examples);
        let reply = self.backend.demos();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.layers.contains(Layer::Examples) {
                    return;
                }
                match result {
                    Ok(found) if found.is_empty() => {
                        this.demo_note =
                            Some(tr("This copy of Wisp has no example conversations.").into())
                    }
                    Ok(found) => {
                        let first = found[0].id.clone();
                        this.demos = found;
                        this.read_example(first, window, cx);
                    }
                    Err(error) => this.demo_note = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Forgets what the dialog was showing, once its layer is gone.
    pub(super) fn close_examples(&mut self) {
        self.demo = None;
        self.demo_open.clear();
        self.demo_note = None;
        self.demo_list.reset(0);
    }

    fn read_example(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::ExampleCopy);
        self.demo_note = None;
        self.demo_open.clear();
        self.demo_list.reset(0);
        // The one chosen shows as chosen while it is read, and a reply for
        // one chosen before it is not shown.
        self.demo = Some(Demo {
            id: id.clone(),
            title: String::new(),
            rows: Vec::new(),
        });
        let reply = self.backend.demo(id.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                let chosen = this.demo.as_ref().map(|demo| demo.id.as_str());
                if !this.layers.contains(Layer::Examples) || chosen != Some(id.as_str()) {
                    return;
                }
                match result {
                    Ok(demo) => {
                        this.demo_list.reset(demo.rows.len());
                        this.demo = Some(demo);
                    }
                    Err(error) => this.demo_note = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Copies the example being read into `project` and opens it there.
    fn copy_example(
        &mut self,
        project: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.demo.as_ref().map(|demo| demo.id.clone()) else {
            return;
        };
        if self.pending {
            return;
        }
        self.pending = true;
        self.demo_note = None;
        self.layers.close(Layer::ExampleCopy);
        let reply = self.backend.copy_demo(id, project.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(session) => {
                        this.layers.close(Layer::Examples);
                        this.close_examples();
                        this.notice = Some(crate::i18n::fill(
                            tr("The example was copied into {}, to go on with there."),
                            &[&name],
                        ));
                        this.load_projects(window, cx);
                        this.open_session(project, session, window, cx);
                    }
                    Err(error) => this.demo_note = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// One row of the example being read, as the list draws it.
    fn example_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(demo) = self.demo.as_ref() else {
            return div().into_any_element();
        };
        let Some(row) = demo.rows.get(index) else {
            return div().into_any_element();
        };
        let muted = cx.theme().muted_foreground;
        if !demos::folded(row) {
            let id = SharedString::from(format!("demo-{}-{index}", demo.id));
            let you = row.role == DemoRole::You;
            let mut head = div().h_flex().gap_2().text_xs().text_color(muted).child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .child(demos::label(row)),
            );
            if let Some(model) = &row.model {
                head = head.child(model.clone());
            }
            let body = div()
                .px_3()
                .py_2()
                .rounded(cx.theme().radius_lg)
                .when(you, |body| body.bg(cx.theme().muted))
                .child(TextView::markdown(id, row.text.clone(), window, cx).selectable(true));
            return div()
                .v_flex()
                .gap_1()
                .px_4()
                .py_1p5()
                .child(head)
                .child(body)
                .into_any_element();
        }
        let opens = demos::opens(row);
        let open = opens && self.demo_open.contains(&index);
        let failed = row.ok == Some(false);
        let mut line = div()
            .id(("demo-row", index))
            .h_flex()
            .items_start()
            .gap_2()
            .px_1()
            .py_0p5()
            .rounded(cx.theme().radius)
            .text_xs()
            .child(
                div()
                    .w(px(14.))
                    .pt_0p5()
                    .flex_shrink_0()
                    .children(opens.then(|| {
                        glyph(if open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        })
                        .xsmall()
                        .text_color(muted)
                    })),
            )
            .child(
                div()
                    .w(px(120.))
                    .flex_shrink_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(demos::label(row)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(muted)
                    .child(demos::line(row)),
            );
        if failed {
            line = line.child(chip(tr("Failed"), Tone::Danger, cx));
        }
        if opens {
            line = line
                .cursor_pointer()
                .hover(|line| line.bg(cx.theme().accent))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.demo_open.remove(&index) {
                        this.demo_open.insert(index);
                    }
                    // The row changes height.
                    this.demo_list.splice(index..index + 1, 1);
                    cx.notify();
                }));
        }
        let mut block = div().px_4().py_0p5().child(line);
        if open {
            block = block.child(
                div()
                    .ml(px(22.))
                    .mt_1()
                    .p_2()
                    .rounded(cx.theme().radius)
                    .bg(cx.theme().muted)
                    .text_xs()
                    .font_family(cx.theme().mono_font_family.clone())
                    .child(row.text.clone()),
            );
        }
        block.into_any_element()
    }

    /// The projects the example can be copied into, under its button.
    fn example_copy_menu(&self, cx: &mut Context<Self>) -> Div {
        let menu = div()
            .absolute()
            .top(px(30.))
            .right_0()
            .w(px(280.))
            .max_h(px(320.))
            .v_flex()
            .p_1()
            .occlude()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_md();
        let targets = demos::targets(&self.projects);
        if targets.is_empty() {
            return menu.child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("Make a project first, then copy the example into it.")),
            );
        }
        let mut list = div().id("demo-targets").v_flex().overflow_y_scroll();
        for project in targets {
            let (id, name) = (project.id.clone(), project.name.clone());
            list = list.child(
                Self::menu_row(
                    SharedString::from(format!("demo-target-{id}")),
                    project.name.clone(),
                    project.workspace_dir.clone(),
                    false,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.copy_example(id.clone(), name.clone(), window, cx)
                })),
            );
        }
        menu.child(list)
    }

    pub(super) fn render_examples(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let chosen = self.demo.as_ref().map(|demo| demo.id.as_str());
        let mut side = div()
            .id("demo-list")
            .v_flex()
            .gap_0p5()
            .w(px(250.))
            .h_full()
            .flex_shrink_0()
            .pr_3()
            .border_r_1()
            .border_color(border)
            .overflow_y_scroll();
        for (index, info) in self.demos.iter().enumerate() {
            let id = info.id.clone();
            let selected = chosen == Some(info.id.as_str());
            side = side.child(
                div()
                    .id(("demo-info", index))
                    .px_2()
                    .py_1p5()
                    .rounded(cx.theme().radius)
                    .text_sm()
                    .cursor_pointer()
                    .when(selected, |row| row.bg(cx.theme().accent))
                    .hover(|row| row.bg(cx.theme().list_hover))
                    .child(info.title.clone())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.read_example(id.clone(), window, cx)
                    })),
            );
        }
        let weak = cx.entity().downgrade();
        let mut reading = div().v_flex().flex_1().min_w_0().h_full().gap_2();
        if let Some(demo) = &self.demo {
            let title = self
                .demos
                .iter()
                .find(|info| info.id == demo.id)
                .map(|info| info.title.clone())
                .unwrap_or_else(|| demo.title.clone());
            let copying = self.layers.contains(Layer::ExampleCopy);
            let mut copy = div().relative().flex_shrink_0().child(
                Button::new("demo-copy")
                    .icon(glyph("copy"))
                    .label(tr("Copy to a project"))
                    .small()
                    .selected(copying)
                    .disabled(self.pending || demo.rows.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if !this.layers.close(Layer::ExampleCopy) {
                            this.layers.open(Layer::ExampleCopy);
                        }
                        cx.notify();
                    })),
            );
            if copying {
                copy = copy.child(deferred(self.example_copy_menu(cx)));
            }
            reading = reading
                .child(
                    div()
                        .h_flex()
                        .items_start()
                        .gap_3()
                        .px_4()
                        .child(
                            div()
                                .v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_0p5()
                                .child(div().font_weight(FontWeight::MEDIUM).child(title))
                                .child(div().text_xs().text_color(muted).child(
                                    if demo.rows.is_empty() {
                                        if self.demo_note.is_some() {
                                            String::new()
                                        } else {
                                            tr("Reading…").into()
                                        }
                                    } else {
                                        demos::summary(&demo.rows)
                                    },
                                )),
                        )
                        .child(copy),
                )
                .child(
                    list(self.demo_list.clone(), move |index, window, cx| {
                        weak.update(cx, |this, cx| this.example_row(index, window, cx))
                            .unwrap_or_else(|_| div().into_any_element())
                    })
                    .flex_1()
                    .min_h_0(),
                );
        } else if self.demo_note.is_none() {
            reading = reading.child(
                div()
                    .px_4()
                    .text_sm()
                    .text_color(muted)
                    .child(tr("Reading…")),
            );
        }
        let mut card = div()
            .id("examples")
            .v_flex()
            .gap_3()
            .w(px(1000.))
            .h(px(680.))
            .max_w(relative(0.94))
            .max_h(relative(0.9))
            .p_5()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(border)
            .bg(cx.theme().popover)
            .shadow_lg()
            // A click inside is not a click on the backdrop.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_lg()
                            .font_weight(FontWeight::MEDIUM)
                            .child(tr("Example conversations")),
                    )
                    .child(
                        Button::new("examples-close")
                            .label(tr("Close"))
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::Examples);
                                this.close_examples();
                                cx.notify();
                            })),
                    ),
            )
            .child(div().text_xs().text_color(muted).child(tr(
                "Research recorded with Wisp, to read before you start. Copying one into a project adds it as a conversation you can go on with, with the files it refers to; files already there are left as they are.",
            )));
        if let Some(note) = &self.demo_note {
            card = card.child(error_banner(note, cx));
        }
        card = card.child(
            div()
                .h_flex()
                .items_start()
                .flex_1()
                .min_h_0()
                .child(side)
                .child(reading),
        );
        div()
            .id("examples-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(cx.theme().overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.layers.close(Layer::Examples);
                    this.close_examples();
                    cx.notify();
                }),
            )
            .child(card)
    }
}
