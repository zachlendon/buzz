# Sprout × mesh-llm (deez) — Integration Report

**Date:** 2026-05-19
**Scope:** Does Sprout have a "baked-in" agent? If yes, how do we plug
[`mesh-llm`](../../deez) in as the inference backend — ideally as a Rust
dependency that boots a relay-scoped mesh in serve mode.

---

## TL;DR

1. **Yes, Sprout has a baked-in agent.** The crate is `sprout-agent` — a
   first-party, ACP-compliant LLM agent. It is the only place in the Sprout
   workspace that talks HTTP to an LLM. Other "agent-looking" crates
   (`sprout-acp`, `sprout-mcp`, `sprout-cli`) are protocol bridges, not LLM
   clients.

2. **Plugging in mesh-llm is straightforward — at the wire level.**
   mesh-llm serves an **OpenAI-compatible HTTP API on `:9337/v1`**.
   `sprout-agent` already speaks the OpenAI Chat Completions / Responses API
   via a configurable base URL (`OPENAI_COMPAT_BASE_URL`). So the
   *zero-code* integration is: run `mesh-llm serve --auto` somewhere and set
   `OPENAI_COMPAT_BASE_URL=http://localhost:9337/v1`. That works today.

3. **The "ultimate" form you described — a crate dep that boots a
   relay-scoped mesh — is feasible but constrained.**
   - mesh-llm's *client* is embeddable as a Rust crate (`mesh-api`).
   - mesh-llm's *server* is **not** a library: `mesh-llm-host-runtime`
     exposes only `run()` / `run_main()`; all its modules are private.
   - The right primitive, therefore, is **subprocess supervision**: a new
     `sprout-mesh` crate that boots `mesh-llm serve …` as a child of the
     Sprout daemon, scopes its lifetime to a specific relay/workspace, and
     exposes the local OpenAI endpoint to `sprout-agent`.

4. **The provider abstraction in `sprout-agent` is enum-based and
   deliberately not trait-based.** The author of `sprout-agent` explicitly
   rejected `Box<dyn Provider>` in favor of a `match` arm. Adding a `Mesh`
   variant is an option, but unnecessary — mesh-llm is wire-compatible with
   OpenAI, so the existing `Provider::OpenAi` arm suffices.

---

## 1. Sprout's "baked-in agent" — does it exist?

**Yes.** Three crates collaborate to give Sprout an end-to-end agent
experience, but only one of them is an LLM client:

| Crate | Role | LLM client? |
|---|---|---|
| `sprout-acp` | Daemon. Listens on relay, spawns and supervises an ACP-speaking child process. | No |
| `sprout-mcp` | MCP server (stdio) exposing Sprout tools (`send_message`, `get_channel_history`, …) to any agent. | No |
| `sprout-cli` | Machine-friendly CLI; "agent-first" = JSON in / JSON out / documented exit codes. | No |
| **`sprout-agent`** | **First-party ACP agent. Talks HTTP to Anthropic / OpenAI / OpenAI-compatible servers.** | **Yes** |

The full process tree at runtime is:

```
sprout-acp                            ← harness daemon, no LLM code
├── sprout-agent                      ← THE agent — calls Anthropic / OpenAI
│   └── sprout-mcp-server             ← stdio MCP server exposing Sprout tools
├── sprout-agent (#2)
│   └── sprout-mcp-server (#2)
└── …
```

### How a message becomes a reply

1. A user posts a kind-9 message mentioning the agent's pubkey.
2. `sprout-acp` (already connected to the relay over WebSocket with NIP-42
   auth) receives it, runs the author gate (`RespondTo::OwnerOnly` by
   default), and queues it on a per-channel FIFO.
3. `sprout-acp` sends an ACP `session/prompt` to its child agent over stdio.
4. The child agent (`sprout-agent`) loops:
   - `POST /v1/messages` (Anthropic) or `POST /v1/chat/completions` /
     `/v1/responses` (OpenAI) — see `crates/sprout-agent/src/llm.rs`.
   - Parse tool calls, dispatch them to `sprout-mcp-server` over stdio.
   - Feed results back; repeat until `stop_reason == end_turn`.
5. `sprout-mcp`'s `send_message` tool signs a kind-9 event and posts it via
   the relay's `POST /events` endpoint. The reply fans out to subscribers.

### Where the LLM provider is configured

**Not on `sprout-acp`** — the harness is LLM-agnostic. The provider is
configured on the child:

