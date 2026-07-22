#!/usr/bin/env bash
# Literal GitHub expressions and Dockerfile continuation backslashes are the
# values under test; they must not expand in this shell.
# shellcheck disable=SC2016,SC1003
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
contract_failed=0

fail() {
  echo "relay image publication contract: $1" >&2
  contract_failed=1
  return 0
}

require_line() {
  local file=$1
  local expected=$2
  local description=$3
  local count
  count=$(grep -Fxc -- "$expected" "$file" || true)
  [[ "$count" == 1 ]] || fail "$description (found $count exact matches)"
}

require_block_line() {
  local block=$1
  local expected=$2
  local description=$3
  local count
  count=$(grep -Fxc -- "$expected" <<<"$block" || true)
  [[ "$count" == 1 ]] || fail "$description (found $count exact matches)"
}

check_contract() {
  local root=$1
  local workflow="$root/.github/workflows/docker.yml"
  local ci="$root/.github/workflows/ci.yml"
  local dockerfile="$root/Dockerfile"
  local relay_manifest="$root/crates/buzz-relay/Cargo.toml"
  local trigger_block relay_workflow build_job merge_job gateway_build gateway_merge
  contract_failed=0

  trigger_block=$(awk '/^on:/{capture=1} /^# One image build/{exit} capture' "$workflow")
  relay_workflow=$(awk '/^  push-gateway-build:/{exit} {print}' "$workflow")
  build_job=$(awk '/^  build:/{capture=1} /^  merge:/{exit} capture' "$workflow")
  merge_job=$(awk '/^  merge:/{capture=1} /^  push-gateway-build:/{exit} capture' "$workflow")
  gateway_build=$(awk '/^  push-gateway-build:/{capture=1} /^  push-gateway-merge:/{exit} capture' "$workflow")
  gateway_merge=$(awk '/^  push-gateway-merge:/{capture=1} capture' "$workflow")

  require_block_line "$trigger_block" '    branches: [main]' \
    'docker publisher must have exactly the main branch push trigger'
  [[ $(grep -Ec '^    branches:' <<<"$trigger_block" || true) == 1 ]] || \
    fail 'docker publisher has an additional branch push trigger'

  require_block_line "$relay_workflow" \
    "  IMAGE_NAME: \${{ vars.GHCR_IMAGE != '' && vars.GHCR_IMAGE || 'ghcr.io/zachlendon/buzz' }}" \
    'relay publisher must retain GHCR_IMAGE override and default to the fork package'
  if grep -Fq "github.repository == 'block/buzz'" <<<"$relay_workflow"; then
    fail 'upstream repository guard must not disable the fork relay jobs'
  fi
  require_block_line "$merge_job" "    if: github.event_name != 'pull_request'" \
    'relay manifest job must run for fork publication events'

  require_block_line "$build_job" '      packages: write    # push to GHCR' \
    'relay build job must retain packages: write'
  require_block_line "$merge_job" '      packages: write    # push the merged manifest' \
    'relay manifest job must retain packages: write'
  require_block_line "$build_job" '          username: ${{ github.repository_owner }}' \
    'relay build login must use github.repository_owner'
  require_block_line "$merge_job" '          username: ${{ github.repository_owner }}' \
    'relay manifest login must use github.repository_owner'

  require_block_line "$build_job" \
    "          outputs: type=image,name=\${{ env.IMAGE_NAME }},push-by-digest=true,name-canonical=true,push=\${{ github.event_name != 'pull_request' }}" \
    'relay build must push the configured image by digest'
  require_block_line "$build_job" '          DIGEST: ${{ steps.build.outputs.digest }}' \
    'relay digest artifact must originate from the build output'
  require_block_line "$build_job" '          touch "/tmp/digests/${DIGEST#sha256:}"' \
    'relay digest artifact filename must carry the build digest'
  require_block_line "$merge_job" '          pattern: digests-*' \
    'relay manifest must download the per-architecture digest artifacts'
  require_block_line "$merge_job" '            digests+=("${IMAGE_NAME}@sha256:${digest}")' \
    'relay manifest inputs must bind the configured image to each build digest'
  require_block_line "$merge_job" '          echo "digest=${merged_digest}" >> "$GITHUB_OUTPUT"' \
    'relay manifest digest must be exported from the merged manifest inspection'
  require_block_line "$merge_job" '          subject-name: ${{ env.IMAGE_NAME }}' \
    'relay attestation subject must bind to the configured image'
  require_block_line "$merge_job" '          subject-digest: ${{ steps.manifest.outputs.digest }}' \
    'relay attestation digest must bind to the merged manifest output'
  require_block_line "$merge_job" '          push-to-registry: true' \
    'relay merged-manifest attestation must be pushed with the image'

  require_block_line "$gateway_build" "    if: github.repository == 'block/buzz'" \
    'push-gateway build must be restricted to the upstream repository'
  require_block_line "$gateway_merge" \
    "    if: github.repository == 'block/buzz' && github.event_name != 'pull_request'" \
    'push-gateway publication must be restricted to upstream non-PR events'

  require_line "$dockerfile" \
    '      org.opencontainers.image.source="https://github.com/zachlendon/buzz" \' \
    'relay OCI source label must identify the fork'
  require_line "$dockerfile" \
    '      org.opencontainers.image.url="https://github.com/zachlendon/buzz" \' \
    'relay OCI URL label must identify the fork'
  require_line "$dockerfile" \
    '      org.opencontainers.image.documentation="https://github.com/zachlendon/buzz#readme" \' \
    'relay OCI documentation label must identify the fork'

  require_line "$root/deploy/charts/buzz/values.yaml" \
    '  repository: ghcr.io/zachlendon/buzz' 'Helm relay image default is wrong'
  require_line "$root/deploy/compose/compose.yml" \
    '    image: ${BUZZ_IMAGE:-ghcr.io/zachlendon/buzz:main}' 'Compose relay image default is wrong'
  require_line "$root/deploy/compose/.env.example" \
    'BUZZ_IMAGE=ghcr.io/zachlendon/buzz:main' 'Compose environment relay image default is wrong'
  require_line "$root/benchmarks/harbor-buzz-orchestra/scripts/benchmark.py" \
    '        "BUZZ_IMAGE": os.environ.get("BUZZ_IMAGE", "ghcr.io/zachlendon/buzz:main"),' \
    'Harbor benchmark relay image default is wrong'

  require_line "$relay_manifest" \
    '# (ghcr.io/zachlendon/buzz), released on its own cadence via `just release-relay`.' \
    'relay package metadata must identify the fork-owned image'
  require_line "$root/RELEASING.md" \
    '| Relay | `just release-relay` | `ghcr.io/zachlendon/buzz` container image |' \
    'relay release table must identify the fork-owned image'
  require_line "$root/RELEASING.md" \
    '   image and publishes `ghcr.io/zachlendon/buzz:<version>` (plus `:<major>.<minor>`,' \
    'relay release procedure must identify the fork-owned image'
  require_line "$root/deploy/compose/README.md" \
    '- Default `BUZZ_IMAGE` tracks `ghcr.io/zachlendon/buzz:main` for early testing. Pin it to `ghcr.io/zachlendon/buzz:sha-<7>` or a semver release tag for production once available.' \
    'Compose guidance must identify the fork-owned image'
  require_line "$root/Justfile" \
    '# Open or update the relay release PR (ghcr.io/zachlendon/buzz image)' \
    'relay release recipe must identify the fork-owned image'
  require_line "$root/.github/workflows/helm-chart.yml" \
    '    # Full end-to-end install requires the public ghcr.io/zachlendon/buzz image to' \
    'Helm relay test guidance must identify the fork-owned image'
  require_line "$ci" '        run: scripts/test-relay-image-publication-contract.sh' \
    'CI must execute the relay publication contract'

  # A slash denotes the separately published upstream chart namespace and a
  # hyphen denotes the separately scoped push-gateway package. All other
  # upstream relay-image references must be historical changelog entries or
  # this fail-closed checker naming the forbidden value.
  local upstream_matches
  upstream_matches=$(grep -REn \
    --exclude-dir=.git \
    --exclude='*CHANGELOG.md' \
    --exclude='test-relay-image-publication-contract.sh' \
    'ghcr\.io/block/buzz([:@[:space:]]|$)' "$root" || true)
  [[ -z "$upstream_matches" ]] || {
    printf '%s\n' "$upstream_matches" >&2
    fail 'an active relay surface still references the upstream image'
  }

  [[ "$contract_failed" == 0 ]]
}

