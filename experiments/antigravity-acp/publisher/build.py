"""Build the probe in a disposable Cargo workspace outside the product tree."""
import argparse
import json
import pathlib
import shutil
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument("--target-dir", type=pathlib.Path, required=True)
args = parser.parse_args()
source = pathlib.Path(__file__).resolve().parent
repository = source.parents[2]
dependency = repository / "packages" / "adapter-output-mcp"
target = args.target_dir.resolve()
if target == repository or repository in target.parents:
    parser.error("target directory must be outside the product tree")
with tempfile.TemporaryDirectory(prefix="lens-antigravity-publisher-build-") as temporary:
    workspace = pathlib.Path(temporary)
    template = (source / "Cargo.toml.template").read_text()
    manifest = template.replace("@ADAPTER_OUTPUT_MCP_PATH@", json.dumps(str(dependency)))
    (workspace / "Cargo.toml").write_text(manifest)
    shutil.copyfile(source / "Cargo.lock", workspace / "Cargo.lock")
    (workspace / "src").mkdir()
    shutil.copyfile(source / "src" / "main.rs.template", workspace / "src" / "main.rs")
    subprocess.run([
        "cargo", "build", "--offline", "--locked", "--manifest-path",
        str(workspace / "Cargo.toml"), "--target-dir", str(target),
    ], check=True)
print(json.dumps({"binary": str(target / "debug" / "lens-antigravity-publisher-probe")}))
