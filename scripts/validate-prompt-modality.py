"""Opt-in neutral-format ACP experiment using production prompts and fixed sources.

No supplementary output-format instruction is added. Results describe observed
choices, not a requirement to select images. Each case starts a fresh session.
Raw notifications can contain image data and are saved with private permissions.
"""
import argparse
import asyncio
import hashlib
import json
import os
import re
import tempfile
import uuid
from pathlib import Path

CASES = {
    "monet-water-garden": {
        "title": "Water lily pond — Claude Monet's house and gardens",
        "source_url": "https://claudemonetgiverny.fr/en/decouvrir/water-lily-pond/",
        "text": (
            "Source summary: Claude Monet's water garden at Giverny. Monet was fascinated "
            "by changing light and clouds reflected upside down on water. In 1893 he "
            "acquired land beyond the railway and diverted a branch of the Epte to form "
            "the pond. A Japanese bridge aligned with the main garden path was painted "
            "green instead of the traditional red. Bamboo, ginkgo, maples, Japanese tree "
            "peonies, lilies and weeping willows frame the water. Water lilies occupy the "
            "pond, where reflected sky and floating patches of flower colour share the "
            "same surface. Monet spent hours contemplating this garden. From 1897 his "
            "water-lily paintings explored the atmosphere of the reflective surface, "
            "eventually approaching abstraction through shimmering colours and the "
            "sensations and emotions they evoke."
        ),
    },
    "white-sands-light": {
        "title": "Photography — White Sands National Park",
        "source_url": "https://www.nps.gov/whsa/planyourvisit/photography.htm",
        "text": (
            "Source summary: The US National Park Service describes the changing light "
            "and shadows of White Sands. Near sunrise and sunset, clouds change colour "
            "and the Sacramento Mountains to the east receive a pink afterglow after "
            "sunset. A high dune gives a broad view; from a low viewpoint, yuccas stand "
            "against the sky and fine dune grasses become prominent. Close views reveal "
            "sand patterns made by windblown grass and leaves, sometimes with tiny beetle "
            "tracks. A nearby plant or rock supplies a sense of scale and distance. "
            "Layering the dunes against mountains that are darker or lighter creates "
            "contrast and depth. An off-centre subject or partial framing changes the "
            "composition. Footprints interrupt undisturbed lines in the sand."
        ),
    },
    "rain-garden": {
        "title": "A fictional residential rain garden",
        "text": (
            "Design notes for a fictional residential rain garden. A 6 m by 4 m shallow oval basin "
            "lies downhill of a roof downpipe, 5 m from the house foundation. A gently sloping "
            "stone-lined channel carries runoff from the downpipe into the west end. The basin "
            "floor is 150 mm below the surrounding lawn; side slopes are 1:4. From the surface "
            "downward, the basin has 50 mm of mulch, 450 mm of sandy planting soil and the "
            "existing permeable subsoil. Water spreads across the basin, temporarily ponds, "
            "then infiltrates downward; a low overflow notch on the east side routes excess "
            "water to a lawn swale away from the house. Sedges occupy the wet central floor, "
            "blue flag irises the lower slopes, and drought-tolerant grasses the drier rim. "
            "The plants slow surface flow and their roots maintain soil pore spaces. "
            "After ordinary rainfall the basin drains within 24 hours. These are assumed "
            "site conditions for this example, not universal construction specifications."
        ),
    },
    "pricing": {
        "title": "Fictional subscription pricing",
        "text": (
            "A fictional service offers three monthly plans, with no taxes or other charges. "
            "Basic costs $12 and includes 100 requests; each additional request costs $0.20. "
            "Plus costs $30 and includes 300 requests; each additional request costs $0.10. "
            "Pro costs $60 and includes 800 requests; each additional request costs $0.05. "
            "Unused requests expire monthly. There are no annual discounts or feature differences. "
            "At 50 requests the costs are $12, $30 and $60; at 250 requests they are $42, "
            "$30 and $60; at 600 requests they are $112, $60 and $60; at 1000 requests "
            "they are $192, $100 and $70, respectively. Cost within each allowance stays flat; "
            "above it, cost grows linearly at that plan's overage rate."
        ),
    },
}
REPO = Path(__file__).resolve().parent.parent


