#!/usr/bin/env bash
# Regenerate the pinned Codex app-server protocol schemas the Rust tests validate against.
#
#   scripts/update-codex-schemas.sh            refresh the current pinned version
#   scripts/update-codex-schemas.sh 0.150.0    move the pin to another Codex release
#
# Codex runs once through pnpm; it needs no login and makes no model calls.
# Add a name to SCHEMAS when the tests start validating a new message type.
set -euo pipefail
cd "$(dirname "$0")/.."

out=fixtures/codex-schemas
version="${1:-$(cat "$out/VERSION")}"
SCHEMAS=(
  ClientRequest
  CommandExecutionRequestApprovalParams
  CommandExecutionRequestApprovalResponse
  ConfigReadParams
  FileChangeRequestApprovalParams
  FileChangeRequestApprovalResponse
  InitializeParams
  McpServerElicitationRequestParams
  McpServerElicitationRequestResponse
  PermissionsRequestApprovalParams
  PermissionsRequestApprovalResponse
  ServerRequest
  ThreadResumeParams
  ThreadStartParams
  TurnInterruptParams
  TurnStartParams
)

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
pnpm dlx "@openai/codex@$version" app-server generate-json-schema --experimental --out "$tmp"

rm -rf "$out"
mkdir -p "$out"
for name in "${SCHEMAS[@]}"; do
  matches=()
  while IFS= read -r file; do matches+=("$file"); done < <(find "$tmp" -name "$name.json")
  if ((${#matches[@]} != 1)); then
    echo "Codex $version: expected one $name.json schema, found ${#matches[@]}" >&2
    exit 1
  fi
  cp "${matches[0]}" "$out/$name.json"
done
echo "$version" >"$out/VERSION"
echo "Pinned ${#SCHEMAS[@]} Codex $version protocol schemas in $out."
