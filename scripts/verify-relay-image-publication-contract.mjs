#!/usr/bin/env node

/* biome-ignore-all lint/suspicious/noTemplateCurlyInString: exact shell and GitHub expressions are contract values */

import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { isAlias, isMap, isScalar, isSeq, parseDocument } from "yaml";

const WORKFLOW = ".github/workflows/docker.yml";
const FORK_IMAGE = "ghcr.io/zachlendon/buzz";
const IMAGE_EXPRESSION =
  "${{ vars.GHCR_IMAGE != '' && vars.GHCR_IMAGE || 'ghcr.io/zachlendon/buzz' }}";
const REPOSITORY_OWNER = "${{ github.repository_owner }}";
const GITHUB_TOKEN = "${{ secrets.GITHUB_TOKEN }}";
const GATEWAY_JOBS = new Set(["push-gateway-build", "push-gateway-merge"]);
const PROVENANCE_PERMISSIONS = {
  contents: "read",
  packages: "write",
  "id-token": "write",
  attestations: "write",
};
const GATEWAY_BUILD_PERMISSIONS = {
  contents: "read",
  packages: "write",
};

const BUILD_STEP_NAMES = [
  "Checkout",
  "Verify tag-bound release source",
  "Set up Docker Buildx",
  "Log in to GHCR",
  "Extract metadata",
  "Build and push by digest",
  "Export digest",
  "Upload digest",
];
const MERGE_STEP_NAMES = [
  "Download all per-arch digests",
  "Set up Docker Buildx",
  "Log in to GHCR",
  "Extract metadata",
  "Create and push manifest list",
  "Attest provenance for the merged image",
  "Summary",
];

const EXPORT_DIGEST_RUN = `mkdir -p /tmp/digests
touch "/tmp/digests/\${DIGEST#sha256:}"
`;

const MANIFEST_RUN = `set -euo pipefail
# Build -t flags from the metadata-action output.
tags=()
while IFS= read -r tag; do
  [ -n "$tag" ] && tags+=("-t" "$tag")
done <<< "$META_TAGS"

# Build the digest refs from the per-arch artifacts.
digests=()
for digest in *; do
  digests+=("\${IMAGE_NAME}@sha256:\${digest}")
done

docker buildx imagetools create "\${tags[@]}" "\${digests[@]}"

# Capture the merged manifest digest for the attestation step.
first_tag=$(echo "$META_TAGS" | head -n1)
merged_digest=$(docker buildx imagetools inspect "$first_tag" \\
  --format '{{json .Manifest}}' | jq -r '.digest')
echo "digest=\${merged_digest}" >> "$GITHUB_OUTPUT"
`;

const SUMMARY_LINES = [
  "{",
  '  echo "### Published \\`${IMAGE_NAME}\\`"',
  "  echo",
  '  echo "**Digest:** \\`${MERGED_DIGEST}\\`"',
  "  echo",
  '  echo "**Tags:**"',
  "  echo '```'",
  '  echo "${META_TAGS}"',
  "  echo '```'",
  "  echo",
  '  echo "Verify provenance:"',
  "  echo '```'",
  '  echo "gh attestation verify oci://${IMAGE_NAME}@${MERGED_DIGEST} --owner ${GITHUB_REPOSITORY_OWNER}"',
  "  echo '```'",
  '} >> "$GITHUB_STEP_SUMMARY"',
];
const SUMMARY_RUN = `${SUMMARY_LINES.join("\n")}\n`;

function readFile(root, relativePath, overrides) {
  return (
    overrides.get(relativePath) ??
    fs.readFileSync(path.join(root, relativePath), "utf8")
  );
}

function scalarKey(node) {
  if (!isScalar(node)) {
    throw new Error("workflow mapping keys must be scalar values");
  }
  return String(node.value);
}

function walkYaml(node, currentPath, visitor) {
  if (isAlias(node)) {
    throw new Error(`YAML aliases are not allowed at ${currentPath.join(".")}`);
  }
  if (isMap(node)) {
    for (const pair of node.items) {
      const key = scalarKey(pair.key);
      const nextPath = [...currentPath, key];
      visitor({ key, node: pair.value, path: nextPath, pair });
      walkYaml(pair.value, nextPath, visitor);
    }
  } else if (isSeq(node)) {
    node.items.forEach((item, index) => {
      walkYaml(item, [...currentPath, index], visitor);
    });
  }
}

function parseYamlDocument(source, label) {
  const document = parseDocument(source, {
    prettyErrors: true,
    uniqueKeys: true,
  });
  if (document.errors.length > 0) {
    throw new Error(
      `${label} is invalid YAML:\n${document.errors.map(String).join("\n")}`,
    );
  }
  walkYaml(document.contents, [], () => {});
  return {
    ast: document.contents,
    value: document.toJS({ maxAliasCount: 0 }),
  };
}

