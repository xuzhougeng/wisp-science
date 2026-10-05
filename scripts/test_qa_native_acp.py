import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class NativeAcpFixtureTests(unittest.TestCase):
    def test_plan_proposal_and_exit_use_only_synthetic_events(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            operations = [
                ("session/load", {"cwd": str(root), "sessionId": "qa-plan"}),
                ("session/set_mode", {"sessionId": "qa-plan", "modeId": "plan"}),
                ("session/prompt", {"sessionId": "qa-plan", "prompt": [{"type": "text", "text": "create a plan"}]}),
                ("session/set_mode", {"sessionId": "qa-plan", "modeId": "brief"}),
                ("session/prompt", {"sessionId": "qa-plan", "prompt": [{"type": "text", "text": "approve"}]})]
            completed = subprocess.run([sys.executable, str(Path(__file__).with_name("qa_native_acp.py")),
                "--workspace", str(root), "--log", str(root / "events.jsonl")],
                input="".join(json.dumps({"id": i, "method": method, "params": params}) + "\n"
                    for i, (method, params) in enumerate(operations)), capture_output=True, text=True, check=True, timeout=10)
            output = [json.loads(line) for line in completed.stdout.splitlines()]
            plans = [row["params"]["update"] for row in output if row.get("params", {}).get("update", {}).get("sessionUpdate") == "plan"]
            self.assertEqual(len(plans), 1)
            self.assertEqual(plans[0]["entries"][1]["priority"], "high")
            self.assertEqual([row["id"] for row in output if row.get("result", {}).get("stopReason") == "end_turn"], [2, 4])

    def test_modes_and_grouped_config_options_are_confirmed_by_session_state(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            def request(identifier, method, **params):
                return {"jsonrpc": "2.0", "id": identifier, "method": method, "params": params}
            messages = [
                request(1, "session/load", cwd=str(root), sessionId="qa-controls"),
                request(2, "session/set_mode", sessionId="qa-controls", modeId="detailed"),
                request(3, "session/set_config_option", sessionId="qa-controls", configId="detail", value="long"),
                request(4, "session/load", cwd=str(root), sessionId="qa-controls"),
                request(5, "session/set_mode", sessionId="foreign", modeId="brief"),
                request(6, "session/set_mode", sessionId="qa-controls", modeId="Detailed QA response"),
            ]
            completed = subprocess.run(
                [sys.executable, str(Path(__file__).with_name("qa_native_acp.py")), "--workspace", str(root), "--log", str(root / "events.jsonl")],
                input="".join(json.dumps(value) + "\n" for value in messages), capture_output=True, text=True, check=True, timeout=10,
            )
            output = {value["id"]: value for value in map(json.loads, completed.stdout.splitlines())}
            self.assertEqual(output[4]["result"]["modes"]["currentModeId"], "detailed")
            self.assertEqual(output[3]["result"]["configOptions"][0]["currentValue"], "long")
            self.assertEqual(output[1]["result"]["configOptions"][0]["options"][1]["options"][0]["value"], "long")
            self.assertEqual(output[5]["error"]["code"], -32602)
            self.assertEqual(output[6]["error"]["code"], -32602)

    def test_startup_stall_records_readiness_without_replying(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve(); log = root / "events.jsonl"
            completed = subprocess.run(
                [sys.executable, str(Path(__file__).with_name("qa_native_acp.py")), "--workspace", str(root), "--log", str(log), "--stall-initialize"],
                input=json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}) + "\n",
                capture_output=True, text=True, check=True, timeout=10,
            )
            self.assertEqual(completed.stdout, "")
            self.assertEqual(json.loads(log.read_text())["method"], "initialize_wait")

    def test_restored_session_permission_stop_and_workspace_guard(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            log = root / "events.jsonl"
            def request(identifier, method, **params):
                return {"jsonrpc": "2.0", "id": identifier, "method": method, "params": params}
            messages = [
                request(1, "initialize"),
                request(2, "session/load", cwd=str(root), sessionId="qa-saved"),
                request(3, "session/prompt", sessionId="qa-saved", prompt=[{"type": "text", "text": "permission private prompt"}]),
                request(4, "session/cancel", sessionId="qa-saved"),
                request(5, "session/prompt", sessionId="qa-saved", prompt=[{"type": "text", "text": "wait private prompt"}]),
                request(6, "session/cancel", sessionId="qa-saved"),
                request(7, "session/new", cwd=str(root / "wrong")),
            ]
            completed = subprocess.run(
                [sys.executable, str(Path(__file__).with_name("qa_native_acp.py")), "--workspace", str(root), "--log", str(log)],
                input="".join(json.dumps(value) + "\n" for value in messages),
                capture_output=True, text=True, check=True, timeout=10,
            )
            output = [json.loads(line) for line in completed.stdout.splitlines()]
            permission = next(value for value in output if value.get("method") == "session/request_permission")
            self.assertEqual(permission["params"]["sessionId"], "qa-saved")
            self.assertEqual([option["optionId"] for option in permission["params"]["options"]], ["allow", "deny"])
            for identifier in (3, 5):
                self.assertEqual(next(value for value in output if value.get("id") == identifier)["result"]["stopReason"], "cancelled")
            self.assertEqual(next(value for value in output if value.get("id") == 7)["error"]["code"], -32602)
            self.assertNotIn("private prompt", log.read_text())
            self.assertEqual(sum(json.loads(line)["method"] == "session/cancel" for line in log.read_text().splitlines()), 2)


if __name__ == "__main__":
    unittest.main()
