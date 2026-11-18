#!/usr/bin/env python3
"""Deterministic ACP peer. The CLI receives only Studio's private home and empty workspace."""
import json
import os
import sys
from pathlib import Path

def send(frame):
    print(json.dumps({"jsonrpc": "2.0", **frame}), flush=True)

def chunk(session, text):
    send({"method": "session/update", "params": {"sessionId": session, "update": {
        "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}}})

def end(request, reason="end_turn"):
    send({"id": request, "result": {"stopReason": reason}})

assert sys.argv[1:] == ["acp", "--agent", "verum-conversation", "--disable-telemetry"]
profile = Path(os.environ["HOME"]) / ".gemini"
settings = json.loads((profile / "antigravity-cli" / "settings.json").read_text())
assert settings["toolPermission"] == "strict" and settings["permissions"]["allow"] == []
assert "command(*)" in settings["permissions"]["deny"]
assert "tools: []" in (profile / "config" / "agents" / "verum-conversation.md").read_text()
assert json.loads((profile / "config" / "mcp_config.json").read_text()) == {"mcpServers":{}}
assert not any(os.getenv(key) for key in ["GOOGLE_API_KEY", "GOOGLE_APPLICATION_CREDENTIALS", "ANTHROPIC_API_KEY", "OPENAI_API_KEY"])
assert settings["enableTerminalSandbox"] is True and settings["allowNonWorkspaceAccess"] is False
assert list(Path.cwd().iterdir()) == []
sessions, active, refusals = 0, {}, set()
account = Path(os.environ["HOME"]) / "fixture-account"
authenticated = account.exists()
for line in sys.stdin:
    msg = json.loads(line)
    assert msg["jsonrpc"] == "2.0"
    method, request, params = msg.get("method"), msg.get("id"), msg.get("params", {})
    if method == "initialize":
        assert params["clientCapabilities"] == {} and params["protocolVersion"] == 1
        send({"id": request, "result": {"protocolVersion": 1, "agentInfo":{"name":"Antigravity ACP Server"}, "authMethods": [{"id": "oauth-personal"}]}})
    elif method == "authenticate":
        assert params == {"methodId":"oauth-personal"}
        authenticated = True
        account.touch()
        send({"id": request, "result": {}})
    elif method == "session/new":
        assert Path(params["cwd"]).resolve() == Path.cwd() and params["mcpServers"] == []
        if not authenticated:
            send({"id":request,"error":{"code":-32000,"message":"Authentication required"}})
            continue
        sessions += 1
        session = "s" + str(sessions)
        send({"id": request, "result": {"sessionId":session}})
    elif method == "session/prompt":
        session = params["sessionId"]
        assert len(params["prompt"]) == 1 and params["prompt"][0]["type"] == "text"
        text = params["prompt"][0]["text"]
        if text == "hold":
            active[session] = request
            chunk(session, "first ")
        elif text == "interleave":
            active[session] = request
            chunk(session, "reviewer ")
            send({"method":"session/update","params":{"sessionId":session,"update":{"sessionUpdate":"tool_call","toolCallId":"proposal","status":"pending"}}})
            send({"id": "permission", "method":"session/request_permission", "params":{"sessionId":session,"options":[]}})
            send({"id": "file", "method":"fs/read_text_file", "params":{"sessionId":session,"path":"example.vr"}})
        elif text == "fail":
            send({"id": request, "error":{"code":-32603, "message":"untrusted provider detail"}})
        elif text == "expired":
            authenticated = False
            account.unlink(missing_ok=True)
            send({"id": request, "error":{"code":-32000,"message":"Authentication required"}})
        elif text == "native":
            send({"method":"session/update","params":{"sessionId":session,"update":{"sessionUpdate":"tool_call_update","toolCallId":"unexpected","status":"in_progress"}}})
        elif text == "wait":
            active[session] = request
        elif text == "disconnect":
            sys.exit(0)
        else:
            chunk(session, text)
            end(request)
    elif method == "session/cancel":
        session = params["sessionId"]
        chunk(session,"late text after cancellation")
        end(active.pop(session), "cancelled")
    elif request == "permission":
        assert msg["result"] == {"outcome":{"outcome":"cancelled"}}
        send({"method":"session/update","params":{"sessionId":"s2","update":{"sessionUpdate":"tool_call_update","toolCallId":"proposal","status":"failed"}}})
        refusals.add(request)
    elif request == "file":
        assert msg["error"]["code"] == -32601
        refusals.add(request)
    if refusals == {"permission", "file"}:
        chunk("s1", "author")
        chunk("s2", "complete")
        end(active["s2"])
        end(active["s1"])
        end(active["s1"])
        active.clear()
        refusals.clear()