function own(object, key) {
  return Object.hasOwn(object, key);
}

function exactKeys(object, expected, label) {
  assert.deepEqual(Object.keys(object).sort(), [...expected].sort(), label);
}

function findStep(job, name) {
  const matches = job.steps.filter((step) => step.name === name);
  assert.equal(matches.length, 1, `expected exactly one '${name}' step`);
  return matches[0];
}

function validateConditions(build, merge) {
  assert.equal(own(build, "if"), false, "relay build job must not have an if");
  assert.equal(
    merge.if,
    "github.event_name != 'pull_request'",
    "relay merge job condition changed",
  );

  const buildConditions = new Map([
    [
      "Verify tag-bound release source",
      "github.ref_type == 'tag' || github.event_name == 'workflow_dispatch'",
    ],
    [
      "Log in to GHCR",
      "github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository",
    ],
    ["Export digest", "github.event_name != 'pull_request'"],
    ["Upload digest", "github.event_name != 'pull_request'"],
  ]);

  for (const step of build.steps) {
    const expected = buildConditions.get(step.name);
    if (expected === undefined) {
      assert.equal(own(step, "if"), false, `${step.name} must not have an if`);
    } else {
      assert.equal(step.if, expected, `${step.name} condition changed`);
    }
  }
  for (const step of merge.steps) {
    assert.equal(own(step, "if"), false, `${step.name} must not have an if`);
  }
}

function validateLogin(job, label) {
  const login = findStep(job, "Log in to GHCR");
  assert.equal(
    login.uses,
    "docker/login-action@650006c6eb7dba73a995cc03b0b2d7f5ca915bee",
    `${label} login action changed`,
  );
  assert.deepEqual(
    login.with,
    {
      registry: "ghcr.io",
      username: REPOSITORY_OWNER,
      password: GITHUB_TOKEN,
    },
    `${label} login inputs changed`,
  );
}

function validateDigestDataflow(build, merge) {
  const buildPush = findStep(build, "Build and push by digest");
  assert.equal(buildPush.id, "build", "relay build step id changed");
  assert.equal(
    buildPush.uses,
    "docker/build-push-action@f9f3042f7e2789586610d6e8b85c8f03e5195baf",
  );
  assert.equal(
    buildPush.with.outputs,
    "type=image,name=${{ env.IMAGE_NAME }},push-by-digest=true,name-canonical=true,push=${{ github.event_name != 'pull_request' }}",
    "relay build output must bind IMAGE_NAME and publish by digest",
  );
  assert.equal(
    buildPush.with["cache-from"],
    "type=registry,ref=${{ env.IMAGE_NAME }}-buildcache:${{ matrix.arch }}\n",
    "relay build cache source must derive from IMAGE_NAME",
  );

  const exportDigest = findStep(build, "Export digest");
  assert.deepEqual(
    exportDigest.env,
    { DIGEST: "${{ steps.build.outputs.digest }}" },
    "DIGEST must be assigned exactly from the build output",
  );
  assert.equal(
    exportDigest.run,
    EXPORT_DIGEST_RUN,
    "digest export script changed",
  );

  const upload = findStep(build, "Upload digest");
  assert.equal(
    upload.uses,
    "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
  );
  assert.deepEqual(
    upload.with,
    {
      name: "digests-${{ matrix.arch }}",
      path: "/tmp/digests/*",
      "if-no-files-found": "error",
      "retention-days": 1,
    },
    "digest artifact upload changed",
  );

  const download = findStep(merge, "Download all per-arch digests");
  assert.equal(
    download.uses,
    "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c",
  );
  assert.deepEqual(
    download.with,
    {
      path: "/tmp/digests",
      pattern: "digests-*",
      "merge-multiple": true,
    },
    "manifest digest download changed",
  );

  const manifest = findStep(merge, "Create and push manifest list");
  assert.equal(manifest.id, "manifest", "manifest step id changed");
  assert.equal(manifest["working-directory"], "/tmp/digests");
  assert.deepEqual(
    manifest.env,
    { META_TAGS: "${{ steps.meta.outputs.tags }}" },
    "manifest environment changed",
  );
  assert.equal(
    manifest.run,
    MANIFEST_RUN,
    "manifest inputs or digest export changed",
  );

  const scripts = [...build.steps, ...merge.steps]
    .filter((step) => typeof step.run === "string")
    .map((step) => ({ name: step.name, run: step.run }));
  const digestAssignments = [];
  const digestExports = [];
  for (const script of scripts) {
    for (const match of script.run.matchAll(
      /(?:^|\n)\s*(?:export\s+)?(DIGEST|merged_digest)\s*(?:\+?=)/g,
    )) {
      digestAssignments.push({ name: script.name, variable: match[1] });
    }
    for (const match of script.run.matchAll(/digest=\$\{merged_digest\}/g)) {
      digestExports.push({ name: script.name, value: match[0] });
    }
  }
  assert.deepEqual(
    digestAssignments,
    [{ name: "Create and push manifest list", variable: "merged_digest" }],
    "DIGEST or merged_digest was reassigned",
  );
  assert.deepEqual(
    digestExports,
    [
      {
        name: "Create and push manifest list",
        value: "digest=${merged_digest}",
      },
    ],
    "merged digest must be exported exactly once",
  );

  const summary = findStep(merge, "Summary");
  assert.deepEqual(summary.env, {
    MERGED_DIGEST: "${{ steps.manifest.outputs.digest }}",
    META_TAGS: "${{ steps.meta.outputs.tags }}",
  });
  assert.equal(
    summary.run,
    SUMMARY_RUN,
    "publication summary dataflow changed",
  );
}