def completed_images(updates):
    images, states = [], {}
    for item in updates:
        update = item.get("params", {}).get("update", {})
        if update.get("sessionUpdate") == "agent_message_chunk":
            block = update.get("content", {})
            if block.get("type") == "image" and block.get("data") and block.get("mimeType"):
                images.append(block)
        elif update.get("sessionUpdate") in ("tool_call", "tool_call_update"):
            state = states.setdefault(update["toolCallId"], {"status": None, "images": []})
            state["status"] = update.get("status", state["status"])
            for wrapper in update.get("content", []):
                block = wrapper.get("content", {})
                if (wrapper.get("type") == "content" and block.get("type") == "image"
                        and block.get("data") and block.get("mimeType")):
                    state["images"].append(block)
    for state in states.values():
        if state["status"] == "completed":
            images.extend(state["images"])
    return images


def tool_evidence(updates):
    states, skill_reads = {}, []
    for index, item in enumerate(updates):
        update = item.get("params", {}).get("update", {})
        if update.get("sessionUpdate") not in ("tool_call", "tool_call_update"):
            continue
        ident = update["toolCallId"]
        state = states.setdefault(ident, {"id": ident, "firstUpdate": index})
        for key in ("title", "kind", "status"):
            if key in update:
                state[key] = update[key]
        # Only tool invocation fields are evidence of a read; prose/output can
        # mention a skill without invoking it.
        invocation = json.dumps({key: update.get(key) for key in ("title", "rawInput")})
        for skill in ("visualize", "imagegen"):
            if re.search(rf"{skill}/(?:[^\s\"']*/)?SKILL\.md", invocation):
                if not any(read["toolCallId"] == ident and read["skill"] == skill for read in skill_reads):
                    skill_reads.append({"skill": skill, "toolCallId": ident, "updateIndex": index})
    return list(states.values()), skill_reads


def permission_allowed(call):
    name = str(call.get("title", "")).lower()
    if name == "image generation":
        return True
    raw = call.get("rawInput", {})
    tool_name = str(raw.get("name", raw.get("tool", ""))).lower() if isinstance(raw, dict) else ""
    # Match tool identity, never arbitrary descriptions or prompts containing a keyword.
    if any(re.fullmatch(r"(?:[a-z0-9_]+(?:[.:/]|__))*" + tool, candidate)
           for candidate in (name, tool_name)
           for tool in ("publish_html", "imagegen", "image_gen", "generate_image")):
        return True
    command = raw.get("command", raw.get("cmd", "")) if isinstance(raw, dict) else ""
    if isinstance(command, list):
        command = " ".join(command)
    if not command:
        command = name
    # Read-only skill discovery. Reject shell composition, substitutions and redirection.
    return bool(
        isinstance(command, str)
        and not re.search(r"[;&|><\x60$\n]", command)
        and re.fullmatch(r"(?:cat|head|sed -n ['\"]?[0-9,]+p['\"]?)\s+[^\s]+/SKILL\.md", command)
    )


def observation(case):
    contract = json.loads((REPO / "apps/desktop/tests/fixtures/workspace-contracts.json").read_text())
    projection = json.loads(contract["projection_bytes"])
    source = projection["sources"][0]
    source["provenance"]["window_title"] = CASES[case]["title"]
    source["document"]["nodes"][1]["value"] = CASES[case]["text"] + ("\nSource: " + CASES[case]["source_url"] if "source_url" in CASES[case] else "")
    return projection


def private_write(path, value):
    with path.open("w", encoding="utf-8") as stream:
        os.chmod(path, 0o600)
        stream.write(value)


