#!/usr/bin/env python3
"""Offline ACP peer for native UI smoke, restricted to an explicit QA workspace.

Configure a QA-only ACP profile with Python plus this script and
--workspace /absolute/isolated/workspace --log /absolute/qa-events.jsonl.
Messages containing 'permission' request a harmless choice; 'wait' waits for
Stop; plan mode emits a synthetic plan; other messages stream a fixed answer. It never executes tools,
contacts a provider, or reads research data. The event log omits prompt content.
"""
import argparse
import json
from pathlib import Path
import sys
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", required=True, type=Path)
    parser.add_argument("--log", required=True, type=Path)
    parser.add_argument("--stall-initialize", action="store_true", help="Keep initialization pending to test startup cancellation")
    args = parser.parse_args()
    root = args.workspace.resolve(strict=True)
    pending_prompt = None
    permission_id = None
    session = None
    mode = "brief"
    config_options = [{"id": "detail", "name": "Response detail", "type": "select", "currentValue": "short",
                       "options": [{"group": "basic", "name": "Basic", "options": [{"value": "short", "name": "Short"}]},
                                   {"group": "advanced", "name": "Advanced", "options": [{"value": "long", "name": "Long"}]}]}]

    def session_state():
        return {"modes": {"currentModeId": mode, "availableModes": [
            {"id": "brief", "name": "Brief QA response"}, {"id": "detailed", "name": "Detailed QA response"},
            {"id": "plan", "name": "Plan QA response"}]},
            "configOptions": config_options}

    def send(value):
        print(json.dumps({"jsonrpc": "2.0", **value}), flush=True)

    def result(identifier, value):
        send({"id": identifier, "result": value})

    def record(method):
        with args.log.open("a", encoding="utf-8") as output:
            output.write(json.dumps({"method": method, "session_id": session}) + "\n")

    def chunk(text):
        send({"method": "session/update", "params": {
            "sessionId": session,
            "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}},
        }})

    for line in sys.stdin:
        value = json.loads(line)
        method = value.get("method", "")
        params = value.get("params", {})
        identifier = value.get("id")
        if not method:
            if permission_id is not None and identifier == permission_id:
                record("permission_response")
                outcome = value.get("result", {}).get("outcome", {})
                chunk("\nQA permission result: " + outcome.get("optionId", outcome.get("outcome", "cancelled")))
                result(pending_prompt, {"stopReason": "end_turn"})
                pending_prompt = permission_id = None
            continue
        if method == "initialize":
            record("initialize_wait" if args.stall_initialize else "initialize")
            if args.stall_initialize:
                continue
            result(identifier, {"protocolVersion": 1, "agentInfo": {"name": "wisp-native-qa", "version": "1"},
                                "agentCapabilities": {"loadSession": True, "sessionCapabilities": {"resume": {}, "close": {}}}, "authMethods": []})
        elif method in ("session/new", "session/load", "session/resume", "_unstable/session/resume"):
            if Path(params.get("cwd", "")).resolve() != root:
                send({"id": identifier, "error": {"code": -32602, "message": "This QA peer only accepts the configured isolated workspace"}})
                continue
            session = params.get("sessionId") or "qa-" + str(uuid.uuid4())
            record(method)
            result(identifier, {**session_state(), **({"sessionId": session} if method == "session/new" else {})})
        elif method in ("session/set_mode", "session/set_config_option"):
            if params.get("sessionId") != session:
                send({"id": identifier, "error": {"code": -32602, "message": "Unknown QA session"}})
                continue
            if method == "session/set_mode" and params.get("modeId") in ("brief", "detailed", "plan"):
                mode = params["modeId"]
                record(method)
                result(identifier, {})
            elif method == "session/set_config_option" and params.get("configId") == "detail" and params.get("value") in ("short", "long"):
                config_options[0]["currentValue"] = params["value"]
                record(method)
                result(identifier, {"configOptions": config_options})
            else:
                send({"id": identifier, "error": {"code": -32602, "message": "Unknown QA option"}})
        elif method == "session/prompt":
            if params.get("sessionId") != session:
                send({"id": identifier, "error": {"code": -32602, "message": "Unknown QA session"}})
                continue
            record(method)
            pending_prompt = identifier
            text = " ".join(part.get("text", "") for part in params.get("prompt", []) if part.get("type") == "text").lower()
            chunk("Synthetic ACP response from the isolated QA peer.")
            if mode == "plan":
                record("plan_proposal")
                send({"method": "session/update", "params": {"sessionId": session,
                    "update": {"sessionUpdate": "plan", "entries": [
                        {"content": "Inspect **synthetic samples**\n\n```python\nprint('fixture only')\n```", "status": "completed", "priority": "medium"},
                        {"content": "Review the synthetic result. No command is executed.", "status": "pending", "priority": "high"}
                    ]}}})
            if "permission" in text:
                permission_id = "qa-permission-" + str(uuid.uuid4())
                send({"id": permission_id, "method": "session/request_permission", "params": {
                    "sessionId": session,
                    "toolCall": {"toolCallId": "qa-noop", "title": "Synthetic permission (no tool executes)", "status": "pending", "kind": "read", "rawInput": {"fixture": "no-op"}},
                    "options": [{"optionId": "allow", "name": "Allow synthetic choice", "kind": "allow_once"},
                                {"optionId": "deny", "name": "Reject synthetic choice", "kind": "reject_once"}],
                }})
            elif "wait" not in text:
                result(identifier, {"stopReason": "end_turn"})
                pending_prompt = None
        elif method == "session/cancel":
            record(method)
            if pending_prompt is not None:
                chunk("\nSynthetic turn cancelled.")
                result(pending_prompt, {"stopReason": "cancelled"})
                pending_prompt = permission_id = None
        elif method in ("session/close", "_unstable/session/close"):
            record(method)
            result(identifier, {})
        elif identifier is not None:
            send({"id": identifier, "error": {"code": -32601, "message": "Unsupported QA method: " + method}})


if __name__ == "__main__":
    main()
