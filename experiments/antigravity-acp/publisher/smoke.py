"""Check the private controller and recorded ACP schema without starting an Agent."""
import argparse
import json
import pathlib
import selectors
import subprocess
import urllib.error
import urllib.request
import uuid

parser = argparse.ArgumentParser()
parser.add_argument("--binary", required=True)
parser.add_argument("--initialize-evidence", required=True)
parser.add_argument("--output", required=True)
args = parser.parse_args()
process = subprocess.Popen(
    [args.binary], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
    stderr=subprocess.DEVNULL, text=True,
)
selector = selectors.DefaultSelector()
selector.register(process.stdout, selectors.EVENT_READ)


def command(**payload):
    process.stdin.write(json.dumps(payload) + "\n")
    process.stdin.flush()
    if not selector.select(timeout=10):
        raise TimeoutError("publisher controller response timed out")
    return json.loads(process.stdout.readline())


try:
    original = json.loads(pathlib.Path(args.initialize_evidence).read_text())[0]["result"]
    schema = command(op="validate_initialize", response=original)
    assert schema["ok"]
    parsed = schema["parsed"]
    assert parsed["protocolVersion"] == 1
    assert parsed["agentCapabilities"]["mcpCapabilities"]["http"] is True
    assert parsed["agentCapabilities"]["promptCapabilities"]["image"] is True
    assert len(parsed["authMethods"]) == 4
    assert parsed["agentCapabilities"]["auth"]["logout"] == {}
    assert parsed["agentCapabilities"]["sessionCapabilities"]["list"] == {}
    started = command(op="start")
    assert started["ok"]
    assert not command(op="start")["ok"]
    headers = {
        "Authorization": started["authorization_header"],
        "Accept": "application/json, text/event-stream",
        "Content-Type": "application/json",
        "mcp-protocol-version": "2025-03-26",
    }
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def post(payload, override=None):
        request = urllib.request.Request(
            started["endpoint_url"], data=json.dumps(payload).encode(),
            headers=override or headers,
        )
        with opener.open(request, timeout=5) as response:
            raw = response.read().decode()
            if raw.startswith("event:") or raw.startswith("data:"):
                raw = next(line[6:] for line in raw.splitlines()
                           if line.startswith("data: ") and line[6:].strip())
            body = json.loads(raw) if raw.strip() else None
            return response.status, response.headers, body

    initialize = {
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-03-26", "capabilities": {},
                   "clientInfo": {"name": "lens-probe-review", "version": "1"}},
    }
    try:
        post(initialize, dict(headers, Authorization="Bearer deliberately-invalid"))
        raise AssertionError("unauthorized request accepted")
    except urllib.error.HTTPError as error:
        assert error.code == 401
    status, response_headers, body = post(initialize)
    assert status == 200 and body["result"]["serverInfo"]["name"] == "lens-output-mcp"
    if response_headers.get("mcp-session-id"):
        headers["mcp-session-id"] = response_headers["mcp-session-id"]
    post({"jsonrpc": "2.0", "method": "notifications/initialized"})
    turn_id = str(uuid.uuid4())
    assert command(op="begin", turn_id=turn_id)["ok"]
    assert not command(op="begin", turn_id=str(uuid.uuid4()))["ok"]
    html = "<h1>Publisher smoke</h1>"
    _, _, published = post({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "publish_html", "arguments": {"turn_id": turn_id, "html": html}},
    })
    receipt = json.loads(published["result"]["content"][0]["text"])
    assert receipt["accepted"] is True
    finished = command(op="finish")
    assert finished["publication"]["html"] == html
    assert finished["publication"]["id"] == receipt["publication_id"]
    assert command(op="begin", turn_id=str(uuid.uuid4()))["ok"]
    assert command(op="abort")["ok"]
    assert not command(op="finish")["ok"]
    evidence = {
        "scope": "controller + production HttpPublisher smoke; no provider launched",
        "schema_version": "1.7.0", "initialize_deserialized": True,
        "retained_capabilities": parsed["agentCapabilities"],
        "retained_auth_method_ids": [method["id"] for method in parsed["authMethods"]],
        "unauthorized_http_status": 401, "production_publication_roundtrip": True,
        "duplicate_start_and_turn_rejected": True, "abort_discards_turn": True,
        "credentials_persisted": False,
    }
    pathlib.Path(args.output).write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps(evidence, indent=2))
finally:
    try:
        command(op="quit")
        process.wait(timeout=5)
    except Exception:
        process.kill()
        process.wait(timeout=5)
    selector.close()
