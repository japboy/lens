#!/bin/sh

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
NODE_VERSION=24.19.0
NODE_ARCHIVE=node-v${NODE_VERSION}-darwin-arm64.tar.gz
NODE_SHA256=8294b7aa9b03997481c06babf1e8b270c859358f27da57a11509afe537ac381d
DOWNLOAD_DIR=${SCRIPT_DIR}/.downloads
RUNTIME_DIR=${SCRIPT_DIR}/runtime
NODE_URL=https://nodejs.org/dist/v${NODE_VERSION}/${NODE_ARCHIVE}
NODE_ARCHIVE_PATH=${DOWNLOAD_DIR}/${NODE_ARCHIVE}

mkdir -p "${DOWNLOAD_DIR}" "${RUNTIME_DIR}"

if [ ! -f "${NODE_ARCHIVE_PATH}" ]; then
  curl --fail --location --silent --show-error "${NODE_URL}" --output "${NODE_ARCHIVE_PATH}"
fi

ACTUAL_SHA256=$(shasum -a 256 "${NODE_ARCHIVE_PATH}" | awk '{print $1}')
if [ "${ACTUAL_SHA256}" != "${NODE_SHA256}" ]; then
  echo "Node archive checksum mismatch" >&2
  exit 1
fi

tar -xzf "${NODE_ARCHIVE_PATH}" \
  -C "${RUNTIME_DIR}" \
  --strip-components=2 \
  node-v${NODE_VERSION}-darwin-arm64/bin/node
mv "${RUNTIME_DIR}/node" "${RUNTIME_DIR}/node-aarch64-apple-darwin"
chmod 755 "${RUNTIME_DIR}/node-aarch64-apple-darwin"

cd "${SCRIPT_DIR}"
npm ci --omit=dev --ignore-scripts

"${RUNTIME_DIR}/node-aarch64-apple-darwin" \
  "${SCRIPT_DIR}/node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js" \
  --version
"${RUNTIME_DIR}/node-aarch64-apple-darwin" \
  "${SCRIPT_DIR}/node_modules/@agentclientprotocol/codex-acp/dist/index.js" \
  --version