function validateAttestation(merge) {
  const step = findStep(merge, "Attest provenance for the merged image");
  exactKeys(step, ["name", "uses", "with"], "attestation step shape changed");
  assert.equal(
    step.uses,
    "actions/attest-build-provenance@0f67c3f4856b2e3261c31976d6725780e5e4c373",
  );
  assert.deepEqual(step.with, {
    "subject-name": "${{ env.IMAGE_NAME }}",
    "subject-digest": "${{ steps.manifest.outputs.digest }}",
    "push-to-registry": true,
  });
}

function suspiciousUpstreamTarget(value) {
  const lower = value.toLowerCase();
  return (
    lower.includes("ghcr.io") &&
    /(?:^|[^a-z0-9])block(?:[^a-z0-9]|$)/.test(lower) &&
    /(?:^|[^a-z0-9])buzz(?:[^a-z0-9]|$)/.test(lower)
  );
}

function removeAllowedTargets(value, family) {
  if (family === "gateway") {
    return value.replaceAll(
      /ghcr\.io\/block\/buzz-push-gateway(?:-buildcache)?/gi,
      "",
    );
  }
  if (family === "charts") {
    return value.replaceAll(/ghcr\.io\/block\/buzz\/charts/gi, "");
  }
  return value;
}

function validateWorkflowUpstreamTargets(ast, label, locationFamily) {
  walkYaml(ast, [], ({ node, path: yamlPath }) => {
    if (!isScalar(node) || typeof node.value !== "string") return;
    const value = node.value;
    if (!suspiciousUpstreamTarget(value)) return;
    const family = locationFamily(yamlPath);
    const remainder = removeAllowedTargets(value, family);
    assert.equal(
      suspiciousUpstreamTarget(remainder),
      false,
      `${label}:${yamlPath.join(".")} computes or targets the upstream relay image`,
    );
    assert.notEqual(
      remainder,
      value,
      `${label}:${yamlPath.join(".")} uses upstream fragments outside an allowlisted package family`,
    );
  });
}

