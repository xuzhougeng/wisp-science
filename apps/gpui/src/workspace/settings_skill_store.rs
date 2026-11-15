//! Getting skills on the Skills page of Settings: the community directory
//! and a GitHub address, each previewed as the packages found at an exact
//! commit and installed into your own skills folder. The address form is a
//! layer inside Settings.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_dto::{CommunitySkillEntry, SkillStoreCandidate};

use super::{Workspace, chip, shell::error_banner};
use crate::{backend::receive, layers::Layer, present::Tone};

impl Workspace {
    /// The project skills are looked at for: the open one, or the one the
    /// host was started on.
    fn store_project(&self) -> String {
        self.state
            .project
            .clone()
            .unwrap_or_else(|| wisp_server::desktop::BUILT_IN_PROJECT.to_owned())
    }

    pub(super) fn load_community_skills(
        &mut self,
        refresh: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.skill_store_busy = refresh;
        let reply = self.backend.community_skills(refresh);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                this.skill_store_busy = false;
                match result {
                    Ok(catalog) => this.community_skills = Some(catalog),
                    Err(error) => this.skill_store_note = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Reads the packages at `address`, to be installed from the list.
    fn preview_skills(
        &mut self,
        address: String,
        exact_ref: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.skill_store_busy {
            return;
        }
        self.skill_store_busy = true;
        self.skill_store_note = None;
        self.skill_candidates.clear();
        let reply = self
            .backend
            .preview_github_skills(self.store_project(), address, exact_ref);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                this.skill_store_busy = false;
                match result {
                    Ok(found) if found.is_empty() => {
                        this.skill_store_note = Some(tr("No skill package was found there.").into())
                    }
                    Ok(found) => this.skill_candidates = found,
                    Err(error) => this.skill_store_note = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn install_skill(
        &mut self,
        candidate: SkillStoreCandidate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.skill_store_busy {
            return;
        }
        self.skill_store_busy = true;
        self.skill_store_note = None;
        let reply = self
            .backend
            .install_github_skill(self.store_project(), candidate.source);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.skill_store_busy = false;
                match result {
                    Ok(installed) => {
                        this.skill_candidates.clear();
                        this.layers.close(Layer::SkillFromGithub);
                        this.skill_store_note = Some(crate::i18n::fill(
                            tr("{} is installed in {}; every project has it from its next turn."),
                            &[&installed.name, &installed.directory],
                        ));
                        this.load_skills(window, cx);
                    }
                    Err(error) => this.skill_store_note = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// One entry of the community directory.
    fn community_row(
        &self,
        index: usize,
        entry: &CommunitySkillEntry,
        cx: &mut Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let installed = self
            .skills
            .iter()
            .flatten()
            .any(|skill| skill.name.eq_ignore_ascii_case(&entry.name));
        let address = format!(
            "https://github.com/{}/tree/{}/{}",
            entry.repository, entry.git_ref, entry.package_path
        );
        let exact = entry.git_ref.clone();
        let about = div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .gap_0p5()
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(entry.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(entry.author.clone()),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(entry.description.clone()),
            );
        let action = if installed {
            chip(tr("Installed"), Tone::Success, cx).into_any_element()
        } else {
            Button::new(("community-get", index))
                .label(tr("Look at it"))
                .small()
                .disabled(self.skill_store_busy)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.preview_skills(address.clone(), exact.clone(), window, cx)
                }))
                .into_any_element()
        };
        div()
            .h_flex()
            .items_start()
            .gap_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(about)
            .child(div().flex_shrink_0().child(action))
    }

    /// A package found at an address, to install or not.
    fn candidate_row(
        &self,
        index: usize,
        candidate: &SkillStoreCandidate,
        cx: &mut Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let problems: Vec<String> = candidate
            .format_errors
            .iter()
            .chain(&candidate.resource_errors)
            .cloned()
            .collect();
        let blocked = candidate.conflict.is_some() || !problems.is_empty();
        let mut about = div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .gap_0p5()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(candidate.name.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(candidate.description.clone()),
            )
            .child(div().text_xs().text_color(muted).child(crate::i18n::fill(
                tr("{} at {}, in {}"),
                &[
                    &candidate.source.repository,
                    &candidate.source.commit.chars().take(12).collect::<String>(),
                    &if candidate.source.package_path.is_empty() {
                        "/".to_owned()
                    } else {
                        candidate.source.package_path.clone()
                    },
                ],
            )));
        for said in candidate
            .conflict
            .iter()
            .chain(&problems)
            .chain(&candidate.warnings)
        {
            about = about.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child(said.clone()),
            );
        }
        let picked = candidate.clone();
        div()
            .h_flex()
            .items_start()
            .gap_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(about)
            .child(
                Button::new(("candidate-install", index))
                    .label(tr("Install"))
                    .small()
                    .primary()
                    .disabled(blocked || self.skill_store_busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.install_skill(picked.clone(), window, cx)
                    })),
            )
    }