```env
# sprout-agent — these are the only LLM-control knobs in the workspace
SPROUT_AGENT_PROVIDER=openai          # or "anthropic"
OPENAI_COMPAT_API_KEY=…
OPENAI_COMPAT_MODEL=…
OPENAI_COMPAT_BASE_URL=https://api.openai.com/v1   # ← the integration seam
OPENAI_COMPAT_API=auto                # auto | chat | responses
```

> `OPENAI_COMPAT_BASE_URL` is the single point where you can route
> `sprout-agent`'s inference traffic anywhere that speaks OpenAI's API —
> vLLM, llama.cpp, Ollama, OpenRouter, Block Gateway, **and mesh-llm**.

### Author's stance on extending providers

From `crates/sprout-agent/README.md`:

> `Provider` is a Rust `enum` with one `match` in `Llm::complete`. There is
> no trait, no `Box<dyn>`, no async-trait. Adding a third provider is a
> `match` arm and one `body`/`parse` pair in `llm.rs`.

This is relevant because it tells us: a "mesh-native" provider variant would
be welcome stylistically, but is *not required* — mesh-llm rides in on the
existing OpenAI arm.

---

## 2. mesh-llm in one screen

- **What it is:** A distributed LLM inference layer (custom QUIC mesh via
  `iroh`, embedded patched `llama.cpp`) that exposes the union of its nodes
  as **one OpenAI-compatible API** on `http://localhost:9337/v1`.
- **CLI:** `mesh-llm serve [--auto | --join <token> | --model <ref> | --client]`.
- **Endpoints (`:9337`):** `/v1/models`, `/v1/chat/completions`,
  `/v1/completions`, `/v1/responses` — SSE streaming.
- **Management API (`:3131`):** `/api/status`, `/api/models`,
  `/api/runtime/*`, `/api/plugins/*`, SSE `/api/events`.
- **MCP:** Speaks MCP **both ways** — as a server (exposes plugin tools /
  blackboard over stdio/HTTP/TCP/UNIX) and as a client (can connect out to
  external MCP servers via `[[plugin]]` entries in `~/.mesh-llm/config.toml`).
- **Embeddability:**
  - `mesh-api` (path: `crates/mesh-api`) — public Rust client SDK
    (`MeshClient`, `ClientBuilder`, `ChatRequest`, `InviteToken`, …).
  - `mesh-llm-host-runtime` — the actual server runtime, but **modules are
    private**. `lib.rs` exposes only `run()` / `run_main()` / `VERSION`.
  - `mesh-llm-plugin` — plugin SDK if you want to author a plugin that runs
    *inside* mesh.
- **Mesh discovery:** `--auto`, `--join <token>`, or Nostr-based
  (`--discover`, `--mesh-discovery-mode nostr|mdns`). Worth noting: **mesh-llm
  already uses Nostr relays for mesh discovery**, which is a nice cultural fit
  with Sprout.

The crucial fact for us: from `sprout-agent`'s point of view, mesh-llm is
indistinguishable from OpenAI. Same `POST /v1/chat/completions`, same SSE,
same tool-call JSON.

---

## 3. Integration options, ranked

### Option A — Zero-code, env-only (works today)

Run mesh-llm anywhere reachable. Point sprout-agent at it.

```bash
# terminal 1
mesh-llm serve --auto --model Qwen3-8B-Q4_K_M

# terminal 2 — Sprout
export SPROUT_ACP_AGENT_COMMAND=sprout-agent
export SPROUT_AGENT_PROVIDER=openai
export OPENAI_COMPAT_BASE_URL=http://localhost:9337/v1
export OPENAI_COMPAT_MODEL=Qwen3-8B-Q4_K_M
export OPENAI_COMPAT_API_KEY=unused      # mesh-llm ignores it but reqwest expects one
sprout-acp
```

- **Pros:** No new code, no new crates, no coupling. Validates the wire
  protocol immediately. Easy to demo.
- **Cons:** mesh lifecycle is the operator's problem. No relay-scoped
  isolation. Manual config.

**Recommendation:** ship this *first* as the "Tier 0" docs path. It is the
test that proves all the harder options will work.

### Option B — `sprout-mesh` crate: subprocess supervisor (recommended)

Add a new workspace crate, `crates/sprout-mesh`, that:

1. Spawns `mesh-llm serve …` as a tokio child process.
2. Owns a unique mesh identity (owner keystore) per Sprout workspace/relay.
3. Reads relay-scoped config to decide:
   - which models to keep warm (`--model …`),
   - whether to `--publish` (announce to a public mesh) or stay private,
   - whether to `--join <token>` an existing mesh provisioned for this
     relay.
4. Probes `http://127.0.0.1:<port>/v1/models` for readiness.
5. Exposes a small Rust API: `MeshHandle { base_url, models(), shutdown() }`.
6. Tears the mesh down when the supervisor exits (SIGKILL via process group,
   matching how `sprout-acp` already supervises `sprout-agent`).

Then `sprout-acp` (or a thin wrapper binary) becomes:

```text
sprout-acp
├── sprout-mesh                       ← NEW: supervises mesh-llm
│   └── mesh-llm serve … (child)
├── sprout-agent #1                   ← OPENAI_COMPAT_BASE_URL injected
│   └── sprout-mcp-server #1
├── sprout-agent #2
│   └── sprout-mcp-server #2
└── …
```

**Why this shape?**

- It matches the established pattern: `sprout-acp` already supervises
  `sprout-agent` and `sprout-mcp-server` over stdio with process groups,
  respawn, and idle timeouts. `sprout-mesh` would be the same idea applied
  to a third subprocess.
- It keeps mesh-llm's licensing and dependency surface (llama.cpp, CUDA,
  Metal, iroh, …) **out of the Sprout build graph**. `sprout-mesh` depends
  on no mesh code at compile time — it only needs the `mesh-llm` binary on
  `PATH` (or a configured path).
- Relay-scoped lifecycle: when a Sprout workspace switches relays (desktop)
  or the daemon stops, the mesh subprocess dies with it.
- It does not require `mesh-llm-host-runtime` to be a library, which it
  isn't.

**Why not depend on the `mesh-api` crate?**
You could — but `mesh-api` is the *client* SDK. It joins an existing mesh
and lets you call `chat()`. It does not run inference itself. If we used
`mesh-api`, we would still need a mesh server somewhere to talk to. So
embedding `mesh-api` only makes sense if we explicitly want Sprout to
**join** a mesh, not **host** one. (See Option C.)

**Sketch — `crates/sprout-mesh/Cargo.toml`:**

```toml
[package]
name = "sprout-mesh"
version = "0.1.0"
edition = "2021"
description = "Supervises a local mesh-llm process scoped to a Sprout relay/workspace."

[dependencies]
tokio = { workspace = true, features = ["process", "macros", "rt-multi-thread", "fs", "time"] }
reqwest = { workspace = true, features = ["json"] }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
```

**Sketch — `crates/sprout-mesh/src/lib.rs`:**

```rust
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::{Child, Command};
use tokio::time::{sleep, Duration, Instant};

pub struct MeshConfig {
    pub binary: PathBuf,           // SPROUT_MESH_BIN, default "mesh-llm"
    pub port: u16,                 // 9337
    pub console_port: u16,         // 3131
    pub models: Vec<String>,       // --model entries
    pub auto: bool,                // --auto
    pub join_tokens: Vec<String>,  // --join
    pub publish: bool,             // --publish
    pub headless: bool,            // --headless (no React console)
    pub owner_key: Option<PathBuf>,
    pub extra_args: Vec<String>,
    pub runtime_root: Option<PathBuf>, // MESH_LLM_RUNTIME_ROOT, scoped per relay
}

pub struct MeshHandle {
    child: Child,
    pub base_url: String,          // "http://127.0.0.1:9337/v1"
    pub console_url: String,       // "http://127.0.0.1:3131"
}

impl MeshHandle {
    pub async fn spawn(cfg: MeshConfig) -> Result<Self, MeshError> {
        let mut cmd = Command::new(&cfg.binary);
        cmd.arg("serve")
            .arg("--port").arg(cfg.port.to_string())
            .arg("--console").arg(cfg.console_port.to_string());
        if cfg.auto      { cmd.arg("--auto"); }
        if cfg.publish   { cmd.arg("--publish"); }
        if cfg.headless  { cmd.arg("--headless"); }
        for m in &cfg.models       { cmd.arg("--model").arg(m); }
        for t in &cfg.join_tokens  { cmd.arg("--join").arg(t); }
        if let Some(k) = &cfg.owner_key { cmd.arg("--owner-key").arg(k); }
        if let Some(root) = &cfg.runtime_root {
            cmd.env("MESH_LLM_RUNTIME_ROOT", root);
        }
        cmd.args(&cfg.extra_args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);      // kill tree on shutdown — matches sprout-acp pattern
        let child = cmd.spawn()?;
        let me = Self {
            child,
            base_url:   format!("http://127.0.0.1:{}/v1", cfg.port),
            console_url:format!("http://127.0.0.1:{}",   cfg.console_port),
        };
        me.wait_ready(Duration::from_secs(60)).await?;
        Ok(me)
    }

    async fn wait_ready(&self, timeout: Duration) -> Result<(), MeshError> {
        let deadline = Instant::now() + timeout;
        let client = reqwest::Client::new();
        let url = format!("{}/models", self.base_url);
        while Instant::now() < deadline {
            if let Ok(r) = client.get(&url).send().await {
                if r.status().is_success() { return Ok(()); }
            }
            sleep(Duration::from_millis(500)).await;
        }
        Err(MeshError::NotReady)
    }

    pub async fn shutdown(mut self) -> Result<(), MeshError> {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
        Ok(())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum MeshError {
    #[error("mesh-llm not ready in time")]
    NotReady,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
```