function validateDockerWorkflow(source) {
  const { ast, value: workflow } = parseYamlDocument(source, WORKFLOW);
  assert.deepEqual(
    workflow.permissions,
    {},
    "workflow permissions must default to none",
  );
  assert.deepEqual(
    workflow.on.push.branches,
    ["main"],
    "Docker push branches must be exactly main",
  );
  exactKeys(
    workflow.on.push,
    ["branches", "tags"],
    "Docker push trigger must not define paths, paths-ignore, or extra filters",
  );

  const imageDefinitions = [];
  walkYaml(ast, [], ({ key, node, path: yamlPath }) => {
    if (key === "IMAGE_NAME") {
      imageDefinitions.push({
        path: yamlPath,
        value: isScalar(node) ? node.value : node,
      });
    }
  });
  assert.deepEqual(
    imageDefinitions,
    [{ path: ["env", "IMAGE_NAME"], value: IMAGE_EXPRESSION }],
    "IMAGE_NAME must be defined exactly once at workflow env",
  );
  assert.deepEqual(workflow.env, { IMAGE_NAME: IMAGE_EXPRESSION });

  const build = workflow.jobs.build;
  const merge = workflow.jobs.merge;
  assert.ok(build && merge, "relay build and merge jobs are required");
  assert.deepEqual(
    build.steps.map((step) => step.name),
    BUILD_STEP_NAMES,
  );
  assert.deepEqual(
    merge.steps.map((step) => step.name),
    MERGE_STEP_NAMES,
  );
  assert.equal(merge.needs, "build", "manifest job must depend on relay build");
  assert.deepEqual(
    build.permissions,
    PROVENANCE_PERMISSIONS,
    "relay build permissions changed",
  );
  assert.deepEqual(
    merge.permissions,
    PROVENANCE_PERMISSIONS,
    "relay merge permissions changed",
  );
  assert.deepEqual(
    workflow.jobs["push-gateway-build"].permissions,
    GATEWAY_BUILD_PERMISSIONS,
    "push-gateway build permissions changed",
  );
  assert.deepEqual(
    workflow.jobs["push-gateway-merge"].permissions,
    PROVENANCE_PERMISSIONS,
    "push-gateway merge permissions changed",
  );
  validateConditions(build, merge);
  validateLogin(build, "build");
  validateLogin(merge, "merge");

  for (const job of [build, merge]) {
    const metadata = findStep(job, "Extract metadata");
    assert.equal(metadata.id, "meta", "relay metadata step id changed");
    assert.equal(metadata.with.images, "${{ env.IMAGE_NAME }}");
  }

  validateDigestDataflow(build, merge);
  validateAttestation(merge);

  assert.equal(
    workflow.jobs["push-gateway-build"].if,
    "github.repository == 'block/buzz'",
  );
  assert.equal(
    workflow.jobs["push-gateway-merge"].if,
    "github.repository == 'block/buzz' && github.event_name != 'pull_request'",
  );

  validateWorkflowUpstreamTargets(ast, WORKFLOW, (yamlPath) => {
    if (yamlPath[0] === "jobs" && GATEWAY_JOBS.has(yamlPath[1])) {
      return "gateway";
    }
    return null;
  });
}

function validateChartWorkflow(source, relativePath) {
  const { ast } = parseYamlDocument(source, relativePath);
  validateWorkflowUpstreamTargets(ast, relativePath, () => "charts");
}

function validateDirectUpstreamReferences(root, overrides) {
  const excludedDirectories = new Set([".git", "node_modules", "target"]);
  const excludedNames = new Set([
    "test-relay-image-publication-contract.mjs",
    "verify-relay-image-publication-contract.mjs",
  ]);
  const extensions = new Set([
    ".env",
    ".example",
    ".js",
    ".json",
    ".md",
    ".mjs",
    ".py",
    ".sh",
    ".toml",
    ".ts",
    ".yaml",
    ".yml",
  ]);
  const specialNames = new Set(["CODEOWNERS", "Dockerfile", "Justfile"]);
  const allowedLocations = new Map([
    [WORKFLOW, new Set(["gateway"])],
    [".github/workflows/helm-chart.yml", new Set(["charts"])],
    [".github/workflows/push-gateway-helm-chart.yml", new Set(["charts"])],
    ["deploy/charts/buzz/README.md", new Set(["charts"])],
    ["deploy/charts/buzz/examples/argocd-app.yaml", new Set(["charts"])],
    ["deploy/charts/buzz/examples/flux-helmrelease.yaml", new Set(["charts"])],
    ["deploy/charts/buzz-push-gateway/Chart.yaml", new Set(["charts"])],
    ["deploy/charts/buzz-push-gateway/values.yaml", new Set(["gateway"])],
    ["docs/push-gateway-deployment.md", new Set(["charts", "gateway"])],
  ]);
  const violations = [];

  function inspect(relativePath, source) {
    if (relativePath.endsWith("CHANGELOG.md")) return;
    let offset = 0;
    const needle = "ghcr.io/block/buzz";
    let matchIndex = source.indexOf(needle, offset);
    while (matchIndex >= 0) {
      const tail = source.slice(matchIndex + needle.length);
      const families = allowedLocations.get(relativePath) ?? new Set();
      const allowedChart =
        families.has("charts") && /^\/charts(?:[^\w-]|$)/.test(tail);
      const allowedGateway =
        families.has("gateway") &&
        /^-push-gateway(?:-buildcache)?(?:[^\w-]|$)/.test(tail);
      if (!allowedChart && !allowedGateway) {
        violations.push(relativePath);
        break;
      }
      offset = matchIndex + needle.length;
      matchIndex = source.indexOf(needle, offset);
    }
  }

  function visitDirectory(relativeDirectory) {
    const absoluteDirectory = path.join(root, relativeDirectory);
    for (const entry of fs.readdirSync(absoluteDirectory, {
      withFileTypes: true,
    })) {
      if (entry.isDirectory() && excludedDirectories.has(entry.name)) continue;
      const relativePath = path.join(relativeDirectory, entry.name);
      if (entry.isDirectory()) {
        visitDirectory(relativePath);
        continue;
      }
      if (excludedNames.has(entry.name)) continue;
      if (
        !extensions.has(path.extname(entry.name)) &&
        !specialNames.has(entry.name)
      ) {
        continue;
      }
      inspect(relativePath, readFile(root, relativePath, overrides));
    }
  }

  visitDirectory("");
  assert.deepEqual(
    violations,
    [],
    `active files target the upstream relay image: ${violations.join(", ")}`,
  );
}