replace_once() {
  local file=$1
  local old=$2
  local new=$3
  OLD=$old NEW=$new perl -0pi -e '
    BEGIN { $old = $ENV{OLD}; $new = $ENV{NEW}; $count = 0 }
    $count += s/\Q$old\E/$new/;
    END { die "mutation target count was $count, expected 1\n" unless $count == 1 }
  ' "$file"
}

run_negative_probe() {
  local fixture_base=$1
  local name=$2
  local relative_file=$3
  local old=$4
  local new=$5
  local fixture="$fixture_base/cases/$name"
  mkdir -p "$fixture"
  cp -R "$fixture_base/source/." "$fixture/"
  replace_once "$fixture/$relative_file" "$old" "$new"
  if check_contract "$fixture" >/dev/null 2>&1; then
    echo "relay image publication contract: negative probe '$name' was not rejected" >&2
    return 1
  fi
  echo "negative probe rejected: $name"
}

check_contract "$repo_root"

fixture_base=$(mktemp -d)
trap 'rm -rf "$fixture_base"' EXIT
fixture_source="$fixture_base/source"
fixture_files=(
  .github/workflows/ci.yml
  .github/workflows/docker.yml
  .github/workflows/helm-chart.yml
  Dockerfile
  Justfile
  RELEASING.md
  benchmarks/harbor-buzz-orchestra/scripts/benchmark.py
  crates/buzz-relay/Cargo.toml
  deploy/charts/buzz/values.yaml
  deploy/compose/.env.example
  deploy/compose/README.md
  deploy/compose/compose.yml
)
for relative_file in "${fixture_files[@]}"; do
  mkdir -p "$fixture_source/$(dirname "$relative_file")"
  cp "$repo_root/$relative_file" "$fixture_source/$relative_file"
