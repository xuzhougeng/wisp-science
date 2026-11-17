//! Getting skills on the Skills page of Settings: the community directory
//! and a GitHub address, each previewed as the packages found at an exact
//! commit and installed into your own skills folder. The address form is a
//! layer inside Settings.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_dto::{CommunitySkillEntry, SkillStoreCandidate};

use super::{
    Workspace,
    settings_sections::setting,
    shell::error_banner,
    surfaces::{self, RecordRow},
};
use crate::{backend::receive, layers::Layer};

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

    /// One entry of the community directory, as a record row: its name and
    /// author, what it does under it, and either a stamp that it is
    /// installed or the way to look at it.
    fn community_row(
        &self,
        index: usize,
        entry: &CommunitySkillEntry,
        cx: &mut Context<Self>,
    ) -> RecordRow {
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
        let row = surfaces::record_row(("community-skill", index), entry.name.clone())
            .icon("skill")
            .suffix(entry.author.clone())
            .below((!entry.description.is_empty()).then(|| {
                surfaces::settings_note(entry.description.clone(), cx).into_any_element()
            }));
        if installed {
            row.stamp(surfaces::StampKind::Success, tr("Installed"))
        } else {
            row.action(
                surfaces::outlined_action(("community-get", index), "eye", tr("Look at it"), cx)
                    .disabled(self.skill_store_busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.preview_skills(address.clone(), exact.clone(), window, cx)
                    })),
            )
        }
    }

    /// A package found at an address, to install or not: a record row with
    /// where it is from and what stands in its way under it.
    fn candidate_row(
        &self,
        index: usize,
        candidate: &SkillStoreCandidate,
        cx: &mut Context<Self>,
    ) -> RecordRow {
        let problems: Vec<String> = candidate
            .format_errors
            .iter()
            .chain(&candidate.resource_errors)
            .cloned()
            .collect();
        let blocked = candidate.conflict.is_some() || !problems.is_empty();
        let mut about = div()
            .v_flex()
            .gap_0p5()
            .child(surfaces::settings_note(candidate.description.clone(), cx))
            .child(surfaces::fact(
                crate::i18n::fill(
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
                ),
                cx,
            ));
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
        surfaces::record_row(("skill-candidate", index), candidate.name.clone())
            .icon("skill")
            .below(Some(about.into_any_element()))
            .action(
                surfaces::outlined_action(
                    ("candidate-install", index),
                    "download",
                    tr("Install"),
                    cx,
                )
                .disabled(blocked || self.skill_store_busy)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.install_skill(picked.clone(), window, cx)
                })),
            )
    }

    /// The address to look for skills at, as setting rows.
    fn github_skill_form(&self, cx: &mut Context<Self>) -> Div {
        div()
            .v_flex()
            .child(surfaces::settings_note_help(
                tr("A GitHub repository or a folder in one."),
                tr(
                    "A GitHub repository, or a folder in one, and the branch, tag or commit if the \
                     address does not say. Nothing downloaded is run.",
                ),
                cx,
            ))
            .child(setting(
                tr("Address"),
                "",
                div()
                    .w(px(300.))
                    .child(surfaces::input(&self.github_skill_fields[0], cx).small()),
                cx,
            ))
            .child(setting(
                tr("Branch, tag or commit"),
                "",
                div()
                    .w(px(200.))
                    .child(surfaces::input(&self.github_skill_fields[1], cx).small()),
                cx,
            ))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .pt_3()
                    .child(
                        surfaces::outlined_action(
                            "github-skill-look",
                            "search",
                            tr("Look for skills"),
                            cx,
                        )
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
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::SkillFromGithub);
                                cx.notify();
                            })),
                    ),
            )
    }

    /// Below the installed skills: getting more.
    pub(super) fn skill_store_section(&self, cx: &mut Context<Self>) -> Div {
        let from_github = self.layers.contains(Layer::SkillFromGithub);
        let mut section = div()
            .v_flex()
            .gap_2()
            .child(
                div()
                    .h_flex()
                    .items_end()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(surfaces::settings_heading(tr("Get skills"), cx)),
                    )
                    .child(
                        surfaces::icon_action(
                            "community-refresh",
                            "refresh",
                            if self.skill_store_busy {
                                tr("Working…")
                            } else {
                                tr("Refresh the directory")
                            },
                            surfaces::IconTone::Faded,
                            cx,
                        )
                        .disabled(self.skill_store_busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.load_community_skills(true, window, cx)
                        })),
                    )
                    .child(
                        surfaces::labelled_action(
                            "github-skill",
                            "link",
                            tr("From GitHub…"),
                            surfaces::IconSize::Inline,
                            if from_github {
                                surfaces::IconTone::On
                            } else {
                                surfaces::IconTone::Faded
                            },
                            cx,
                        )
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
            .child(surfaces::settings_note_help(
                tr("Skills others have published, for every project."),
                tr(
                    "Skills others have published, installed into your own skills folder \
                     (~/.wisp/skills), where every project can use them. A name that is taken is \
                     never replaced.",
                ),
                cx,
            ));
        if from_github {
            section = section.child(self.github_skill_form(cx));
        }
        if let Some(note) = &self.skill_store_note {
            section = section.child(error_banner(note, cx));
        }
        let candidates: Vec<_> = self
            .skill_candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| self.candidate_row(index, candidate, cx))
            .collect();
        if !candidates.is_empty() {
            section = section.child(surfaces::record_group(candidates, cx));
        }
        match &self.community_skills {
            None => section.child(surfaces::settings_note(tr("Loading…"), cx)),
            Some(catalog) => {
                if let Some(notice) = &catalog.notice {
                    section = section.child(surfaces::settings_note(notice.clone(), cx));
                }
                let rows: Vec<_> = catalog
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(index, entry)| self.community_row(index, entry, cx))
                    .collect();
                if !rows.is_empty() {
                    section = section.child(surfaces::record_group(rows, cx));
                }
                section
            }
        }
    }
}
