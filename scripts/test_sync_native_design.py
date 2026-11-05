"""Native resource regressions; no desktop, host connection or network required."""
import json
import unittest

import sync_native_design as design


class NativePaletteTests(unittest.TestCase):
    def test_scoped_translation_helpers_dont_hide_missing_or_double_prefix_keys(self):
        keys = set(design.native_translation_keys({
            "Page.Helpers.cs": 'private string T(string key) => Design.T("page." + key);',
            "Page.cs": 'T("title"); Design.T("global.cancel");',
            "Panel.Review.cs": 'string T(string k) => design.T("review." + k); T("ok");',
            "Panel.cs": 'T("absolute");',
        }))
        self.assertEqual(keys, {"page.title", "global.cancel", "review.ok", "absolute"})

    def test_message_brand_uses_the_canonical_logo(self):
        exports = dict(design.exports())
        self.assertEqual(exports["logo.svg"], (design.ROOT / "ui/logo.svg").read_bytes())
        catalog = json.loads(exports["native-locales.json"])
        for locale in ("en", "zh"):
            for key in ("chat.assistant", "msg.show_all", "msg.show_less", "msg.sent_at", "msg.replied_at"):
                self.assertTrue(catalog[locale][key])

    def test_response_syntax_colors_inherit_the_webview_theme(self):
        palettes = json.loads(dict(design.exports())["palette.json"])
        for name, palette in palettes.items():
            theme = "dark" if name.startswith("dark") else "light"
            for token in ("syntax-text", "syntax-comment", "syntax-string", "syntax-variable", "syntax-keyword"):
                self.assertEqual(palette[token], palettes[theme][token])
        self.assertNotEqual(palettes["dark"]["syntax-text"], palettes["light"]["syntax-text"])

    def test_context_panel_placement_icons_and_labels_are_shared(self):
        exports = dict(design.exports())
        for name in ("dock", "resize"):
            self.assertIn(f"icon-{name}.svg", exports)
        self.assertIn(b'd="m8 20 12-12m-6 12 6-6"', exports["icon-resize.svg"])
        catalog = json.loads(exports["native-locales.json"])
        for locale in ("en", "zh"):
            for key in ("context_usage.dock", "context_usage.resize"):
                self.assertTrue(catalog[locale][key])

    def test_context_compaction_uses_shared_icon_and_labels(self):
        exports = dict(design.exports())
        self.assertIn(b'd="M6 5h12"', exports["icon-context-compact.svg"])
        self.assertIn(b'd="M4 12h16"', exports["icon-context-compact.svg"])
        catalog = json.loads(exports["native-locales.json"])
        for locale in ("en", "zh"):
            for key in ("context_usage.nudge_compact", "context_usage.close", "compact.title", "compact.start"):
                self.assertTrue(catalog[locale][key])

    def test_transcript_process_icons_and_labels_are_shared(self):
        exports = dict(design.exports())
        for name in ("activity-orbit", "circle-minus", "circle-check", "chevron-right", "chevron-down"):
            self.assertIn(f"icon-{name}.svg", exports)
            self.assertIn(b'viewBox="0 0 24 24"', exports[f"icon-{name}.svg"])
        self.assertNotEqual(exports["icon-circle-minus.svg"], exports["icon-circle-check.svg"])
        catalog = json.loads(exports["native-locales.json"])
        for locale in ("en", "zh"):
            for key in ("chat.activity_done", "chat.thinking", "chat.progress", "chat.step_lines", "tool.badge.mcp", "tool.badge.skill"):
                self.assertTrue(catalog[locale][key])

    def test_connection_value_toggle_uses_shared_icons_and_bilingual_labels(self):
        exports = dict(design.exports())
        for icon in ("eye", "eye-off"):
            self.assertIn(f"icon-{icon}.svg", exports)
            self.assertIn(b'stroke-width="2"', exports[f"icon-{icon}.svg"])
        self.assertIn(b'<circle cx="12" cy="12" r="3"', exports["icon-eye.svg"])
        self.assertIn(b'd="m3 3 18 18"', exports["icon-eye-off.svg"])
        catalog = json.loads(exports["native-locales.json"])
        for locale in ("en", "zh"):
            for key in ("conn.secret_show", "conn.secret_hide", "conn.secret_value", "conn.secret_keep"):
                self.assertTrue(catalog[locale][key])
            self.assertNotEqual(catalog[locale]["conn.secret_show"], catalog[locale]["conn.secret_hide"])

    def test_workflow_conversion_checklist_icons_are_packaged(self):
        exports = dict(design.exports())
        for icon in ("branch", "list", "shield", "doc"):
            self.assertIn(f"icon-{icon}.svg", exports)
        self.assertIn(b'd="M20 13c0 5-3.5 7.5', exports["icon-shield.svg"])

    def test_credentials_use_the_webview_registry_and_locales(self):
        exports = dict(design.exports())
        groups = json.loads(exports["credential-groups.json"])
        self.assertEqual([group["id"] for group in groups], ["openalex", "infinisynapse", "scimaster", "ncbi"])
        self.assertEqual([(field["id"], field["secret"]) for field in groups[-1]["fields"]], [("ncbi_api_key", True), ("ncbi_email", False)])
        catalog = json.loads(exports["native-locales.json"])
        for group in groups:
            for key in ("name_key", "about_key", "configured_key", "unconfigured_key", "hint_key"):
                for locale in ("en", "zh"):
                    self.assertIn(group[key], catalog[locale])
            for link in group["links"]:
                self.assertTrue(link["url"].startswith("https://"))
        self.assertIn(b'd="M9.1 9a3', exports["icon-circle-help.svg"])

    def test_translucent_info_badges_preserve_shared_alpha(self):
        self.assertEqual(design.native_color("hsl(213 68% 50% / 0.12)"), "rgba(41, 119, 214, 0.12)")
        self.assertEqual(design.native_color("hsl(213 68% 50% / 12%)"), "rgba(41, 119, 214, 0.12)")
        palettes = json.loads(dict(design.exports())["palette.json"])
        for name, palette in palettes.items():
            self.assertEqual(palette["info-soft"], palettes[name.split("-")[0]]["info-soft"])
            self.assertTrue(palette["info-soft"].startswith("rgba("))

    def test_library_image_icon_is_exported_from_compose_icon(self):
        exports = dict(design.exports())
        self.assertIn("icon-image.svg", exports)
        self.assertIn(b'<circle cx="8.5" cy="8.5" r="1.5"', exports["icon-image.svg"])

    def test_multiline_brain_icon_preserves_all_shared_paths(self):
        svg = dict(design.exports())["icon-brain.svg"]
        self.assertEqual(svg.count(b"<path "), 9)
        self.assertIn(b'M19.967 17.484A4 4 0 0 1 18 18', svg)
        self.assertNotIn(b'into_view', svg)

    def test_session_number_arrows_are_exported_from_the_shared_icon_set(self):
        exports = dict(design.exports())
        self.assertIn(b'd="m6 15 6-6 6 6"', exports["icon-chevron-up.svg"])
        self.assertIn(b'd="m6 9 6 6 6-6"', exports["icon-chevron-down.svg"])

    def test_storage_semantic_colors_are_available_in_every_palette(self):
        palettes = json.loads(dict(design.exports())["palette.json"])
        for name, palette in palettes.items():
            for token in ("clay", "info", "warn", "ok", "border-strong"):
                self.assertIn(token, palette, f"{name}: {token}")
            self.assertEqual(palette["info"], palettes[name.split("-")[0]]["info"])
            self.assertTrue(palette["info"].startswith("#"))

    def test_new_project_uses_the_shared_bilingual_form_and_layout_catalog(self):
        catalog = json.loads(dict(design.exports())["native-locales.json"])
        for locale in ("en", "zh"):
            for key in ("projects.new", "projects.create", "projects.cancel", "projects.name_ph",
                        "projects.directory", "projects.choose_dir", "projects.standard_layout",
                        "projects.standard_layout_hint", "proj_settings.name", "proj_settings.description",
                        "proj_settings.description_hint", "proj_settings.agent_context", "proj_settings.agent_context_hint"):
                self.assertTrue(catalog[locale][key], f"{locale}: {key}")
            block = catalog[locale]["projects.layout_context"]
            self.assertGreater(block.count("\n"), 5)
            for directory in ("figures/", "results/tables/", "data/raw/", "literature/"):
                self.assertIn(directory, block)
        self.assertEqual(catalog["en"]["projects.create"], "Create")
        self.assertEqual(catalog["zh"]["projects.create"], "创建")
        self.assertNotEqual(catalog["en"]["projects.layout_context"], catalog["zh"]["projects.layout_context"])

    def test_media_options_are_exported_from_shared_dto_constants(self):
        media = json.loads(dict(design.exports())["model-media-options.json"])
        self.assertEqual(media["OPENAI_IMAGE_QUALITIES"], ["auto", "low", "medium", "high"])
        self.assertEqual(media["GROK_IMAGE_RESOLUTIONS"], ["1k", "2k"])
        self.assertEqual(media["GROK_IMAGE_QUALITIES"], ["medium", "low"])
        self.assertEqual(media["VIDEO_ASPECT_RATIOS"], ["16:9", "9:16", "1:1", "4:3", "3:4"])
        self.assertEqual(media["VIDEO_DURATION_DEFAULT_SECS"], 5)
        self.assertEqual((media["VIDEO_DURATION_MIN_SECS"], media["VIDEO_DURATION_MAX_SECS"]), (1, 15))

    def test_decision_presets_are_exported_from_shared_dto_constants(self):
        decision = json.loads(dict(design.exports())["decision-model.json"])
        presets = {row["id"]: row for row in decision["presets"]}
        self.assertEqual(list(presets), ["typesafe", "startlux_bamboo", "startlux_local", "custom"])
        self.assertEqual(presets["startlux_local"]["base_url"], "http://127.0.0.1:8090")
        self.assertEqual(presets["startlux_bamboo"]["timeout_secs"], 60)
        self.assertTrue(presets["typesafe"]["key_required"])
        self.assertFalse(presets["custom"]["key_required"])
        self.assertEqual(decision["think_modes"], ["", "auto", "never", "always"])

    def test_composer_accent_foreground_is_available_in_every_palette(self):
        palettes = json.loads(dict(design.exports())["palette.json"])
        for name, palette in palettes.items():
            self.assertEqual(palette["on-clay"], palettes[name.split("-")[0]]["on-clay"])
        self.assertEqual(palettes["light"]["on-clay"], "#ffffff")
        self.assertEqual(palettes["dark"]["on-clay"], "#f4fffc")

    def test_workspace_welcome_uses_all_shared_rotating_translations(self):
        catalog = json.loads(dict(design.exports())["native-locales.json"])
        for locale in ("en", "zh"):
            for prefix, count in (("empty.title", 7), ("empty.subtitle", 7), ("tip", 10)):
                for index in range(count):
                    self.assertTrue(catalog[locale][f"{prefix}.{index}"])
            for key in ("sidebar.no_sessions", "sidebar.new_session", "tips.title", "tips.close",
                        "projects.sessions_n", "projects.artifacts_n", "projects.star_hint", "projects.star_link"):
                self.assertTrue(catalog[locale][key])

    def test_appearance_hints_and_samples_keep_shared_translations(self):
        catalog = json.loads(dict(design.exports())["native-locales.json"])
        for locale in ("en", "zh"):
            for key in ("appearance.ui_font_size", "appearance.ui_font_size_hint",
                        "appearance.ui_font_family", "appearance.ui_font_family_hint",
                        "appearance.code_font_size", "appearance.code_font_size_hint",
                        "appearance.code_font_family", "appearance.code_font_family_hint",
                        "appearance.preview_question", "appearance.preview_heading",
                        "appearance.preview_answer", "appearance.preview_diff"):
                self.assertTrue(catalog[locale][key], f"{locale}: {key}")

    def test_locale_catalog_preserves_keys_multiline_hints_and_escapes(self):
        source = r'''(Locale::En, "settings.one") => Some(
            "Line one\nLine two with \"quotes\"",
        ),
        (Locale::Zh, "settings.one") => Some("中文"),
        (Locale::En, "settings.two") => Some("Distinct context"),
        (Locale::Zh, "settings.two") => Some("中文"),'''
        catalog = design.locale_catalog(source)
        self.assertEqual(catalog["en"]["settings.one"], 'Line one\nLine two with "quotes"')
        self.assertEqual(catalog["zh"]["settings.one"], catalog["zh"]["settings.two"])
        self.assertNotEqual(catalog["en"]["settings.one"], catalog["en"]["settings.two"])

    def test_action_helpers_export_their_shared_icons(self):
        source = 'design.Icon("copy"); Icon("Zoom out", "minus", action); design.ToolButton("Settings", "gear");'
        self.assertEqual(design.native_icon_references(source), {"copy", "minus", "gear"})
        self.assertEqual(design.native_icon_references('FileButton(design.T("files.up"), "up");'), {"up"})
        self.assertEqual(design.native_icon_references('FileButton(design.L(name + " 操作", "Actions for " + name), "more");'), {"more"})
        self.assertIn("minus", design.ICONS)
        plus = dict(design.exports())["icon-plus.svg"]
        self.assertIn(b'<path d="M12 5v14"', plus)
        self.assertIn(b'<path d="M5 12h14"', plus)

    def test_locale_catalog_accepts_rustfmt_blocks_and_wrapped_match_patterns(self):
        source = '''(Locale::En, "settings.semantic") => {
            Some("Semantic compact")
        }
        (
            Locale::Zh,
            "settings.semantic",
        ) => Some("语义压缩"),'''
        catalog = design.locale_catalog(source)
        self.assertEqual(catalog["en"]["settings.semantic"], "Semantic compact")
        self.assertEqual(catalog["zh"]["settings.semantic"], "语义压缩")

    def test_input_background_is_exported_for_every_theme(self):
        palettes = json.loads(dict(design.exports())["palette.json"])
        self.assertTrue(all("bg-input" in palette for palette in palettes.values()))
        self.assertEqual(palettes["light-paper"]["bg-input"], "#fff")
        self.assertEqual(palettes["dark-charcoal"]["bg-input"], "#262521")
        self.assertNotEqual(palettes["dark-charcoal"]["bg-input"], palettes["dark-charcoal"]["bg-elev"])

    def test_dialog_focus_ring_inherits_the_webview_theme_for_every_palette(self):
        palettes = json.loads(dict(design.exports())["palette.json"])
        for name, palette in palettes.items():
            theme = name.split("-", 1)[0]
            self.assertEqual(palette["focus-ring"], palettes[theme]["focus-ring"], name)
        self.assertEqual(palettes["light"]["focus-ring"], "rgba(13, 148, 136, 0.16)")
        self.assertEqual(palettes["dark"]["focus-ring"], "rgba(45, 168, 152, 0.28)")

    def test_pet_and_browser_use_the_shared_bilingual_catalog(self):
        catalog = json.loads(dict(design.exports())["native-locales.json"])
        for locale in ("en", "zh"):
            for key in ("pet.not_configured", "pet.description", "pet.enabled", "pet.directory",
                        "pet.directory_hint", "browser.filters.block", "browser.filters.prefer",
                        "browser.filters.reason", "browser.filters.saved", "browser.auto_launch_saved",
                        "browser.auto_close_tabs_saved"):
                self.assertTrue(catalog[locale][key], f"{locale}: {key}")
        self.assertEqual(catalog["en"]["pet.not_configured"], "No pet configured")
        self.assertEqual(catalog["en"]["browser.filters.block"], "Blocked hosts")

    def test_plugin_page_and_installer_share_complete_bilingual_copy(self):
        catalog = json.loads(dict(design.exports())["native-locales.json"])
        keys = ("settings.nav.plugins", "settings.cancel", "plugins.install_title", "plugins.install_safety",
                "plugins.install_close", "plugins.source_local", "plugins.source_remote", "plugins.zip_file",
                "plugins.no_zip_selected", "plugins.choose_zip", "plugins.url", "plugins.sha256_optional",
                "plugins.sha256_required", "plugins.sha256_hint", "plugins.install_action", "plugins.install_url",
                "plugins.summary", "plugins.search", "plugins.empty", "plugins.no_match", "plugins.details",
                "plugins.runtime_ready", "plugins.runtime_unavailable", "plugins.runtime_not_applicable",
                "plugins.installed", "plugins.remove", "plugins.remove_confirm")
        for locale in ("en", "zh"):
            for key in keys:
                self.assertTrue(catalog[locale][key], f"{locale}: {key}")
            self.assertIn("{enabled}", catalog[locale]["plugins.summary"])
            self.assertIn("{total}", catalog[locale]["plugins.summary"])
            self.assertIn("{plugin}", catalog[locale]["plugins.remove_confirm"])
        self.assertEqual(catalog["en"]["plugins.source_remote"], "Release URL")

    def test_memory_page_shares_labels_and_distinct_mutation_feedback(self):
        catalog = json.loads(dict(design.exports())["native-locales.json"])
        keys = ("settings.nav.memory", "settings.cancel", "memory.enabled_label", "memory.choose_project",
                "memory.load_failed", "memory.scope_hint", "memory.add", "memory.empty", "memory.clear_all",
                "memory.off_banner", "memory.turn_on", "memory.global_scope", "memory.global_add",
                "memory.global_timing_hint", "memory.global_empty", "memory.global_edit", "memory.global_delete",
                "memory.global_save", "memory.global_created", "memory.global_updated", "memory.global_deleted",
                "memory.proposal.content", "memory.save", "memory.delete")
        for locale in ("en", "zh"):
            for key in keys:
                self.assertTrue(catalog[locale][key], f"{locale}: {key}")
            self.assertEqual(len({catalog[locale]["memory.global_" + action] for action in ("created", "updated", "deleted")}), 3)

    def test_hook_page_uses_shared_bilingual_labels_and_event_descriptions(self):
        catalog = json.loads(dict(design.exports())["native-locales.json"])
        keys = ("settings.nav.hooks", "settings.cancel", "settings.save", "hooks.intro", "hooks.builtin",
                "hooks.custom", "hooks.add", "hooks.empty", "hooks.event", "hooks.matcher", "hooks.matcher_hint",
                "hooks.timeout", "hooks.command", "hooks.command_hint", "hooks.enabled", "hooks.remove",
                "hooks.auto_review.desc", "hooks.failure_analysis.desc", "hooks.acp_note", "hooks.project",
                "hooks.project_desc", "hooks.project_trust", "hooks.project_revoke", "hooks.project_trusted",
                "hooks.project_untrusted", "composer.auto_review", "composer.auto_failure_analysis",
                "composer.reviewer_model", "composer.failure_rate_threshold", "composer.minimum_failures",
                "composer.reviewer.default_http", "composer.reviewer.follow_session")
        events = ("user_prompt_submit", "pre_tool_use", "post_tool_use", "post_tool_use_failure", "stop")
        for locale in ("en", "zh"):
            for key in keys + tuple("hooks.event_hint." + event for event in events):
                self.assertTrue(catalog[locale][key], f"{locale}: {key}")
            self.assertIn("{path}", catalog[locale]["hooks.project_desc"])
            self.assertEqual(len({catalog[locale]["hooks.event_hint." + event] for event in events}), 5)

    def test_skill_store_shares_bilingual_browse_preview_and_confirmation_copy(self):
        assets = dict(design.exports())
        self.assertIn("loader", design.ICONS)
        self.assertIn(b'<circle cx="12" cy="12" r="9"', assets["icon-loader.svg"])
        catalog = json.loads(dict(design.exports())["native-locales.json"])
        keys = ("settings.nav.skills", "store.back", "store.title", "store.guide", "store.scope",
                "store.browse", "store.github", "store.marketplaces", "store.openai_source", "store.anthropic_source",
                "store.bear_source", "store.refresh_source", "store.marketplace_notice", "store.community_notice",
                "store.search", "store.tags", "store.all_tags", "store.refresh", "store.directory_entries", "store.empty",
                "store.community", "store.user_source", "store.name_present", "store.not_installed", "store.preview",
                "store.url", "store.exact_ref", "store.ref_help", "store.discover", "store.loading_title", "store.loading",
                "store.search_packages", "store.metadata_drift", "store.responsibilities", "store.trigger", "store.inputs",
                "store.outputs", "store.excludes", "store.required", "store.optional", "store.boundaries", "store.declared",
                "store.verified", "store.unverified", "store.limits", "store.author", "store.feedback", "store.format_pass",
                "store.format_error", "store.resource_error", "store.conflict", "store.invalid", "store.installed_source",
                "store.dependencies_pending", "store.already_installed", "store.review_install", "store.confirm_title",
                "store.confirm_install", "store.installing", "store.no_execution", "store.installed", "store.cancel")
        for locale in ("en", "zh"):
            for key in keys:
                self.assertTrue(catalog[locale][key], f"{locale}: {key}")
        self.assertIn("~/.wisp/skills", catalog["en"]["store.scope"])
        self.assertIn("never replaced", catalog["en"]["store.no_execution"])

    def test_skills_list_and_detail_share_translations_and_template_values(self):
        catalog = json.loads(dict(design.exports())["native-locales.json"])
        keys = ("settings.nav.skills", "settings.auto_saved_new_session", "skills.add", "skills.add_file",
                "skills.add_folder", "skills.search_ph", "skills.summary", "skills.enable_visible", "skills.disable_visible",
                "skills.reload", "skills.all", "skills.untagged", "skills.enabled", "skills.disabled", "skills.empty",
                "skills.scope.bundled", "skills.scope.global", "skills.scope.project", "skills.scope.extra",
                "skills.scope.plugin", "skills.scope.custom", "skills.remove", "skills.remove_confirm", "skills.managed_by",
                "skills.edit_tags", "skills.tags_placeholder", "skills.retry", "skills.files", "skills.select_file",
                "skills.preview", "skills.source", "skills.loading", "skills.no_files")
        for locale in ("en", "zh"):
            for key in keys:
                self.assertTrue(catalog[locale][key], f"{locale}: {key}")
            for field in ("visible", "enabled", "total"):
                self.assertIn("{" + field + "}", catalog[locale]["skills.summary"])
            self.assertIn("{skill}", catalog[locale]["skills.remove_confirm"])
            self.assertIn("{plugin}", catalog[locale]["skills.managed_by"])

    def test_model_context_view_keeps_shared_icons_and_epoch_copy(self):
        exported = dict(design.exports())
        for icon in ("history", "eye", "chevron-right"):
            self.assertIn(f"icon-{icon}.svg", exported)
        catalog = json.loads(exported["native-locales.json"])
        for locale in ("en", "zh"):
            for key in ("chat.view_full", "chat.view_model", "chat.context_system",
                        "chat.context_tombstone", "chat.compaction_checkpoint",
                        "chat.context_view_loading", "chat.context_view_changed"):
                self.assertTrue(catalog[locale][key], f"{locale}: {key}")
            for key in ("context_usage.epoch_line", "context_usage.epoch_line_no_checkpoint"):
                for field in ("epoch", "turns"):
                    self.assertIn("{" + field + "}", catalog[locale][key])

    def test_missing_color_fails_export_before_page_construction(self):
        with self.assertRaisesRegex(ValueError, "dark.*bg-input"):
            design.validate_palette_tokens({"light": {"bg-input": "#fff"}, "dark": {}}, {"bg-input"})

    def test_quick_actions_and_specialists_share_translated_controls(self):
        catalog = json.loads(dict(design.exports())["native-locales.json"])
        for locale in ("en", "zh"):
            for key in ("quick_actions.title", "quick_actions.help", "quick_actions.new",
                        "quick_actions.builtin", "quick_actions.open_workflow", "quick_actions.enabled",
                        "selection.research_literature", "specialists.name", "specialists.instructions.hint",
                        "specialists.builtin_locked", "specialists.model.follow", "specialists.skills.search_hint",
                        "specialists.skills.remove", "composer.reviewer.default_http", "composer.reviewer.follow_session",
                        "specialists.reviewer.test", "specialists.reviewer.test_hint", "specialists.reviewer.testing",
                        "specialists.reviewer.test_ok", "specialists.reviewer.test_failed",
                        "specialists.add.scratch", "specialists.add.chat", "specialists.chat_prompt",
                        "specialists.add.import", "specialists.import.reviewed", "specialists.import.missing_hint",
                        "specialists.import.model_follow", "specialists.export.saved_hint", "specialists.export.secret_body",
                        "specialists.export.anyway", "specialists.secret.assignment"):
                self.assertTrue(catalog[locale][key], f"{locale}: {key}")
        self.assertIn("{count}", catalog["en"]["specialists.skills.search_hint"])
        self.assertIn("{skill}", catalog["zh"]["specialists.skills.remove"])

    def test_builtin_badge_keeps_shared_translucent_accent_in_all_themes(self):
        palettes = json.loads(dict(design.exports())["palette.json"])
        self.assertTrue(all("clay-soft" in palette for palette in palettes.values()))
        self.assertTrue(palettes["light"]["clay-soft"].startswith("rgba("))
        self.assertTrue(palettes["dark"]["clay-soft"].startswith("rgba("))

    def test_approval_colors_keep_the_base_theme_semantics(self):
        palettes = json.loads(dict(design.exports())["palette.json"])
        for name, palette in palettes.items():
            for token in ("ok", "warn", "err"):
                self.assertEqual(palette[token], palettes[name.split("-")[0]][token])
        self.assertTrue({"hand", "ban"}.issubset(design.ICONS))

    def test_exported_colors_are_readable_by_native_renderers(self):
        self.assertEqual(design.native_color("hsl(120 100% 50%)"), "#00ff00")
        for palette in json.loads(dict(design.exports())["palette.json"]).values():
            for color in palette.values():
                self.assertRegex(color, r"^(#[0-9a-fA-F]{3}(?:[0-9a-fA-F]{3})?|rgba\([\d.,\s]+\))$")

    def test_assistant_rails_use_exported_shared_icons(self):
        exported = dict(design.exports())
        for icon in ("panel-left", "panel", "layers", "sparkles"):
            self.assertIn(f"icon-{icon}.svg", exported)

    def test_shelved_conversations_use_the_shared_restore_icon(self):
        exported = dict(design.exports())
        for icon in ("eye-off", "adjustments", "chat", "undo", "folder-move", "trash", "minus", "check"):
            self.assertIn(f"icon-{icon}.svg", exported)
        self.assertIn(b'd="M9 14 4 9l5-5"', exported["icon-undo.svg"])


if __name__ == "__main__":
    unittest.main()