done

run_negative_probe "$fixture_base" main-trigger .github/workflows/docker.yml \
  '    branches: [main]' '    branches: [main, release]'
run_negative_probe "$fixture_base" relay-job-upstream-guard .github/workflows/docker.yml \
  $'  build:\n    name: Build (${{ matrix.platform }})' \
  $'  build:\n    name: Build (${{ matrix.platform }})\n    if: github.repository == '\''block/buzz'\'''
run_negative_probe "$fixture_base" build-package-write .github/workflows/docker.yml \
  '      packages: write    # push to GHCR' '      packages: read     # mutated'
run_negative_probe "$fixture_base" merge-package-write .github/workflows/docker.yml \
  '      packages: write    # push the merged manifest' '      packages: read     # mutated'
run_negative_probe "$fixture_base" repository-owner-login .github/workflows/docker.yml \
  '          username: ${{ github.repository_owner }}' '          username: block'
run_negative_probe "$fixture_base" merge-repository-owner-login .github/workflows/docker.yml \
  $'      - name: Log in to GHCR\n        uses: docker/login-action@650006c6eb7dba73a995cc03b0b2d7f5ca915bee  # v4.2.0\n        with:\n          registry: ghcr.io\n          username: ${{ github.repository_owner }}' \
  $'      - name: Log in to GHCR\n        uses: docker/login-action@650006c6eb7dba73a995cc03b0b2d7f5ca915bee  # v4.2.0\n        with:\n          registry: ghcr.io\n          username: block'
run_negative_probe "$fixture_base" build-digest-source .github/workflows/docker.yml \
  '          DIGEST: ${{ steps.build.outputs.digest }}' '          DIGEST: ${{ steps.meta.outputs.version }}'
run_negative_probe "$fixture_base" manifest-digest-source .github/workflows/docker.yml \
  '            digests+=("${IMAGE_NAME}@sha256:${digest}")' '            digests+=("${IMAGE_NAME}:main")'
run_negative_probe "$fixture_base" attestation-subject .github/workflows/docker.yml \
  '          subject-name: ${{ env.IMAGE_NAME }}' '          subject-name: ghcr.io/block/buzz'
run_negative_probe "$fixture_base" attestation-digest .github/workflows/docker.yml \
  '          subject-digest: ${{ steps.manifest.outputs.digest }}' '          subject-digest: ${{ steps.build.outputs.digest }}'
run_negative_probe "$fixture_base" helm-pull-default deploy/charts/buzz/values.yaml \
  '  repository: ghcr.io/zachlendon/buzz' '  repository: ghcr.io/block/buzz'
run_negative_probe "$fixture_base" compose-pull-default deploy/compose/compose.yml \
  '    image: ${BUZZ_IMAGE:-ghcr.io/zachlendon/buzz:main}' '    image: ${BUZZ_IMAGE:-ghcr.io/block/buzz:main}'
run_negative_probe "$fixture_base" compose-env-pull-default deploy/compose/.env.example \
  'BUZZ_IMAGE=ghcr.io/zachlendon/buzz:main' 'BUZZ_IMAGE=ghcr.io/block/buzz:main'
run_negative_probe "$fixture_base" benchmark-pull-default benchmarks/harbor-buzz-orchestra/scripts/benchmark.py \
  '        "BUZZ_IMAGE": os.environ.get("BUZZ_IMAGE", "ghcr.io/zachlendon/buzz:main"),' \
  '        "BUZZ_IMAGE": os.environ.get("BUZZ_IMAGE", "ghcr.io/block/buzz:main"),'
run_negative_probe "$fixture_base" push-gateway-build-guard .github/workflows/docker.yml \
  "    if: github.repository == 'block/buzz'" "    if: github.repository == 'zachlendon/buzz'"
run_negative_probe "$fixture_base" push-gateway-merge-guard .github/workflows/docker.yml \
  "    if: github.repository == 'block/buzz' && github.event_name != 'pull_request'" \
  "    if: github.event_name != 'pull_request'"

echo "relay image publication contract passed"
