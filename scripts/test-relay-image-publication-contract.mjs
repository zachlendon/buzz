#!/usr/bin/env node

/* biome-ignore-all lint/suspicious/noTemplateCurlyInString: fixtures intentionally contain shell and GitHub expressions */

import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { verifyRepository } from "./verify-relay-image-publication-contract.mjs";

const root = path.resolve(import.meta.dirname, "..");

function replaceFirst(source, oldValue, newValue) {
  assert.ok(
    source.includes(oldValue),
    `mutation target must exist: ${oldValue}`,
  );
  return source.replace(oldValue, newValue);
}

function mutation(name, relativePath, oldValue, newValue) {
  return {
    name,
    overrides: new Map([
      [
        relativePath,
        replaceFirst(
          fs.readFileSync(path.join(root, relativePath), "utf8"),
          oldValue,
          newValue,
        ),
      ],
    ]),
  };
}

function transformMutation(name, relativePath, transform) {
  const source = fs.readFileSync(path.join(root, relativePath), "utf8");
  const transformed = transform(source);
  assert.notEqual(transformed, source, `mutation must change ${relativePath}`);
  return { name, overrides: new Map([[relativePath, transformed]]) };
}

const workflow = ".github/workflows/docker.yml";
const canonicalWorkflow = fs.readFileSync(path.join(root, workflow), "utf8");
const workflowEnvStart = canonicalWorkflow.indexOf("\nenv:\n");
const workflowEnvEnd = canonicalWorkflow.indexOf("\njobs:\n", workflowEnvStart);
assert.ok(workflowEnvStart >= 0 && workflowEnvEnd > workflowEnvStart);
const flowCanonicalWorkflow =
  canonicalWorkflow.slice(0, workflowEnvStart) +
  "\nenv: { \"IMAGE_NAME\": \"${{ vars.GHCR_IMAGE != '' && vars.GHCR_IMAGE || 'ghcr.io/zachlendon/buzz' }}\" }\n" +
  canonicalWorkflow.slice(workflowEnvEnd);
