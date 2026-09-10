"""Opt-in real ACP prompt acceptance with synthetic sources and an isolated HTML sink.

Input JSON: [{"id": "...", "turns": [{"kind": "initial|update|retry|unchanged",
"prompt": "production-rendered instruction"}]}]. Outputs retain synthetic responses
and HTML for human quality review; automated checks prove transport, not design quality.
"""
import argparse
import asyncio
import json
import os
import tempfile
import uuid
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--agent", choices=["codex", "claude"], default="codex")
parser.add_argument("--node", type=Path, required=True)
parser.add_argument("--adapter-entry", type=Path, required=True)
parser.add_argument("--prompts", type=Path, required=True)
parser.add_argument("--output", type=Path, required=True)
parser.add_argument("--preset", help="Run only this preset id")
args = parser.parse_args()
for path in (args.node, args.adapter_entry, args.prompts):
    if not path.is_absolute() or not path.is_file():
        parser.error("All input paths must be existing absolute files")
args.output.mkdir(parents=True, exist_ok=True)
repo = Path(__file__).resolve().parent.parent


def observation(revision):
    contract = json.loads((repo / "apps/desktop/tests/fixtures/workspace-contracts.json").read_text())
    projection = json.loads(contract["projection_bytes"])
    source = projection["sources"][0]
    source["provenance"]["window_title"] = "Synthetic monthly service pricing"
    source["document"]["nodes"][1]["value"] = (
        "Monthly cost C(x) = F + p*x, where x is usage in GB. "
        "Plan Cedar: fixed fee $10/month, unit price $2/GB. "
        f"Plan Maple: fixed fee $25/month, unit price ${1 if revision == 1 else 0.5}/GB. "
        "Compare cost at 10 GB and the break-even usage. Prices exclude tax. "
        "These are complete fictional rates for this exercise; no external verification is needed."
    )
    if revision == 3:
        source["document"]["nodes"][1]["value"] += " Editorial note: wording reviewed; all rates are unchanged."
    return projection


async def probe(preset):
    destination = args.output / preset["id"]
    destination.mkdir(exist_ok=True)
    cwd = Path(tempfile.mkdtemp(prefix="lens-prompt-acceptance-"))
    trace = destination / "publications.jsonl"
    trace.write_text("")
    active = destination / "active-turn.txt"
    proc = await asyncio.create_subprocess_exec(
        str(args.node), str(args.adapter_entry),
        stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
        stderr=(destination / "adapter.stderr").open("wb"),
        env={**os.environ, "NODE_OPTIONS": "", "NODE_PATH": ""}, limit=2**22,
    )
    serial = 0
    chunks = []
    calls = []

    async def send(value):
        proc.stdin.write((json.dumps(value) + "\n").encode())
        await proc.stdin.drain()

    async def rpc(method, params):
        nonlocal serial
        serial += 1
        ident = serial
        await send({"jsonrpc": "2.0", "id": ident, "method": method, "params": params})
        async def read():
            while line := await proc.stdout.readline():
                msg = json.loads(line)
                if msg.get("id") == ident and ("result" in msg or "error" in msg):
                    if "error" in msg:
                        raise RuntimeError(json.dumps(msg["error"]))
                    return msg["result"]
                if msg.get("method") == "session/update":
                    update = msg["params"]["update"]
                    if update.get("sessionUpdate") == "agent_message_chunk":
                        chunks.append(update.get("content", {}).get("text", ""))
                    elif update.get("sessionUpdate") in ("tool_call", "tool_call_update"):
                        calls.append(update)
                if "method" in msg and "id" in msg:
                    if msg["method"] == "session/request_permission":
                        p = msg["params"]
                        call = p.get("toolCall", {})
                        allowed = "publish_html" in json.dumps(call)
                        option = next((o for o in p.get("options", []) if o["kind"] == "allow_once"), None)
                        result = {"outcome": {"outcome": "selected", "optionId": option["optionId"]}} if allowed and option else {"outcome": {"outcome": "cancelled"}}
                        await send({"jsonrpc": "2.0", "id": msg["id"], "result": result})
                    else:
                        await send({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32601, "message": "Unavailable in isolated acceptance"}})
            raise RuntimeError("Adapter ended")
        return await asyncio.wait_for(read(), 240)

    results = []
    try:
        await rpc("initialize", {"protocolVersion": 1, "clientInfo": {"name": "lens-prompt-acceptance", "version": "1"}, "clientCapabilities": {}})
        session = await rpc("session/new", {"cwd": str(cwd), "mcpServers": [{"name": "lens_output", "command": str(args.node), "args": [str(repo / "apps/desktop/tests/fixtures/mcp-prompt-output.mjs")], "env": [{"name": "LENS_PROMPT_TRACE", "value": str(trace)}, {"name": "LENS_PROMPT_TURN", "value": str(active)}]}]})
        sid = session["sessionId"]
        (destination / "session.json").write_text(json.dumps(session, indent=2))
        for option in session.get("configOptions", []):
            if option.get("category") == "mode":
                await rpc("session/set_config_option", {"sessionId": sid, "configId": option["id"], "value": {"codex": "read-only", "claude": "plan"}[args.agent]})
        for turn in preset["turns"]:
            chunks.clear()
            calls.clear()
            tid = str(uuid.uuid4())
            active.write_text(tid)
            revision = 1 if turn["kind"] == "initial" else 3 if turn["kind"] == "unchanged" else 2
            prompt = [{"type": "text", "text": turn["prompt"]}]
            if turn["kind"] != "retry":
                prompt.append({"type": "text", "text": json.dumps(observation(revision))})
            prompt.append({"type": "text", "text": json.dumps({"kind": "lens_output_publication", "schema_version": 1, "turn_id": tid})})
            response = await rpc("session/prompt", {"sessionId": sid, "prompt": prompt})
            prose = "".join(chunks)
            publications = [json.loads(line) for line in trace.read_text().splitlines() if json.loads(line)["turn_id"] == tid]
            html = publications[-1]["html"] if publications else ""
            (destination / (turn["kind"] + ".txt")).write_text(prose)
            (destination / (turn["kind"] + ".html")).write_text(html)
            (destination / (turn["kind"] + "-tools.json")).write_text(json.dumps(calls, indent=2))
            passed = response.get("stopReason") == "end_turn" and bool(prose.strip()) and bool(html.strip())
            result = {"preset": preset["id"], "turn": turn["kind"], "passed": passed, "htmlBytes": len(html.encode()), "textBytes": len(prose.encode()), "stopReason": response.get("stopReason")}
            results.append(result)
            print(json.dumps(result), flush=True)
    except Exception as error:
        results.append({"preset": preset["id"], "passed": False, "error": f"{type(error).__name__}: {error}"})
        print(json.dumps(results[-1]), flush=True)
    finally:
        if proc.returncode is None:
            proc.terminate()
            try:
                await asyncio.wait_for(proc.wait(), 5)
            except asyncio.TimeoutError:
                proc.kill()
                await proc.wait()
        (destination / "results.json").write_text(json.dumps(results, indent=2))
    return results


async def main():
    presets = json.loads(args.prompts.read_text())
    if args.preset:
        presets = [p for p in presets if p["id"] == args.preset]
    if not presets:
        raise ValueError("No matching presets")
    results = []
    for preset in presets:
        results.extend(await probe(preset))
    return all(item["passed"] for item in results)


raise SystemExit(0 if asyncio.run(main()) else 1)