**Sketch — wiring into `sprout-acp`:**

In `crates/sprout-acp/src/lib.rs::run`, before `AgentPool::spawn(...)`,
optionally bring up a mesh and inject env into each child:

```rust
let mesh = if cfg.mesh_enabled {
    Some(sprout_mesh::MeshHandle::spawn(cfg.mesh.clone()).await?)
} else { None };

let extra_env: Vec<(String, String)> = mesh
    .as_ref()
    .map(|m| vec![
        ("SPROUT_AGENT_PROVIDER".into(),   "openai".into()),
        ("OPENAI_COMPAT_BASE_URL".into(),  m.base_url.clone()),
        ("OPENAI_COMPAT_API_KEY".into(),   "mesh".into()),  // placeholder
    ])
    .unwrap_or_default();

// pass extra_env into AgentPool / AcpClient::spawn(..., &extra_env)
```

Note that **operator-set env vars already take precedence** (existing
`if std::env::var(key).is_err()` guard in `acp.rs:206`), so a user who
*wants* to point to Anthropic instead of the embedded mesh just exports
`SPROUT_AGENT_PROVIDER=anthropic` and the mesh-derived defaults defer.

**New env vars introduced by `sprout-mesh`:**

| Var | Default | Purpose |
|---|---|---|
| `SPROUT_MESH_ENABLED` | `false` | Master switch. Off by default. |
| `SPROUT_MESH_BIN` | `mesh-llm` | Path / name of the mesh-llm binary. |
| `SPROUT_MESH_PORT` | `9337` | OpenAI port. |
| `SPROUT_MESH_CONSOLE_PORT` | `3131` | Management/console port. |
| `SPROUT_MESH_MODELS` | — | Comma-separated `--model` refs. |
| `SPROUT_MESH_AUTO` | `false` | Pass `--auto` (discover + join). |
| `SPROUT_MESH_JOIN` | — | Comma-separated invite tokens. |
| `SPROUT_MESH_PUBLISH` | `false` | Pass `--publish`. |
| `SPROUT_MESH_HEADLESS` | `true` | Pass `--headless`. |
| `SPROUT_MESH_RUNTIME_ROOT` | `~/.sprout/mesh/<relay-id>/` | Per-relay isolation. |
| `SPROUT_MESH_OWNER_KEY` | — | Owner keystore path. |

### Option C — Sprout *joins* an external mesh via `mesh-api`

Instead of (or in addition to) spawning a server, depend on the `mesh-api`
crate as a Rust path/version dependency and use `MeshClient` directly from
`sprout-agent`:

```toml
# crates/sprout-agent/Cargo.toml
mesh-api = { path = "../../../deez/crates/mesh-api", optional = true }

[features]
mesh = ["dep:mesh-api"]
```

Add `Provider::Mesh` variant in `crates/sprout-agent/src/config.rs`, add a
`match` arm in `Llm::complete` that calls `MeshClient::chat`, and require
`SPROUT_MESH_INVITE_TOKEN` + `SPROUT_MESH_OWNER_KEY` in config.

- **Pros:** No HTTP hop, no port-binding. Native QUIC. Sprout becomes a
  legitimate participant in a mesh — e.g., a community runs a shared mesh,
  every Sprout instance joins it.