def self_test():
    image = {"type": "image", "mimeType": "image/png", "data": "fixture"}
    def event(**update):
        return {"params": {"update": update}}
    pending = event(sessionUpdate="tool_call", toolCallId="i", status="in_progress",
                    content=[{"type": "content", "content": image}])
    failed = event(sessionUpdate="tool_call_update", toolCallId="i", status="failed")
    done = event(sessionUpdate="tool_call_update", toolCallId="i", status="completed")
    assert completed_images([pending]) == []
    assert completed_images([pending, failed]) == []
    assert completed_images([event(sessionUpdate="tool_call", toolCallId="r",
                                  status="completed", rawOutput=image)]) == []
    assert completed_images([pending, done]) == [image]
    assert completed_images([pending, done, failed]) == []
    assert completed_images([event(sessionUpdate="agent_message_chunk", content=image)]) == [image]
    assert completed_images([event(sessionUpdate="user_message_chunk", content=image)]) == []
    for case in CASES.values():
        assert not re.search(r"imagegen|visualize|html|svg|generate|render|draw|infographic", case["text"], re.I)
    assert permission_allowed({"title": "mcp__lens_output__publish_html"})
    assert permission_allowed({"title": "publish_html"})
    assert permission_allowed({"title": "Image generation"})
    assert not permission_allowed({"title": "Run arbitrary command for image generation"})
    assert not permission_allowed({"title": "exec", "rawInput": {"command": "rm -rf /tmp/imagegen"}})
    assert not permission_allowed({"title": "exec", "rawInput": {"command": "cat /a/SKILL.md; touch /tmp/x"}})
    assert permission_allowed({"title": "exec", "rawInput": {"command": "cat /a/SKILL.md"}})
    tools, reads = tool_evidence([event(sessionUpdate="tool_call", toolCallId="r", title="Read",
                                       rawInput={"command": "cat /a/visualize/1/skills/visualize/SKILL.md"})])
    assert len(tools) == 1 and reads[0]["skill"] == "visualize"
    print("Neutral modality self-test passed")