function parseEnv(source) {
  const result = {};
  for (const line of source.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (trimmed === "" || trimmed.startsWith("#")) continue;
    const separator = line.indexOf("=");
    if (separator < 1) continue;
    const key = line.slice(0, separator).trim();
    assert.equal(own(result, key), false, `duplicate environment key ${key}`);
    result[key] = line.slice(separator + 1).trim();
  }
  return result;
}

function requireCount(source, value, count, label) {
  assert.equal(source.split(value).length - 1, count, label);
}

export function verifyRepository(root, overrides = new Map()) {
  const workflowSource = readFile(root, WORKFLOW, overrides);
  validateDockerWorkflow(workflowSource);
  for (const chartWorkflow of [
    ".github/workflows/helm-chart.yml",
    ".github/workflows/push-gateway-helm-chart.yml",
  ]) {
    validateChartWorkflow(
      readFile(root, chartWorkflow, overrides),
      chartWorkflow,
    );
  }

  const helmValues = parseYamlDocument(
    readFile(root, "deploy/charts/buzz/values.yaml", overrides),
    "deploy/charts/buzz/values.yaml",
  ).value;
  assert.equal(helmValues.image.repository, FORK_IMAGE);

  const compose = parseYamlDocument(
    readFile(root, "deploy/compose/compose.yml", overrides),
    "deploy/compose/compose.yml",
  ).value;
  assert.equal(
    compose.services.relay.image,
    "${BUZZ_IMAGE:-ghcr.io/zachlendon/buzz:main}",
  );

  const environment = parseEnv(
    readFile(root, "deploy/compose/.env.example", overrides),
  );
  assert.equal(environment.BUZZ_IMAGE, "ghcr.io/zachlendon/buzz:main");

  requireCount(
    readFile(
      root,
      "benchmarks/harbor-buzz-orchestra/scripts/benchmark.py",
      overrides,
    ),
    '"BUZZ_IMAGE": os.environ.get("BUZZ_IMAGE", "ghcr.io/zachlendon/buzz:main")',
    1,
    "benchmark image default changed",
  );
  requireCount(
    readFile(root, "Dockerfile", overrides),
    'org.opencontainers.image.source="https://github.com/zachlendon/buzz"',
    1,
    "Docker OCI source changed",
  );
  requireCount(
    readFile(root, "Dockerfile", overrides),
    'org.opencontainers.image.url="https://github.com/zachlendon/buzz"',
    1,
    "Docker OCI URL changed",
  );
  requireCount(
    readFile(root, "Dockerfile", overrides),
    'org.opencontainers.image.documentation="https://github.com/zachlendon/buzz#readme"',
    1,
    "Docker OCI documentation changed",
  );
  const ci = parseYamlDocument(
    readFile(root, ".github/workflows/ci.yml", overrides),
    ".github/workflows/ci.yml",
  ).value;
  const ciSteps = ci.jobs.changes.steps;
  const contractStep = ciSteps.filter(
    (step) => step.name === "Relay image publication contract",
  );
  assert.equal(contractStep.length, 1, "CI must have one relay contract step");
  assert.equal(contractStep[0].run, "pnpm test:relay-image-contract");
  const installStep = ciSteps.filter(
    (step) => step.name === "Install relay contract dependencies",
  );
  assert.equal(
    installStep.length,
    1,
    "CI must install relay contract dependencies",
  );
  assert.equal(
    installStep[0].run,
    "pnpm install --frozen-lockfile --filter buzz-workspace",
  );

  const packageManifest = JSON.parse(readFile(root, "package.json", overrides));
  assert.equal(
    packageManifest.scripts["test:relay-image-contract"],
    "scripts/test-relay-image-publication-contract.sh",
  );
  assert.equal(packageManifest.devDependencies.yaml, "2.9.0");

  validateDirectUpstreamReferences(root, overrides);
}

if (process.argv[1] === import.meta.filename) {
  const root = path.resolve(
    process.argv[2] ?? path.join(import.meta.dirname, ".."),
  );
  verifyRepository(root);
  console.log("relay image publication semantic contract passed");
}