    fn github_skill_form(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        div()
            .v_flex()
            .gap_2()
            .p_3()
            .w(px(620.))
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(div().text_xs().text_color(muted).child(tr(
                "A GitHub repository, or a folder in one, and the branch, tag or commit if the \
                 address does not say. Nothing downloaded is run.",
            )))
            .child(Input::new(&self.github_skill_fields[0]).small())
            .child(
                div()
                    .w(px(240.))
                    .child(Input::new(&self.github_skill_fields[1]).small()),
            )
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new("github-skill-look")
                            .label(tr("Look for skills"))
                            .small()
                            .primary()
                            .disabled(self.skill_store_busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                let [address, exact] = [0, 1].map(|at| {
                                    this.github_skill_fields[at].read(cx).value().to_string()
                                });
                                this.preview_skills(address, exact, window, cx)
                            })),
                    )
                    .child(
                        Button::new("github-skill-cancel")
                            .label(tr("Cancel"))
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::SkillFromGithub);
                                cx.notify();
                            })),
                    ),
            )
    }

    /// Below the installed skills: getting more.
    pub(super) fn skill_store_section(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut section = div()
            .v_flex()
            .gap_2()
            .pt_6()
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(tr("Get skills")),
                    )
                    .child(
                        Button::new("community-refresh")
                            .label(if self.skill_store_busy {
                                tr("Working…")
                            } else {
                                tr("Refresh the directory")
                            })
                            .small()
                            .ghost()
                            .disabled(self.skill_store_busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.load_community_skills(true, window, cx)
                            })),
                    )
                    .child(
                        Button::new("github-skill")
                            .label(tr("From GitHub…"))
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                if !this.layers.close(Layer::SkillFromGithub) {
                                    this.layers.open(Layer::SkillFromGithub);
                                    this.github_skill_fields[0]
                                        .update(cx, |input, cx| input.focus(window, cx));
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(div().w(px(640.)).text_xs().text_color(muted).child(tr(
                "Skills others have published, installed into your own skills folder \
                 (~/.wisp/skills), where every project can use them. A name that is taken is \
                 never replaced.",
            )));
        if self.layers.contains(Layer::SkillFromGithub) {
            section = section.child(self.github_skill_form(cx));
        }
        if let Some(note) = &self.skill_store_note {
            section = section.child(error_banner(note, cx));
        }
        for (index, candidate) in self.skill_candidates.iter().enumerate() {
            section = section.child(self.candidate_row(index, candidate, cx));
        }
        match &self.community_skills {
            None => section.child(div().text_xs().text_color(muted).child(tr("Loading…"))),
            Some(catalog) => {
                if let Some(notice) = &catalog.notice {
                    section =
                        section.child(div().text_xs().text_color(muted).child(notice.clone()));
                }
                for (index, entry) in catalog.entries.iter().enumerate() {
                    section = section.child(self.community_row(index, entry, cx));
                }
                section
            }
        }
    }
}
