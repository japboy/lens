"""Opt-in real ACP raster acceptance with synthetic sources and actual provider image tools.

Input JSON contains production-rendered preset turns. The explicit supplementary
user request selects raster output for a fictional ecology source. Provider mode is
unchanged, image-generation permissions are admitted, and actual ACP notifications
are retained privately for production replay. No image generator is mocked.
"""
import argparse
import asyncio
import json
import os
import sys
import tempfile
import uuid
from pathlib import Path


def completed_displayable_images(notifications):
    image_blocks = []
    tool_states = {}
    for notification in notifications:
        update = notification["params"]["update"]
        if update.get("sessionUpdate") == "agent_message_chunk":
            content = update.get("content", {})
            if content.get("type") == "image":
                image_blocks.append(content)
        elif update.get("sessionUpdate") in ("tool_call", "tool_call_update"):
            state = tool_states.setdefault(update["toolCallId"], {"status": None, "images": []})
            if "status" in update:
                state["status"] = update["status"]
            for wrapper in update.get("content", []):
                content = wrapper.get("content", {})
                if wrapper.get("type") == "content" and content.get("type") == "image":
                    state["images"].append(content)
    for state in tool_states.values():
        if state["status"] == "completed":
            image_blocks.extend(state["images"])
    return image_blocks


def verify_image_evidence_classifier():
    image = {"type": "image", "mimeType": "image/png", "data": "fixture"}
    def notification(update):
        return {"params": {"update": update}}
    content = [{"type": "content", "content": image}]
    pending = notification({"sessionUpdate": "tool_call", "toolCallId": "image", "status": "in_progress", "content": content})
    failed = notification({"sessionUpdate": "tool_call_update", "toolCallId": "image", "status": "failed"})
    completed = notification({"sessionUpdate": "tool_call_update", "toolCallId": "image", "status": "completed"})
    raw_only = notification({"sessionUpdate": "tool_call", "toolCallId": "raw", "status": "completed", "rawOutput": image})
    assert completed_displayable_images([pending]) == []
    assert completed_displayable_images([pending, failed]) == []
    assert completed_displayable_images([raw_only]) == []
    assert completed_displayable_images([pending, completed]) == [image]
    assert completed_displayable_images([pending, completed, failed]) == []
    assert completed_displayable_images([notification({"sessionUpdate": "agent_message_chunk", "content": image})]) == [image]
    print("Image evidence classifier: 6 checks passed")


if sys.argv[1:] == ["--self-test"]:
    verify_image_evidence_classifier()
    raise SystemExit(0)


parser = argparse.ArgumentParser(description=__doc__, epilog="Use --self-test alone to check evidence classification without a provider.")
parser.add_argument("--node", type=Path, required=True)
parser.add_argument("--adapter-entry", type=Path, required=True)
parser.add_argument("--prompts", type=Path, required=True)
parser.add_argument("--output", type=Path, required=True)
parser.add_argument("--preset", default="visual-learner")
args = parser.parse_args()
for path in (args.node, args.adapter_entry, args.prompts):
    if not path.is_absolute() or not path.is_file():
        parser.error("All input paths must be existing absolute files")
args.output = args.output.resolve()
args.output.mkdir(parents=True, exist_ok=True)
repo = Path(__file__).resolve().parent.parent


