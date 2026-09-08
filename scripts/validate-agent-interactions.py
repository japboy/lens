import argparse, asyncio, json, os, tempfile
from pathlib import Path

parser = argparse.ArgumentParser(
    description="Opt-in provider acceptance tests using isolated MCP fixtures; requires Agent authentication."
)
parser.add_argument("--agent", choices=["claude", "codex", "both"], default="both")
parser.add_argument(
    "--runtime-root",
    type=Path,
    default=Path.home()
    / "Library/Application Support/com.github.japboy.lens/agent-runtimes",
)
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
ROOT = args.runtime_root
REPO = Path(__file__).resolve().parent.parent
OUT = args.output
OUT.mkdir(parents=True, exist_ok=True)


async def probe(name, safe):
    package_name = "claude-agent-acp" if name == "claude" else "codex-acp"
    manifest = json.loads(
        (REPO / f"apps/desktop/src-tauri/agent-runtime/{name}/package.json").read_text()
    )
    version = manifest["dependencies"][f"@agentclientprotocol/{package_name}"]
    node_version = manifest["engines"]["node"]
    cwd = Path(tempfile.mkdtemp(prefix="lens-provider-"))
    (cwd / ".claude").mkdir()
    (cwd / ".claude/settings.json").write_text(
        json.dumps(
            {
                "permissions": {
                    "allow": [
                        "mcp__lens_fixture__fixture_read",
                        "mcp__lens_fixture__fixture_form",
                        "mcp__lens_fixture__fixture_url",
                    ]
                }
            }
        )
    )
    node = ROOT / f"node/v{node_version}-darwin-arm64/bin/node"
    entry = (
        ROOT
        / f"agents/{name}-acp/{version}/node_modules/@agentclientprotocol/{package_name}/dist/index.js"
    )
    proc = await asyncio.create_subprocess_exec(
        str(node),
        str(entry),
        stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.DEVNULL,
        env={**os.environ, "NODE_OPTIONS": "", "NODE_PATH": ""},
    )
    serial = 0
    text = ""
    counts = {"permission": 0, "form": 0, "url": 0}
    tool_kinds = {}

    async def send(value):
        proc.stdin.write((json.dumps(value) + "\n").encode())
        await proc.stdin.drain()

    async def rpc(method, params, timeout=150):
        nonlocal serial, text
        serial += 1
        ident = serial
        await send({"jsonrpc": "2.0", "id": ident, "method": method, "params": params})

        async def read():
            nonlocal text
            while True:
                line = await proc.stdout.readline()
                if not line:
                    raise RuntimeError("adapter_ended")
                msg = json.loads(line)
                if msg.get("id") == ident and ("result" in msg or "error" in msg):
                    if "error" in msg:
                        raise RuntimeError(
                            "ACP error "
                            + str(msg["error"].get("code"))
                            + ": "
                            + str(msg["error"].get("message"))
                        )
                    return msg["result"]
                if msg.get("method") == "session/update":
                    u = msg["params"]["update"]
                    if u.get("sessionUpdate") == "agent_message_chunk":
                        text += u.get("content", {}).get("text", "")
                    if u.get("sessionUpdate") in ["tool_call", "tool_call_update"]:
                        if u.get("kind"):
                            tool_kinds[u["toolCallId"]] = u["kind"]
                if "method" in msg and "id" in msg:
                    p = msg.get("params", {})
                    result = None
                    if msg["method"] == "session/request_permission":
                        counts["permission"] += 1
                        call = p["toolCall"]
                        kind = call.get("kind", tool_kinds.get(call["toolCallId"]))
                        option = next(
                            (o for o in p["options"] if o["kind"] == "allow_once"), None
                        )
                        result = (
                            {
                                "outcome": {
                                    "outcome": "selected",
                                    "optionId": option["optionId"],
                                }
                            }
                            if kind in ["read", "search", "fetch"] and option
                            else {"outcome": {"outcome": "cancelled"}}
                        )
                    elif msg["method"] in [
                        "session/create_elicitation",
                        "elicitation/create",
                    ]:
                        mode = p.get("mode", "form")
                        counts[mode] += 1
                        if p.get("_meta"):
                            await send(
                                {
                                    "jsonrpc": "2.0",
                                    "id": msg["id"],
                                    "error": {
                                        "code": -32602,
                                        "message": "Unsupported elicitation extensions",
                                    },
                                }
                            )
                            continue
                        result = (
                            {
                                "action": "accept",
                                "content": {"answer": "fixture-answer"},
                            }
                            if mode == "form"
                            else {"action": "decline"}
                        )
                    await send(
                        {"jsonrpc": "2.0", "id": msg["id"], "result": result}
                        if result
                        else {
                            "jsonrpc": "2.0",
                            "id": msg["id"],
                            "error": {
                                "code": -32601,
                                "message": "Unsupported validation request",
                            },
                        }
                    )

        return await asyncio.wait_for(read(), timeout)

    try:
        await rpc(
            "initialize",
            {
                "protocolVersion": 1,
                "clientInfo": {"name": "lens-validation", "version": "1"},
                "clientCapabilities": {
                    "auth": {"terminal": True},
                    "elicitation": {"form": {}, "url": {}},
                },
            },
        )
        session = await rpc(
            "session/new",
            {
                "cwd": str(cwd),
                "mcpServers": [
                    {
                        "name": "lens_fixture",
                        "command": str(node),
                        "args": [str(REPO / "apps/desktop/tests/fixtures/mcp-interactions.mjs")],
                        "env": [
                            {
                                "name": "LENS_FIXTURE_TRACE",
                                "value": str(cwd / "invocations.jsonl"),
                            }
                        ],
                    }
                ],
            },
        )
        sid = session["sessionId"]
        options = session.get("configOptions", [])
        mode = next(o for o in options if o.get("category") == "mode")
        await rpc(
            "session/set_config_option",
            {"sessionId": sid, "configId": mode["id"], "value": safe},
        )
        model = next((o for o in options if o.get("category") == "model"), None)
        if (
            name == "codex"
            and model
            and any(o.get("value") == "gpt-5.6-sol" for o in model.get("options", []))
        ):
            await rpc(
                "session/set_config_option",
                {"sessionId": sid, "configId": model["id"], "value": "gpt-5.6-sol"},
            )
        results = []
        for tool, expected in [
            ("fixture_read", "LENS_FIXTURE_READ_7B4A"),
            ("fixture_form", "LENS_FIXTURE_FORM_ACCEPTED_7B4A"),
            ("fixture_url", "LENS_FIXTURE_URL_DECLINED_7B4A"),
            ("fixture_approval", "LENS_FIXTURE_APPROVAL_DENIED_7B4A"),
        ]:
            text = ""
            response = await rpc(
                "session/prompt",
                {
                    "sessionId": sid,
                    "prompt": [
                        {
                            "type": "text",
                            "text": f"Acceptance test in an isolated directory. Call the lens_fixture MCP tool {tool} exactly once with empty arguments. Do not use any other tool, read any files, or change anything. Return the exact text returned by the tool. If an interaction is declined, still report its returned text.",
                        }
                    ],
                },
            )
            invocations = (
                [
                    json.loads(line)["tool"]
                    for line in (cwd / "invocations.jsonl").read_text().splitlines()
                ]
                if (cwd / "invocations.jsonl").exists()
                else []
            )
            passed = (
                tool not in invocations and counts["form"] > 1
                if tool == "fixture_approval"
                else expected in text
            )
            results.append(
                {
                    "case": tool,
                    "passed": passed,
                    "stopReason": response.get("stopReason"),
                    "output": text[:400],
                }
            )
            print(json.dumps({"adapter": name, **results[-1]}), flush=True)
        (OUT / f"{name}.json").write_text(
            json.dumps(
                {
                    "adapter": name,
                    "version": version,
                    "counts": counts,
                    "results": results,
                },
                indent=2,
            )
        )
        print(json.dumps({"adapter": name, "counts": counts}), flush=True)
        return all(result["passed"] for result in results)
    except Exception as e:
        (OUT / f"{name}.json").write_text(
            json.dumps({"adapter": name, "error": str(e), "counts": counts}, indent=2)
        )
        print(
            json.dumps({"adapter": name, "error": str(e), "counts": counts}), flush=True
        )
        return False
    finally:
        if proc.returncode is None:
            proc.terminate()
            try:
                await asyncio.wait_for(proc.wait(), 5)
            except asyncio.TimeoutError:
                proc.kill()
                await proc.wait()


async def main():
    results = []
    if args.agent in ["codex", "both"]:
        results.append(await probe("codex", "read-only"))
    if args.agent in ["claude", "both"]:
        results.append(await probe("claude", "plan"))
    return all(results)


raise SystemExit(0 if asyncio.run(main()) else 1)