- **Cons:** Couples Sprout's build to mesh-llm's transitive dep tree
  (`iroh`, protobuf, GGUF, etc.). Adds operational complexity (owner keys,
  invite tokens). Violates the author's stated "no new `dyn Provider`"
  preference — though a new enum arm is still fine.
- **When to choose:** Only if "join the team's mesh" is the dominant UX
  story. For "the relay also hosts inference", Option B is cleaner.

### Option D — mesh-llm as a Sprout plugin (or vice versa)

mesh-llm has a first-class **plugin system** that can re-expose external
MCP servers' tools as part of mesh's surface. Two sub-options:

- **D1.** Register `sprout-mcp` as a plugin in `~/.mesh-llm/config.toml`:
  ```toml
  [[plugin]]
  name = "sprout"
  command = "sprout-mcp-server"
  env = ["SPROUT_RELAY_URL", "SPROUT_PRIVATE_KEY"]
  ```
  Now any client of mesh-llm (Goose, Claude Code, OpenCode, mesh's own MoA
  gateway) has Sprout tools.

- **D2.** Author a tiny mesh-llm plugin in Rust (using the
  `mesh-llm-plugin` SDK) that wraps `sprout-sdk` directly and avoids the
  stdio MCP hop.

This is **complementary** to Options A/B/C, not a substitute. It is what
you'd do to make Sprout tools available *from inside* mesh, regardless of
who's calling the inference API.

---

## 4. Recommended path

A staged rollout that ships value at each step:

**Stage 1 — Docs + smoke test (no code).**
Add a section to `crates/sprout-agent/README.md` showing how to point at
`http://localhost:9337/v1`. This is Option A. Validates the wire protocol.

**Stage 2 — `crates/sprout-mesh` crate (Option B).**
Sketched above. ~200–400 LOC. No mesh-llm code in our build graph; just
subprocess supervision. Add an opt-in flag (`SPROUT_MESH_ENABLED=true`) so
existing deployments are untouched. Inject `OPENAI_COMPAT_*` env vars into
spawned `sprout-agent` children. Per-relay `MESH_LLM_RUNTIME_ROOT` for
isolation.

**Stage 3 — Wire `sprout-mesh` into the desktop app.**
Workspace-scoped: the desktop app's `useWorkspaceInit.ts` already has a
reset hook (see AGENTS.md "Workspace Switching"). Adding `mesh.shutdown()`
on workspace switch is consistent with the existing pattern for
`relayClient.disconnect()` etc.

**Stage 4 — Register sprout-mcp as a mesh plugin (Option D1).**
One-line config change on the mesh side. Makes Sprout tools available to
every consumer of that mesh.

**Stage 5 (optional, later) — `mesh-api` Rust dependency (Option C).**
Only if community-mesh-join becomes a real workflow. Adds `Provider::Mesh`
to `sprout-agent` behind a `mesh` cargo feature.

---

## 5. Risks and notes

- **Build graph contagion (Option C only).** Pulling `mesh-api` in pulls
  `iroh`, possibly `protobuf-codegen`, and GGUF helpers. Keep behind a cargo
  feature, off by default. For Option B this is a non-issue because the
  dependency is just "`mesh-llm` binary exists on `PATH`".
- **Port collisions.** mesh-llm defaults to `:9337` (OpenAI) and `:3131`
  (console). The Sprout relay defaults to `:3000`. No collision, but the
  desktop app's Vite dev server is `:1420` — also clear. Add config knobs
  in case someone wants a non-default port.
- **Headless vs. console UI.** mesh-llm bundles a React console at `:3131`.
  Default `--headless true` in `sprout-mesh` to avoid surprise UI; expose a
  knob to turn it back on for debugging.
- **Identity surface.** mesh-llm has its own owner keypair / invite tokens.
  This is *separate* from Sprout's `SPROUT_PRIVATE_KEY` (nsec). They should
  remain distinct: Sprout's key authorizes the relay, mesh's key authorizes
  the mesh. Document clearly.
- **Mesh discovery uses Nostr too.** mesh-llm uses a Nostr relay for
  discovery (`--nostr-relay`). Cultural fit aside, **be careful not to share
  the same relay**, or at least, not to point mesh-llm's discovery at the
  Sprout relay that the agent connects to — unless you've vetted that
  mesh-llm's discovery events won't pollute Sprout's event surface. Use a
  public Nostr relay (`relay.damus.io` etc.) for mesh discovery by default.
