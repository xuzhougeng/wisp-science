"""Regression: filesystem traversal must not select a same-named Rust helper."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import sync_native_settings_contract as contract


class ExportTests(unittest.TestCase):
    def test_example_copy_preserves_the_tauri_alias_and_target(self):
        commands = {row["command"]: row for row in json.loads(contract.export())["commands"]}
        command = commands["copy_demo_to_project"]
        self.assertEqual(command["source"], "src-tauri/src/seed.rs")
        self.assertEqual(command["arguments"], [
            {"name": "id", "rust_type": "String", "optional": False},
            {"name": "targetProjectId", "rust_type": "String", "optional": False},
        ])
        self.assertEqual(command["result"], "Result<String, String>")

    def test_assistant_weixin_keeps_its_separate_destination(self):
        commands = {row["command"]: row for row in json.loads(contract.export())["commands"]}
        self.assertEqual(commands["assistant_weixin_status"]["arguments"], [])
        self.assertEqual(commands["assistant_weixin_status"]["result"], "Result<AssistantWeixinStatus, String>")
        for name in ("set_weixin_channel", "weixin_bind_poll", "weixin_unbind"):
            self.assertIn({"name": "destination", "rust_type": "Option<WeixinDestination>", "optional": True}, commands[name]["arguments"])

    def test_home_skills_can_resolve_the_restored_workspace(self):
        commands = {row["command"]: row for row in json.loads(contract.export())["commands"]}
        command = commands["get_project_info"]
        self.assertEqual(command["arguments"], [])
        self.assertEqual(command["result"], "Result<ProjectInfo, String>")
        self.assertEqual(command["source"], "src-tauri/src/project_commands.rs")

    def test_subscription_account_pool_uses_existing_host_commands(self):
        commands = {row["command"]: row for row in json.loads(contract.export())["commands"]}
        for name in ("list_codex_accounts", "import_local_codex_accounts"):
            self.assertEqual(commands[name]["arguments"], [])
        for name in ("switch_codex_account", "remove_codex_account", "codex_account_usage"):
            self.assertEqual(commands[name]["arguments"], [{"name": "accountId", "rust_type": "String", "optional": False}])
            self.assertEqual(commands[name]["source"], "src-tauri/src/codex_accounts.rs")

    def test_decision_model_reuses_existing_keyring_commands(self):
        commands = {row["command"]: row for row in json.loads(contract.export())["commands"]}
        self.assertEqual(commands["get_decision_model"]["arguments"], [])
        for name in ("save_decision_model", "test_decision_model"):
            self.assertEqual([arg["name"] for arg in commands[name]["arguments"]], ["settings", "apiKey"])
            self.assertEqual(commands[name]["source"], "src-tauri/src/decision_model.rs")
        self.assertEqual(commands["clear_decision_model_key"]["arguments"], [{"name": "preset", "rust_type": "String", "optional": False}])

    def test_workflow_progress_reuses_the_existing_stage_dto(self):
        commands = {row['command']: row for row in json.loads(contract.export())['commands']}
        progress = commands['native_workflow_conversion_progress']
        self.assertEqual(progress['arguments'], [{'name': 'conversionId', 'rust_type': 'String', 'optional': False}])
        self.assertEqual(progress['result'], 'Option<WorkflowConversionProgress>')

    def test_specialist_packages_reuse_the_shared_transfer_commands(self):
        commands = {row["command"]: row for row in json.loads(contract.export())["commands"]}
        self.assertEqual(commands["pick_specialist_package"]["arguments"], [])
        self.assertEqual([arg["name"] for arg in commands["preview_specialist_import"]["arguments"]], ["path"])
        self.assertEqual([arg["name"] for arg in commands["import_specialist"]["arguments"]], ["package", "modelId"])
        self.assertEqual([arg["name"] for arg in commands["export_specialist"]["arguments"]], ["id", "acknowledgeSecrets"])

    def test_reviewer_probe_accepts_optional_session_without_mirroring_result(self):
        commands = json.loads(contract.export())["commands"]
        command = next(row for row in commands if row["command"] == "test_reviewer_backend")
        self.assertEqual(command["arguments"], [
            {"name": "reviewer", "rust_type": "specialists::Specialist", "optional": False},
            {"name": "sessionId", "rust_type": "Option<String>", "optional": True},
        ])
        self.assertEqual(command["result"], "Result<wisp_dto::ReviewerBackendTestResult, String>")

    def test_only_tauri_command_is_exported_with_portable_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "crates/wisp-dto/src/native_settings.rs"
            source.parent.mkdir(parents=True)
            source.write_text('pub const COMMANDS: &[&str] = &["example"];', encoding="utf-8")
            rust = root / "src-tauri/src"
            rust.mkdir(parents=True)
            (rust / "lib.rs").write_text('.invoke_handler(example,)', encoding="utf-8")
            (rust / "a_helper.rs").write_text('fn example(secret: &str) -> () {}', encoding="utf-8")
            (rust / "z_command.rs").write_text(
                '#[tauri::command]\npub(super) async fn example(\n project_id: String,\n) -> String { "研究" }',
                encoding="utf-8")
            with patch.object(contract, "ROOT", root):
                command = json.loads(contract.export())["commands"][0]
                self.assertEqual(command["source"], "src-tauri/src/z_command.rs")
                self.assertEqual(command["arguments"], [{"name": "projectId", "rust_type": "String", "optional": False}])
                (rust / "duplicate.rs").write_text('#[tauri::command]\nfn example() {}', encoding="utf-8")
                with self.assertRaisesRegex(ValueError, "Expected one Tauri command"):
                    contract.export()


if __name__ == "__main__":
    unittest.main()