def observation(revision):
    contract = json.loads((repo / "apps/desktop/tests/fixtures/workspace-contracts.json").read_text())
    projection = json.loads(contract["projection_bytes"])
    source = projection["sources"][0]
    source["provenance"]["window_title"] = "Fictional woodland ecology"
    source["document"]["nodes"][1]["value"] = (
        "Fictional ecosystem: Mosslight woodland. Silver foxes live beside fern-covered fallen logs; "
        "amber moths pollinate the bell-shaped blue duskflowers. Beneath them, white fungi break "
        "down dead wood, returning nutrients to the soil. This is a fictional illustrated learning scene. "
        + ("The stream is dry in late summer." if revision == 1 else
           "Autumn rain has refilled the stream; fallen orange leaves collect along its banks. "
           "All species and their ecological relationships remain unchanged.")
    )
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
        env={**os.environ, "NODE_OPTIONS": "", "NODE_PATH": ""}, limit=2**26,
    )
    serial = 0
    chunks = []
    calls = []
    notifications = []

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
                    notifications.append(msg)
                    update = msg["params"]["update"]
                    if update.get("sessionUpdate") == "tool_call":
                        print(json.dumps({"event":"tool_call","title":update.get("title"),"kind":update.get("kind")}), flush=True)
                    if update.get("sessionUpdate") == "agent_message_chunk":
                        chunks.append(update.get("content", {}).get("text", ""))
                    elif update.get("sessionUpdate") in ("tool_call", "tool_call_update"):
                        calls.append(update)
                if "method" in msg and "id" in msg:
                    if msg["method"] == "session/request_permission":
                        p = msg["params"]
                        call = p.get("toolCall", {})
                        # Approval is gated on the tool kind, never on agent-supplied
                        # arguments: a substring search over the serialized call would
                        # auto-approve any mutating tool whose rawInput merely mentions
                        # one of these names.
                        kind = call.get("kind")
                        title = str(call.get("title") or "").lower()
                        generative = any(name in title for name in ["publish_html", "image_gen", "imagegen", "generate_image", "image generation"])
                        allowed = kind in ["read", "search", "fetch"] or (generative and kind not in ["edit", "delete", "move", "execute"])
                        print(json.dumps({"event":"permission","title":call.get("title"),"allowed":allowed}), flush=True)
                        option = next((o for o in p.get("options", []) if o["kind"] == "allow_once"), None)
                        result = {"outcome": {"outcome": "selected", "optionId": option["optionId"]}} if allowed and option else {"outcome": {"outcome": "cancelled"}}
                        await send({"jsonrpc": "2.0", "id": msg["id"], "result": result})
                    else:
                        await send({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32601, "message": "Unavailable in isolated acceptance"}})
            raise RuntimeError("Adapter ended")
        return await asyncio.wait_for(read(), 600)

    results = []
    try:
        await rpc("initialize", {"protocolVersion": 1, "clientInfo": {"name": "lens-prompt-acceptance", "version": "1"}, "clientCapabilities": {}})
        session = await rpc("session/new", {"cwd": str(cwd), "mcpServers": [{"name": "lens_output", "command": str(args.node), "args": [str(repo / "apps/desktop/tests/fixtures/mcp-prompt-output.mjs")], "env": [{"name": "LENS_PROMPT_TRACE", "value": str(trace)}, {"name": "LENS_PROMPT_TURN", "value": str(active)}]}]})
        sid = session["sessionId"]
        (destination / "session.json").write_text(json.dumps(session, indent=2))
        for turn in preset["turns"]:
            if turn["kind"] == "unchanged":
                continue
            notifications.clear()
            chunks.clear()
            calls.clear()
            tid = str(uuid.uuid4())
            active.write_text(tid)
            revision = 1 if turn["kind"] == "initial" else 2
            prompt = [{"type": "text", "text": turn["prompt"]}, {"type": "text", "text": (
                "Raster image acceptance request from the user: for this fictional ecological scene, "
                "create an actual raster infographic illustration using your imagegen skill and actual "
                "built-in image_gen image-generation capability. Read the applicable imagegen skill "
                "and invoke the actual image-generation tool; do not substitute HTML, SVG, Mermaid, "
                "a hand-drawn programming output, or an external API/CLI fallback. An illustrative raster "
                "image is explicitly requested for this case. You may create the generated output artifacts "
                "in this isolated working directory and return the image through the available output mechanism. "
                "For source updates preserve the previous scene composition while reflecting the changed season. "
                "If the tool is unavailable or fails, report that actual limitation without claiming success. "
                "Do not read unrelated personal files or modify the source or user settings."
            )}]
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
            replay = destination / (turn["kind"] + "-acp-updates.json")
            replay.write_text(json.dumps(notifications))
            print(json.dumps({"event":"replay_saved","path":str(replay)}), flush=True)
            image_blocks = completed_displayable_images(notifications)
            passed = response.get("stopReason") == "end_turn" and bool(prose.strip()) and bool(image_blocks)
            result = {"preset": preset["id"], "turn": turn["kind"], "passed": passed, "imageBlocks": len(image_blocks), "workingDirectory": str(cwd), "textBytes": len(prose.encode()), "stopReason": response.get("stopReason")}
            results.append(result)
            print(json.dumps(result), flush=True)
            if not passed:
                break
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
        (destination / "last-acp-updates.json").write_text(json.dumps(notifications))
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
    # `all([])` is true, so a preset whose turns were all skipped would report success
    # without having executed — or verified — a single acceptance turn.
    return bool(results) and all(item["passed"] for item in results)


raise SystemExit(0 if asyncio.run(main()) else 1)