- **License compatibility.** mesh-llm is `MIT OR Apache-2.0`. Sprout's
  workspace is the same (verify in root `Cargo.toml`). Embedding (Option C)
  is fine; subprocess (Option B) is trivially fine.
- **CI.** `just ci` should *not* require a mesh-llm binary by default. Gate
  any mesh-related integration tests behind a feature flag or a
  `SPROUT_MESH_BIN`-present check, similar to how `just test` already gates
  on Postgres/Redis being up.
- **Tool-call semantics.** mesh-llm passes OpenAI tool-call JSON through
  unmodified. `sprout-agent` already handles the OpenAI tool-call shape via
  its `chat/completions` and `responses` arms. Worth running the existing
  `e2e_mcp.rs` against a mesh-backed setup to confirm round-trip.
- **MoA model id.** mesh-llm reserves the model id `"mesh"` for its
  Mixture-of-Agents fan-out. If a Sprout user sets `OPENAI_COMPAT_MODEL=mesh`,
  they'll get MoA behavior automatically. Worth surfacing as a knob in
  `sprout-mesh` config (e.g., `SPROUT_MESH_USE_MOA=true` → defaults
  `OPENAI_COMPAT_MODEL=mesh`).

---

## 6. Concrete next steps

If we want to land Stage 1 + Stage 2 in a single PR:

1. **`crates/sprout-mesh/`** — new crate (Cargo.toml + lib.rs + a small
   `config.rs` reading `SPROUT_MESH_*` env vars).
2. **`Cargo.toml` (root)** — add `crates/sprout-mesh` to `members`.
3. **`crates/sprout-acp/Cargo.toml`** — add `sprout-mesh = { path = "../sprout-mesh", optional = true }` and a `mesh` feature.
4. **`crates/sprout-acp/src/lib.rs`** — in `run()`, behind `#[cfg(feature = "mesh")]`, conditionally spawn `MeshHandle` and inject `OPENAI_COMPAT_*` into `extra_env` for the agent pool.
5. **`.env.example`** — document `SPROUT_MESH_*` knobs with a commented
   example.
6. **`crates/sprout-agent/README.md`** — append a "Using mesh-llm" section
   pointing at the `:9337/v1` endpoint (Option A docs).
7. **`AGENTS.md`** — add `crates/sprout-mesh` to the repo structure table
   (and finally fix the missing `sprout-agent` / `sprout-dev-mcp` entries
   while we're there).
8. **Optional test:** add a `crates/sprout-test-client/tests/e2e_mesh.rs`
   that is gated on `MESH_LLM_BIN` being set, spawns a mesh-llm with a tiny
   model, runs a sprout-agent ACP round-trip, and asserts a reply event was
   posted. Excluded from `just ci` by default.

Total: ~300–500 net new LOC, zero changes to `sprout-agent` itself, no new
HTTP dependencies, no changes to the relay or DB.

---

## Appendix — file references

**Sprout (this repo):**
- `crates/sprout-agent/src/llm.rs` — `Llm::complete`; OpenAI/Anthropic dispatch.
- `crates/sprout-agent/src/config.rs` — `Provider` enum, env var loading.
- `crates/sprout-agent/README.md` — provider extension policy.
- `crates/sprout-acp/src/lib.rs` — `run()` (entry), `build_mcp_servers()`, `AgentPool`.
- `crates/sprout-acp/src/acp.rs` — `AcpClient::spawn(command, args, extra_env)`.
- `.env.example` — current env-var contract.

**mesh-llm (`../deez`):**
- `crates/mesh-llm/src/main.rs` — binary entry.
- `crates/mesh-llm-host-runtime/src/lib.rs` — public `run()` / `run_main()`.
- `crates/mesh-llm-host-runtime/src/cli/mod.rs` — CLI flags.
- `crates/mesh-llm-host-runtime/src/network/openai/transport.rs` — `/v1` server.
- `crates/openai-frontend/src/backend.rs` — `OpenAiBackend` trait.
- `crates/mesh-api/src/lib.rs` — embeddable Rust client SDK.
- `crates/mesh-llm-plugin/src/lib.rs` — plugin SDK (for Option D2).
- `docs/design/MOA_GATEWAY.md` — Mixture-of-Agents semantics.