const positiveCases = [
  {
    name: "quoted-canonical-image-key",
    overrides: new Map([
      [
        workflow,
        replaceFirst(canonicalWorkflow, "  IMAGE_NAME:", '  "IMAGE_NAME":'),
      ],
    ]),
  },
  {
    name: "flow-canonical-image-env",
    overrides: new Map([[workflow, flowCanonicalWorkflow]]),
  },
];
const cases = [
  mutation(
    "pull-request-target-event",
    workflow,
    "  pull_request:\n",
    "  pull_request_target: {}\n  pull_request:\n",
  ),
  mutation(
    "broadened-tag-trigger",
    workflow,
    '    tags: ["relay-v[0-9]*"]',
    '    tags: ["*"]',
  ),
  mutation(
    "additional-tag-trigger",
    workflow,
    '    tags: ["relay-v[0-9]*"]',
    '    tags: ["relay-v[0-9]*", "v*"]',
  ),
  mutation(
    "fifth-write-all-job",
    workflow,
    "  push-gateway-build:\n",
    "  publish-extra:\n    runs-on: ubuntu-latest\n    permissions: write-all\n    steps: []\n\n  push-gateway-build:\n",
  ),
  mutation(
    "main-trigger",
    workflow,
    "branches: [main]",
    "branches: [main, release]",
  ),
  mutation(
    "push-paths-restriction",
    workflow,
    "    branches: [main]\n    tags:",
    "    branches: [main]\n    paths: [Dockerfile]\n    tags:",
  ),
  mutation(
    "push-paths-ignore-restriction",
    workflow,
    "    branches: [main]\n    tags:",
    "    branches: [main]\n    paths-ignore: [docs/**]\n    tags:",
  ),
  mutation(
    "quoted-image-name-shadow",
    workflow,
    "  build:\n    name:",
    '  build:\n    env:\n      "IMAGE_NAME": ghcr.io/another-owner/buzz\n    name:',
  ),
  mutation(
    "flow-image-name-shadow",
    workflow,
    "  merge:\n    name:",
    '  merge:\n    env: { "IMAGE_NAME": ghcr.io/zachlendon/buzz }\n    name:',
  ),
  mutation(
    "relay-job-block-only-if",
    workflow,
    "  build:\n    name:",
    "  build:\n    if: github.repository == 'block/buzz'\n    name:",
  ),
  mutation(
    "workflow-permission-widening",
    workflow,
    "permissions: {}",
    "permissions:\n  actions: write",
  ),
  mutation(
    "build-contents-write",
    workflow,
    "      contents: read\n      packages: write    # push to GHCR",
    "      contents: write\n      packages: write    # push to GHCR",
  ),
  mutation(
    "merge-attestations-read",
    workflow,
    "      packages: write    # push the merged manifest\n      id-token: write    # OIDC for provenance attestation on the manifest\n      attestations: write",
    "      packages: write    # push the merged manifest\n      id-token: write    # OIDC for provenance attestation on the manifest\n      attestations: read",
  ),
  mutation(
    "extra-administration-permission",
    workflow,
    "      packages: write    # push to GHCR",
    "      packages: write    # push to GHCR\n      administration: write",
  ),
  mutation(
    "extra-security-events-permission",
    workflow,
    "      packages: write    # push to GHCR",
    "      packages: write    # push to GHCR\n      security-events: write",
  ),
  mutation(
    "extra-actions-permission",
    workflow,
    "      packages: write    # push to GHCR",
    "      packages: write    # push to GHCR\n      actions: write",
  ),
  mutation(
    "extra-workflows-permission",
    workflow,
    "      packages: write    # push to GHCR",
    "      packages: write    # push to GHCR\n      workflows: write",
  ),
  mutation(
    "remove-build-id-token",
    workflow,
    "      id-token: write    # OIDC for build provenance attestation\n",
    "",
  ),
  mutation(
    "downgrade-merge-id-token",
    workflow,
    "      id-token: write    # OIDC for provenance attestation on the manifest",
    "      id-token: read     # mutated",
  ),
  mutation(
    "remove-build-contents",
    workflow,
    "      contents: read\n      packages: write    # push to GHCR",
    "      packages: write    # push to GHCR",
  ),
  mutation(
    "remove-merge-packages",
    workflow,
    "      packages: write    # push the merged manifest\n",
    "",
  ),
  mutation(
    "build-package-write",
    workflow,
    "      packages: write    # push to GHCR",
    "      packages: read     # mutated",
  ),
  mutation(
    "merge-package-write",
    workflow,
    "      packages: write    # push the merged manifest",
    "      packages: read     # mutated",
  ),
  mutation(
    "build-repository-owner-login",
    workflow,
    "          username: ${{ github.repository_owner }}",
    "          username: block",
  ),
  mutation(
    "merge-repository-owner-login",
    workflow,
    "      - name: Log in to GHCR\n        uses: docker/login-action@650006c6eb7dba73a995cc03b0b2d7f5ca915bee  # v4.2.0\n        with:\n          registry: ghcr.io\n          username: ${{ github.repository_owner }}",
    "      - name: Log in to GHCR\n        uses: docker/login-action@650006c6eb7dba73a995cc03b0b2d7f5ca915bee  # v4.2.0\n        with:\n          registry: ghcr.io\n          username: block",
  ),
  mutation(
    "build-digest-source",
    workflow,
    "          DIGEST: ${{ steps.build.outputs.digest }}",
    "          DIGEST: ${{ steps.meta.outputs.version }}",
  ),
  mutation(
    "digest-reassignment",
    workflow,
    '          touch "/tmp/digests/${DIGEST#sha256:}"',
    '          touch "/tmp/digests/${DIGEST#sha256:}"\n          DIGEST=sha256:mutated',
  ),
  mutation(
    "manifest-input-source",
    workflow,
    '            digests+=("${IMAGE_NAME}@sha256:${digest}")',
    '            digests+=("${IMAGE_NAME}:main")',
  ),
  mutation(
    "merged-digest-reassignment",
    workflow,
    '          echo "digest=${merged_digest}" >> "$GITHUB_OUTPUT"',
    '          merged_digest=sha256:mutated\n          echo "digest=${merged_digest}" >> "$GITHUB_OUTPUT"',
  ),
  mutation(
    "attestation-subject",
    workflow,
    "          subject-name: ${{ env.IMAGE_NAME }}",
    "          subject-name: ghcr.io/block/buzz",
  ),
  mutation(
    "attestation-digest",
    workflow,
    "          subject-digest: ${{ steps.manifest.outputs.digest }}",
    "          subject-digest: ${{ steps.build.outputs.digest }}",
  ),
  mutation(
    "attestation-impossible-if",
    workflow,
    "      - name: Attest provenance for the merged image\n        # Sigstore",
    "      - name: Attest provenance for the merged image\n        if: ${{ false }}\n        # Sigstore",
  ),
  mutation(
    "attestation-continue-on-error",
    workflow,
    "      - name: Attest provenance for the merged image\n        # Sigstore",
    "      - name: Attest provenance for the merged image\n        continue-on-error: true\n        # Sigstore",
  ),
  mutation(
    "secondary-metadata-image",
    workflow,
    "          images: ${{ env.IMAGE_NAME }}",
    "          images: |\n            ${{ env.IMAGE_NAME }}\n            ${{ vars.EXTRA_IMAGE }}",
  ),
  mutation(
    "extra-metadata-annotation",
    workflow,
    "          images: ${{ env.IMAGE_NAME }}\n          # Tag matrix",
    "          images: ${{ env.IMAGE_NAME }}\n          annotations: org.opencontainers.image.ref.name=${{ vars.EXTRA_IMAGE }}\n          # Tag matrix",
  ),
  mutation(
    "extra-metadata-label",
    workflow,
    "            org.opencontainers.image.licenses=Apache-2.0",
    "            org.opencontainers.image.licenses=Apache-2.0\n            org.opencontainers.image.ref.name=${{ vars.EXTRA_IMAGE }}",
  ),
  mutation(
    "secondary-build-tags-input",
    workflow,
    "          platforms: ${{ matrix.platform }}\n          labels: ${{ steps.meta.outputs.labels }}",
    "          platforms: ${{ matrix.platform }}\n          tags: ${{ vars.EXTRA_IMAGE }}\n          labels: ${{ steps.meta.outputs.labels }}",
  ),
  mutation(
    "secondary-build-output",
    workflow,
    "          outputs: type=image,name=${{ env.IMAGE_NAME }},push-by-digest=true,name-canonical=true,push=${{ github.event_name != 'pull_request' }}",
    "          outputs: |\n            type=image,name=${{ env.IMAGE_NAME }},push-by-digest=true,name-canonical=true,push=${{ github.event_name != 'pull_request' }}\n            type=image,name=${{ vars.EXTRA_IMAGE }},push=true",
  ),
  mutation(
    "secondary-build-annotation",
    workflow,
    "          labels: ${{ steps.meta.outputs.labels }}\n          # Push by digest",
    "          labels: ${{ steps.meta.outputs.labels }}\n          annotations: org.opencontainers.image.ref.name=${{ vars.EXTRA_IMAGE }}\n          # Push by digest",
  ),
  mutation(
    "literal-upstream-cache-from",
    workflow,
    "            type=registry,ref=${{ env.IMAGE_NAME }}-buildcache:${{ matrix.arch }}",
    "            type=registry,ref=ghcr.io/block/buzz-buildcache:${{ matrix.arch }}",
  ),
  mutation(
    "literal-upstream-cache-to",
    workflow,
    "            ${{ (github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository) && format('type=registry,ref={0}-buildcache:{1},mode=max,compression=zstd', env.IMAGE_NAME, matrix.arch) || '' }}",
    "            type=registry,ref=ghcr.io/block/buzz-buildcache:${{ matrix.arch }},mode=max,compression=zstd",
  ),
  mutation(
    "format-built-upstream-cache-to",
    workflow,
    "            ${{ (github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository) && format('type=registry,ref={0}-buildcache:{1},mode=max,compression=zstd', env.IMAGE_NAME, matrix.arch) || '' }}",
    "            ${{ format('type=registry,ref=ghcr.io/{0}/{1}-buildcache:{2},mode=max,compression=zstd', 'block', 'buzz', matrix.arch) }}",
  ),
  mutation(
    "split-built-upstream-cache-to",
    workflow,
    "            ${{ (github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository) && format('type=registry,ref={0}-buildcache:{1},mode=max,compression=zstd', env.IMAGE_NAME, matrix.arch) || '' }}",
    '            \'"type=registry,ref=ghcr.io/" + "block/" + "buzz-buildcache:" + matrix.arch\'',
  ),
  mutation(
    "format-built-upstream",
    workflow,
    "          images: ${{ env.IMAGE_NAME }}",
    "          images: ${{ format('ghcr.io/{0}/{1}', 'block', 'buzz') }}",
  ),
  mutation(
    "split-quoted-upstream",
    workflow,
    "          images: ${{ env.IMAGE_NAME }}",
    '          images: \'"ghcr.io/" + "block/" + "buzz"\'',
  ),
  mutation(
    "helm-pull-default",
    "deploy/charts/buzz/values.yaml",
    "repository: ghcr.io/zachlendon/buzz",
    "repository: ghcr.io/block/buzz",
  ),
  mutation(
    "compose-pull-default",
    "deploy/compose/compose.yml",
    "${BUZZ_IMAGE:-ghcr.io/zachlendon/buzz:main}",
    "${BUZZ_IMAGE:-ghcr.io/block/buzz:main}",
  ),
  mutation(
    "compose-env-pull-default",
    "deploy/compose/.env.example",
    "BUZZ_IMAGE=ghcr.io/zachlendon/buzz:main",
    "BUZZ_IMAGE=ghcr.io/block/buzz:main",
  ),
  mutation(
    "benchmark-pull-default",
    "benchmarks/harbor-buzz-orchestra/scripts/benchmark.py",
    '"BUZZ_IMAGE": os.environ.get("BUZZ_IMAGE", "ghcr.io/zachlendon/buzz:main")',
    '"BUZZ_IMAGE": os.environ.get("BUZZ_IMAGE", "ghcr.io/block/buzz:main")',
  ),
  mutation(
    "push-gateway-build-guard",
    workflow,
    "    if: github.repository == 'block/buzz'",
    "    if: github.repository == 'zachlendon/buzz'",
  ),
  mutation(
    "push-gateway-merge-guard",
    workflow,
    "    if: github.repository == 'block/buzz' && github.event_name != 'pull_request'",
    "    if: github.event_name != 'pull_request'",
  ),
  mutation(
    "quoted-upstream-reference",
    ".github/CODEOWNERS",
    "* @block/buzz-oss-team",
    '"ghcr.io/block/buzz"',
  ),
  mutation(
    "parenthesized-upstream-reference",
    ".github/CODEOWNERS",
    "* @block/buzz-oss-team",
    "(ghcr.io/block/buzz)",
  ),
  mutation(
    "comma-upstream-reference",
    ".github/CODEOWNERS",
    "* @block/buzz-oss-team",
    "ghcr.io/block/buzz,",
  ),
  mutation(
    "period-upstream-reference",
    ".github/CODEOWNERS",
    "* @block/buzz-oss-team",
    "ghcr.io/block/buzz.",
  ),
  mutation(
    "unpinned-docker-action",
    workflow,
    "uses: actions/checkout@df4cb1c069e1874edd31b4311f1884172cec0e10",
    "uses: actions/checkout@main",
  ),
  mutation(
    "unpinned-ci-activate-hermit",
    ".github/workflows/ci.yml",
    "uses: cashapp/activate-hermit@cea9af7913204a965fd488637a8d1811bba2e616",
    "uses: cashapp/activate-hermit@v1",
  ),
  mutation(
    "unpinned-ci-paths-filter",
    ".github/workflows/ci.yml",
    "uses: dorny/paths-filter@7b450fff21473bca461d4b92ce414b9d0420d706",
    "uses: dorny/paths-filter@v4",
  ),
  transformMutation(
    "ci-contract-before-install",
    ".github/workflows/ci.yml",
    (source) => {
      const contract =
        "      - name: Relay image publication contract\n        run: pnpm test:relay-image-contract\n";
      const activation =
        "      - uses: cashapp/activate-hermit@cea9af7913204a965fd488637a8d1811bba2e616 # v1\n";
      return replaceFirst(
        replaceFirst(source, contract, ""),
        activation,
        `${activation}${contract}`,
      );
    },
  ),
  mutation(
    "extra-ci-contract-before-install",
    ".github/workflows/ci.yml",
    "      - name: Install relay contract dependencies\n",
    "      - run: pnpm test:relay-image-contract\n      - name: Install relay contract dependencies\n",
  ),
];

verifyRepository(root);
for (const testCase of positiveCases) {
  verifyRepository(root, testCase.overrides);
  console.log(`positive semantic fixture accepted: ${testCase.name}`);
}
for (const testCase of cases) {
  assert.throws(
    () => verifyRepository(root, testCase.overrides),
    undefined,
    `negative mutation was accepted: ${testCase.name}`,
  );
  console.log(`negative semantic fixture rejected: ${testCase.name}`);
}
console.log(
  `relay image publication semantic contract passed (${cases.length} negatives)`,
);
