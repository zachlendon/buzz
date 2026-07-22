#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
workflow="${repo_root}/.github/workflows/docker.yml"
dockerfile="${repo_root}/Dockerfile"
relay_manifest="${repo_root}/crates/buzz-relay/Cargo.toml"
relay_workflow=$(mktemp)
trap 'rm -f "$relay_workflow"' EXIT

# docker.yml also publishes a separately scoped push-gateway image. Check only
# the relay jobs so that this contract cannot accidentally expand its boundary.
awk '/^  push-gateway-build:/{exit} {print}' "$workflow" >"$relay_workflow"

expected_image="IMAGE_NAME: \${{ vars.GHCR_IMAGE != '' && vars.GHCR_IMAGE || 'ghcr.io/zachlendon/buzz' }}"
if ! grep -Fqx "  ${expected_image}" "$relay_workflow"; then
  echo "relay publisher must default to ghcr.io/zachlendon/buzz while retaining GHCR_IMAGE override" >&2
  exit 1
fi

for label in source url documentation; do
  case "$label" in
    documentation) expected="https://github.com/zachlendon/buzz#readme" ;;
    *) expected="https://github.com/zachlendon/buzz" ;;
  esac
  if ! grep -Fq "org.opencontainers.image.${label}=\"${expected}\"" "$dockerfile"; then
    echo "relay Dockerfile has the wrong OCI ${label} label" >&2
    exit 1
  fi
done

if ! grep -Fq 'ghcr.io/zachlendon/buzz' "$relay_manifest"; then
  echo "relay package metadata does not identify the fork-owned image" >&2
  exit 1
fi

if grep -En 'ghcr\.io/block/buzz([^[:alnum:]_-]|$)|github\.com/block/buzz' \
  "$relay_workflow" "$dockerfile" "$relay_manifest"; then
  echo "relay publication surfaces still fall back to the upstream image or repository" >&2
  exit 1
fi

echo "relay image publication contract passed"