async def probe(args, preset, case):
    destination = args.output / preset["id"] / case
    destination.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(destination, 0o700)
    cwd = Path(tempfile.mkdtemp(prefix="lens-neutral-modality-"))
    trace, active = destination / "publications.jsonl", destination / "active-turn.txt"
    private_write(trace, "")
    turn_id = str(uuid.uuid4())
    private_write(active, turn_id)
    initial = next(turn for turn in preset["turns"] if turn["kind"] == "initial")
    prompt = [
        {"type": "text", "text": initial["prompt"]},
        {"type": "text", "text": json.dumps(observation(case))},
        {"type": "text", "text": json.dumps({"kind": "lens_output_publication", "schema_version": 1, "turn_id": turn_id})},
    ]
    private_write(destination / "input.json", json.dumps(prompt, indent=2))
    notifications, permissions = [], []
    serial = 0
    stderr = (destination / "adapter.stderr").open("wb")
    os.chmod(destination / "adapter.stderr", 0o600)
    proc = await asyncio.create_subprocess_exec(
        str(args.node), str(args.adapter_entry), stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE, stderr=stderr,
        env={**os.environ, "NODE_OPTIONS": "", "NODE_PATH": ""}, limit=2**26,
    )
    async def send(value):
        proc.stdin.write((json.dumps(value) + "\n").encode())
        await proc.stdin.drain()
    async def rpc(method, params):
        nonlocal serial
        serial += 1
        ident = serial
        await send({"jsonrpc": "2.0", "id": ident, "method": method, "params": params})
        while line := await proc.stdout.readline():
            message = json.loads(line)
            if message.get("id") == ident and ("result" in message or "error" in message):
                if "error" in message:
                    raise RuntimeError(json.dumps(message["error"]))
                return message["result"]
            if message.get("method") == "session/update":
                notifications.append(message)
                if message["params"]["update"].get("sessionUpdate") == "tool_call":
                    update = message["params"]["update"]
                    print(json.dumps({"event": "tool_call", "preset": preset["id"], "case": case,
                                      "title": update.get("title"), "kind": update.get("kind")}), flush=True)
            if "method" in message and "id" in message:
                if message["method"] == "session/request_permission":
                    params = message["params"]
                    call = params.get("toolCall", {})
                    allowed = permission_allowed(call)
                    option = next((o for o in params.get("options", []) if o["kind"] == "allow_once"), None)
                    permissions.append({"toolCall": call, "allowed": allowed and bool(option)})
                    result = {"outcome": {"outcome": "selected", "optionId": option["optionId"]}} if allowed and option else {"outcome": {"outcome": "cancelled"}}
                    await send({"jsonrpc": "2.0", "id": message["id"], "result": result})
                else:
                    await send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32601, "message": "Unavailable in isolated acceptance"}})
        raise RuntimeError("Adapter ended")
    result = {"preset": preset["id"], "case": case, "passed": False,
              "productionPromptSha256": hashlib.sha256(initial["prompt"].encode()).hexdigest(),
              "observationSha256": hashlib.sha256(prompt[1]["text"].encode()).hexdigest(),
              "workingDirectory": str(cwd), "providerModeChanged": False}
    try:
        async def run():
            await rpc("initialize", {"protocolVersion": 1, "clientInfo": {"name": "lens-neutral-modality", "version": "1"}, "clientCapabilities": {}})
            servers = [{
                "name": "lens_output", "command": str(args.node),
                "args": [str(REPO / "apps/desktop/tests/fixtures/mcp-prompt-output.mjs")],
                "env": [{"name": "LENS_PROMPT_TRACE", "value": str(trace)}, {"name": "LENS_PROMPT_TURN", "value": str(active)}],
            }]
            session_request = {"cwd": str(cwd), "mcpServers": servers}
            private_write(destination / "session-request.json", json.dumps(session_request, indent=2))
            session = await rpc("session/new", session_request)
            private_write(destination / "session.json", json.dumps(session, indent=2))
            result["sessionId"] = session["sessionId"]
            result["sessionMode"] = session.get("modes")
            result["sessionModels"] = session.get("models")
            result["sessionConfigOptions"] = session.get("configOptions")
            return await rpc("session/prompt", {"sessionId": session["sessionId"], "prompt": prompt})
        response = await asyncio.wait_for(run(), 600)
        result["stopReason"] = response.get("stopReason")
    except Exception as error:
        result["error"] = f"{type(error).__name__}: {error}"
    finally:
        if proc.returncode is None:
            proc.terminate()
            try:
                await asyncio.wait_for(proc.wait(), 5)
            except asyncio.TimeoutError:
                proc.kill()
                await proc.wait()
        stderr.close()
        private_write(destination / "acp-updates.json", json.dumps(notifications))
        private_write(destination / "permissions.json", json.dumps(permissions, indent=2))
    prose = "".join(item["params"]["update"].get("content", {}).get("text", "")
                    for item in notifications
                    if item["params"]["update"].get("sessionUpdate") == "agent_message_chunk")
    publications = [json.loads(line) for line in trace.read_text().splitlines()]
    publications = [entry for entry in publications if entry["turn_id"] == turn_id]
    images = completed_images(notifications)
    tools, reads = tool_evidence(notifications)
    modalities = (["image"] if images else []) + (["html"] if publications else [])
    if not modalities and prose.strip():
        modalities = ["text"]
    result.update({"modalities": modalities, "imageBlocks": len(images),
                   "publicationCount": len(publications), "textBytes": len(prose.encode()),
                   "tools": tools, "observedSkillReads": reads,
                   "skillReadEvidence": "observed invocation paths" if reads else "unknown",
                   "permissionRejections": sum(not permission["allowed"] for permission in permissions)})
    result["passed"] = result.get("stopReason") == "end_turn" and bool(modalities)
    private_write(destination / "prose.txt", prose)
    if publications:
        private_write(destination / "output.html", publications[-1]["html"])
    private_write(destination / "results.json", json.dumps(result, indent=2))
    print(json.dumps(result), flush=True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    for name in ("node", "adapter-entry", "prompts", "output"):
        parser.add_argument("--" + name, type=Path)
    parser.add_argument("--preset")
    parser.add_argument("--case", choices=CASES)
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    for path in (args.node, args.adapter_entry, args.prompts):
        if path is None or not path.is_absolute() or not path.is_file():
            parser.error("--node, --adapter-entry and --prompts must be existing absolute files")
    if args.output is None:
        parser.error("--output is required")
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True, mode=0o700)
    presets = json.loads(args.prompts.read_text())
    presets = [p for p in presets if not args.preset or p["id"] == args.preset]
    if not presets:
        parser.error("No matching presets")
    async def run():
        results = []
        for preset in presets:
            for case in ([args.case] if args.case else CASES):
                results.append(await probe(args, preset, case))
        private_write(args.output / "results.json", json.dumps(results, indent=2))
        return all(result["passed"] for result in results)
    return 0 if asyncio.run(run()) else 1


if __name__ == "__main__":
    raise SystemExit(main())
