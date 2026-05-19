# Sprout × mesh-llm — Deep Dive: Relay-Scoped Auto-Join

**Companion to** `MESH_LLM_INTEGRATION.md`.
**Goal:** Make every Sprout instance connected to the same relay automatically
land in the same mesh-llm mesh — no manual token paste, no out-of-band coordination.

This document corrects and extends the previous report. The previous report
treated mesh-llm as a black-box subprocess; that misses the point. mesh-llm
**already speaks Nostr for discovery**, and once you understand its discovery
protocol, the integration becomes much more elegant: Sprout's relay *is* the
rendezvous, and `mesh-id` derivation makes "joining the right mesh" automatic.

---

## TL;DR

- mesh-llm publishes a **kind 31990** Nostr event (NIP-89 service advertisement,
  replaceable, d-tag `mesh-llm`) whose JSON content contains the **invite token**,
  the **`mesh_id`**, served models, VRAM, etc. TTL via standard NIP-40
  `expiration` tag.
- Discovery is just `REQ` for `kind:31990 #k:mesh-llm` on a configured relay set.
- The Nostr identity used for publishing is **separate** from the mesh owner key:
  it's an auto-generated `nsec` stored at `~/.mesh-llm/nostr.nsec`.
- Default discovery relays are public (`relay.damus.io`, `nos.lol`, etc.) but
  **`--nostr-relay <URL>`** overrides them. **This is the integration seam.**
- Because the discovery event is just JSON-with-an-invite-token over Nostr, we
  can either:
  1. **(Simple, recommended first)** Point `--nostr-relay` at the Sprout relay
     and let mesh-llm's own publish/discover loop do its thing. The Sprout
     relay just transports the events. Every Sprout instance pointed at the
     same relay sees the same mesh.
  2. **(Sprout-native, ideal)** Publish/discover the same listing event
     ourselves from `sprout-mesh` using Sprout's `h`-tag-scoped event surface,
     so the mesh listing lives **inside the channel** and inherits NIP-29
     auth/membership for free.

The first option works today with zero patches to mesh-llm. The second
gives us relay-channel-scoped meshes with no extra ACL plumbing.

---

## 1. The exact wire protocol

From `crates/mesh-llm-host-runtime/src/network/nostr.rs` (verbatim where
relevant):

### 1.1 The event

```rust
pub const MESH_SERVICE_KIND: u16 = 31990;     // NIP-89 application handler

let tags = vec![
    Tag::custom(TagKind::Custom("d".into()),  vec!["mesh-llm".to_string()]),
    Tag::custom(TagKind::Custom("k".into()),  vec!["mesh-llm".to_string()]),
    Tag::custom(TagKind::Custom("expiration".into()),
                vec![expiration.to_string()]),   // NIP-40 TTL
];
let builder = EventBuilder::new(Kind::Custom(MESH_SERVICE_KIND), content).tags(tags);
self.client.send_event_builder(builder).await?;
```

- **Kind:** `31990` (NIP-89). Replaceable in the 30000–39999 range — one
  listing per (pubkey, d-tag) pair, so each publisher has exactly one
  current listing.
- **`d` tag:** `"mesh-llm"` (replaceable-event identifier).
- **`k` tag:** `"mesh-llm"` (also `"mesh-llm"` — used as the filter predicate
  on the discover side; allows multiple coexisting service classes under
  31990).
- **`expiration` tag (NIP-40):** Unix-seconds TTL, `now + ttl` where
  `ttl = interval_secs * 2`. Default republish interval is in
  `PublishLoopConfig.interval_secs`; meshes age out if the publisher dies.
- **Content:** JSON serialization of `MeshListing` (see §1.2).
- **Signing identity:** A dedicated Nostr key, generated on first run and
  saved to `~/.mesh-llm/nostr.nsec` (mode 0600). **Not** the mesh owner key,
  **not** the iroh node id. Rotated by `mesh-llm auth rotate-nostr-keys`.

### 1.2 The content payload

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshListing {
    pub invite_token: String,           // base64-ish; opaque blob from mesh-client
    pub serving: Vec<String>,           // models actively loaded
    #[serde(default)] pub wanted: Vec<String>,
    #[serde(default)] pub on_disk: Vec<String>,
    pub total_vram_bytes: u64,
    pub node_count: usize,
    #[serde(default)] pub client_count: usize,
    #[serde(default)] pub max_clients: usize,
    #[serde(skip_serializing_if = "Option::is_none")] pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh_id: Option<String>,        // ★ stable mesh identity
}
```

The **`invite_token`** is what the `--join` flag consumes. It is opaque
(`mesh_client::InviteToken(String)` newtype — see `crates/mesh-api/src/token.rs`).
Internally it encodes iroh bootstrap node addresses + mesh owner pubkey + ALPN,
but we do not need to understand its bytes; we just shuttle it through.

The **`mesh_id`** is the stable identity all nodes in the same mesh agree on:

```rust
// crates/mesh-llm-host-runtime/src/mesh/mod.rs
pub async fn set_mesh_id(&self, id: String) {
    let mut current = self.mesh_id.lock().await;
    if current.is_none() {
        *current = Some(id);   // first writer wins; gossip propagates
        ...
    }
    // already set → ignore (originator's ID wins)
}
```

This is critical: **two publishers of the same mesh publish the same
`mesh_id`** because they discover each other via gossip and the originator's
ID propagates. Two different meshes — even with the same `name` — have
different `mesh_id`s.

### 1.3 The filter (discover side)

```rust
let nostr_filter = Filter::new()
    .kind(Kind::Custom(MESH_SERVICE_KIND))     // 31990
    .custom_tag(SingleLetterTag::lowercase(Alphabet::K),
                "mesh-llm".to_string())         // #k=mesh-llm
    .limit(100);
```

Just `REQ {"kinds":[31990],"#k":["mesh-llm"],"limit":100}`. Vanilla Nostr.
**Any compliant relay supports this** — no NIPs required beyond the basics
(no NIP-29 needed for discovery alone). Sprout's relay supports replaceable
events and tag filters, so this works as-is.

### 1.4 The relay set

```rust
pub const DEFAULT_RELAYS: &[&str] = &[
    "wss://relay.damus.io",
    "wss://nos.lol",
    "wss://relay.nostr.band",
    "wss://nostr.land",
    "wss://nostr.wine",
];

pub(crate) fn nostr_relays(cli_relays: &[String]) -> Vec<String> {
    if cli_relays.is_empty() {
        nostr::DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect()
    } else {
        cli_relays.to_vec()      // ← --nostr-relay wins; entirely replaces defaults
    }
}
```

**`--nostr-relay <URL>` (repeatable) entirely replaces the public default
list.** This is the cleanest integration point: point it at the Sprout relay
and discovery happens *only* there.

### 1.5 Filtering / matching

```rust
pub struct MeshFilter {
    pub name: Option<String>,     // case-insensitive exact name match
    pub model: Option<String>,    // substring across serving/wanted/on_disk
    pub min_vram_gb: Option<f64>,
    pub region: Option<String>,
}
```

`--discover "lab-a"` becomes `MeshFilter { name: Some("lab-a"), .. }` and
matches by listing's `name`. `--auto` uses `MeshFilter::default()` and picks
by `score_mesh`, which biases toward (a) the `last-mesh` id remembered at
`~/.mesh-llm/last-mesh` and (b) larger meshes.

### 1.6 Same-mesh consolidation

If two nodes independently start as "originators" of a mesh with the same
`name`, they'll publish *different* `mesh_id`s. `maybe_rejoin_larger_mesh`
runs in the publish loop and tells the smaller mesh to migrate into the
larger one. Net effect: **the mesh-llm network self-heals into a single
mesh per (publisher, name) cluster.**

This matters for our integration: if two Sprout instances simultaneously
boot mesh-llm without coordination, they'll briefly form two meshes, then
converge to one within an interval cycle (default ~60s, jittered).

---

## 2. What we actually need

The user story:

> Two Sprout users, both connected to relay `wss://my.sprout.example`, both
> have `mesh-llm` installed. Neither has done any setup. Both should
> automatically be on the same mesh-llm mesh, sharing inference capacity.

Translating to mesh-llm primitives:

1. Both nodes need to publish/discover via **the same Nostr relay**.
2. Both nodes need a way to **agree on a mesh `name`** (or be configured to
   filter on it) so they self-consolidate via `maybe_rejoin_larger_mesh`.
3. The first node to start needs to **publish** (be a server). The second
   needs to **discover and join** (which can also be a server or a client).

That's it. There is no further "joining ceremony" needed. Once both
publish + discover on the same relay with the same name, `mesh-llm`'s own
convergence logic does the rest.

---

## 3. Three concrete designs, increasing depth

### Design A — Just point mesh-llm at the Sprout relay (zero code)

```bash
# Operator runs this once per host:
mesh-llm serve --auto --publish \
  --nostr-relay wss://my.sprout.example \
  --mesh-name sprout-my.sprout.example
```

`sprout-mesh` would simply build this command line:

```rust
let mesh_name = format!("sprout-{}",
    relay_url.host_str().unwrap_or("unknown"));

cmd.arg("serve")
   .arg("--auto")
   .arg("--publish")
   .arg("--nostr-relay").arg(&relay_url.to_string())
   .arg("--mesh-name").arg(&mesh_name)
   .arg("--headless")
   .arg("--port").arg(cfg.port.to_string());
```

#### What this gets us

- ✅ Every Sprout instance pointed at the same relay finds and converges to
  one mesh, named after the relay host.
- ✅ Convergence is automatic via mesh-llm's existing `maybe_rejoin_larger_mesh`.
- ✅ Zero new event kinds, zero protocol work.
- ✅ Works with any Nostr-protocol-compatible relay — the Sprout relay
  doesn't even need to know it's hosting mesh discovery.

#### Caveats

- ⚠️ The Sprout relay sees mesh-discovery traffic on a separate Nostr key
  (`~/.mesh-llm/nostr.nsec`), not the user's Sprout nsec. **The publisher
  pubkey is not authenticated to the relay as a Sprout user.** Open relays
  accept it; an auth-gated Sprout relay (NIP-42) might reject it.
- ⚠️ The mesh is keyed by *relay host*, not by *Sprout channel*. Everyone
  on the relay shares one mesh regardless of channel membership.
- ⚠️ Random pubkeys publishing arbitrary kind-31990 events to the Sprout
  relay add noise. If your relay's event store is on a tight budget, this
  matters; mesh-llm republishes every ~60s.
- ⚠️ Mesh-llm publishes its **own** Nostr key alongside the listing. The
  relay can't verify "this person is on my Sprout relay" — only "some pubkey
  published a mesh listing". Not a security issue (the invite token already
  protects mesh access), but it does mean we can't gate at the discovery
  layer.

### Design B — Channel-scoped meshes, Sprout publishes the listing

Move the listing into a Sprout-native event so it inherits channel ACLs.

Mesh-llm still runs unmodified, but instead of `--publish`/`--discover`
through Nostr, `sprout-mesh` issues the publish/discover side itself,
treating mesh-llm purely as a server with a static invite token.

**Boot sequence (publisher):**

1. `sprout-mesh` boots a private `mesh-llm serve --model … --headless --port 9337`.
   No `--publish`, no `--nostr-relay`.
2. Wait for `/v1/models` to come up.
3. Call the management API to obtain the invite token. From
   `/api/runtime/control/get-config` or — simpler — by passing
   `--print-invite-token` (not currently a flag; would need a tiny mesh-llm
   patch, OR scrape stdout where it's printed on startup, OR call
   `GET /api/runtime` which exposes the running config). The cleanest
   non-patching path is **stdout capture**: mesh-llm prints the token on
   launch as a line containing `mesh-llm serve --join `. Match that line.
4. `sprout-mesh` publishes a **Sprout-native kind** (say `kind:32990`,
   channel-scoped with an `h` tag, replaceable, d-tag `mesh-llm`) whose
   content is the same `MeshListing` JSON. **Signed with the user's
   Sprout nsec.**
5. Republish on an interval; emit a delete on shutdown.

**Boot sequence (joiner):**

1. `sprout-mesh` subscribes to `{ kinds:[32990], "#h":[<channel>], "#k":["mesh-llm"] }`
   over the existing Sprout WebSocket connection.
2. On receipt of a listing, pluck `invite_token`.
3. Boot `mesh-llm serve --join <token> --headless --port 9337`.
4. (Or, if no GPU: `mesh-llm client --join <token>`.)

**What this gets us:**

- ✅ Channel-scoped meshes: every NIP-29 channel can have its own
  inference mesh.
- ✅ Listings are authenticated to a Sprout pubkey — only authorized members
  of the channel (via the relay's NIP-29 enforcement) can publish or read
  them.
- ✅ Mesh-llm runs entirely private (`--publish`-less). No public Nostr
  exposure.
- ✅ Reuses Sprout's existing event bus, no separate Nostr connection.

**Caveats / costs:**

- One new event kind to allocate in `crates/sprout-core/src/kind.rs`
  (suggested `MESH_LLM_LISTING = 32990`, also NIP-89-flavored, replaceable).
- Need to extract the invite token from mesh-llm at startup. Easiest:
  *stdout scrape*. Cleaner: small upstream PR to add `--emit-invite-token`
  that prints `INVITE_TOKEN=<token>` on a line by itself, OR to add a
  `GET /api/runtime/invite-token` endpoint. **Both are trivial upstream
  patches.**
- `sprout-mesh` becomes a small protocol implementation rather than a
  thin wrapper.

### Design C — Custom Nostr-relay glue inside mesh-llm (NOT recommended)

Patch mesh-llm to read the Sprout-side `h` tag when publishing/discovering
to a Sprout relay, so its native `--publish/--discover` cycle becomes
channel-aware. We'd lift `sprout-core::kind::MESH_LLM_LISTING` into
mesh-llm and add a `--mesh-channel <h>` CLI flag.

This is the most "native" feeling but requires sustained upstream changes
and locks the two projects' release cadences together. **Skip unless
Design B proves insufficient.**

---

## 4. Recommended path (revised)

Replace the previous "Stage 2" with the following two stages:

### Stage 2a — Ship Design A first (one PR)

`crates/sprout-mesh` boots `mesh-llm serve` with `--nostr-relay <sprout-relay-url>`
and `--mesh-name sprout-<relay-host>`. Done. Validates end-to-end
discovery + convergence with zero protocol work.

Operator UX:
```env
SPROUT_MESH_ENABLED=true
SPROUT_MESH_MODELS=Qwen3-8B-Q4_K_M
# everything else has a sensible default derived from SPROUT_RELAY_URL
```

### Stage 2b — Migrate to Design B (second PR)

Once Design A proves out, add the Sprout-native listing event:

1. **`sprout-core/src/kind.rs`** — define `MESH_LLM_LISTING = 32990`
   (NIP-89-flavored, replaceable; document the schema as `MeshListing`).
2. **`sprout-mesh`** — add a publisher task (Tokio loop, signs and posts
   to the Sprout relay over the existing WebSocket via `POST /events`) and
   a discoverer task (REQ subscription, picks invite token from latest
   replaceable event per pubkey).
3. **`sprout-mesh`** — drop the `--publish` / `--nostr-relay` /
   `--mesh-name` flags from the `mesh-llm` invocation. Mesh-llm is now
   strictly local; Sprout owns rendezvous.
4. **Token extraction** — start with stdout scrape (regex
   `serve --join (\S+)` on the first 50 lines of mesh-llm output). Open
   an upstream issue for a deterministic `GET /api/runtime/invite-token`
   endpoint and/or `--emit-invite-token` flag.

This is the "ultimate form" you described — a crate that boots mesh in
serve mode scoped to the relay sprout is on. The mesh's *existence* is
gated by the Sprout channel; the mesh's *membership* is gated by the
invite token; both layers ride on the same Nostr-shaped event surface.

### Stage 2c (optional, future) — Sprout-aware identity

If the operator wants the mesh to refuse joins from non-Sprout users,
re-publish the listing as an **encrypted** Nostr event (NIP-44 wrap, one
copy per channel member's pubkey) so the invite token is only readable by
authorized members. This is a pure Sprout-side change; mesh-llm doesn't
need to know.

---

## 5. Open questions worth pinning down before coding

1. **Token extraction interface.** Confirm whether `mesh-llm serve` prints
   the invite token deterministically on stdout, and in what format. If
   yes (likely), `sprout-mesh` can parse it; if no, upstream a small PR
   first.
2. **Per-channel vs per-relay scoping.** Design B is per-channel. Is that
   actually what we want, or do we want per-relay (one mesh shared across
   all channels on a relay)? Per-channel is more flexible (channels for
   different teams use different meshes); per-relay is simpler and matches
   how relay/workspace identity flows in the desktop today. Suggest
   **per-relay first, per-channel later** (use a sentinel `h` tag like
   `_workspace` for the per-relay listing).
3. **GPU-less users.** Most Sprout users won't have GPUs. They should run
   `mesh-llm client --join` (API-only). `sprout-mesh` should
   default-detect: if `mesh-llm gpus` reports no usable GPU, run in
   client mode. Mesh-llm exposes this via the `gpus` subcommand and via
   `/api/runtime/control/get-config`. Or we can just always start in
   client mode and let `--auto` + the publisher decide.
4. **First-mover problem.** Until someone with a GPU joins, the mesh has
   no inference capacity. `sprout-agent` will get `503`s from
   `:9337/v1/chat/completions`. We need a fallback chain in
   `sprout-agent`, or `sprout-mesh` needs to expose a "ready" status that
   gates agent spawn. Suggest: `sprout-acp` polls `mesh.console_url +
   /api/status` and only sets `OPENAI_COMPAT_BASE_URL` once a model is
   serving; otherwise falls back to operator-configured OPENAI/Anthropic
   creds (existing behavior). This makes mesh-llm a *bonus* tier of
   inference, not a hard dependency.
5. **Event noise budget.** Mesh-llm republishes every ~60s. Per-relay,
   that's `n_publishers × 1440 events/day`. For a 10-user relay, ~14k
   events/day in kind 32990, replaceable so only the latest survives. Not
   a problem.
6. **Nostr key reuse.** Should the Sprout-side publish use the user's
   Sprout nsec (clean, but exposes that pubkey on mesh metadata)? Or a
   derived per-relay key? Suggest: **user's Sprout nsec under Design B**
   — that's the point of moving it to a Sprout event. Anyone in the
   channel can already see who's posting; publishing a mesh listing is
   less revealing than a chat message.

---

## 6. Concrete diff sketches

### 6.1 New event kind (Design B)

```rust
// crates/sprout-core/src/kind.rs
/// NIP-89-style application handler advertising a mesh-llm mesh.
/// Replaceable event; d-tag `mesh-llm`; `k`-tag `mesh-llm`; `h`-tag the
/// Sprout channel (NIP-29). Content is a `MeshListing` JSON (compatible
/// with mesh-llm's own schema).
pub const MESH_LLM_LISTING: u32 = 32990;
```

### 6.2 `sprout-mesh` extracts the token

```rust
// crates/sprout-mesh/src/token.rs
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::ChildStdout;

static INVITE_RE: once_cell::sync::Lazy<regex::Regex> =
    once_cell::sync::Lazy::new(|| regex::Regex::new(r"serve\s+--join\s+(\S+)").unwrap());

pub async fn extract_invite(stdout: ChildStdout, timeout: Duration)
    -> Result<String, MeshError>
{
    let mut lines = BufReader::new(stdout).lines();
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        let line = tokio::time::timeout_at(deadline, lines.next_line()).await
            .map_err(|_| MeshError::TokenTimeout)?
            .map_err(MeshError::from)?
            .ok_or(MeshError::TokenTimeout)?;
        if let Some(m) = INVITE_RE.captures(&line) {
            return Ok(m[1].to_string());
        }
    }
    Err(MeshError::TokenTimeout)
}
```

### 6.3 Listing publisher (Design B)

```rust
// crates/sprout-mesh/src/publish.rs
pub async fn publish_listing_loop(
    relay: SproutRelayClient,           // posts via POST /events or WS
    channel: String,                    // h-tag
    signing_key: nostr::SecretKey,
    listing_fn: impl Fn() -> MeshListing,
    interval: Duration,
) {
    loop {
        let listing = listing_fn();
        let content = serde_json::to_string(&listing).unwrap();
        let ttl = (interval.as_secs() * 2).to_string();
        let event = EventBuilder::new(Kind::Custom(32990), content, [])
            .tag(Tag::generic(TagKind::D, ["mesh-llm"]))
            .tag(Tag::generic(TagKind::H, [channel.clone()]))
            .tag(Tag::generic(TagKind::K, ["mesh-llm"]))
            .tag(Tag::generic(TagKind::Custom("expiration".into()), [ttl]))
            .to_event(&Keys::new(signing_key.clone())).unwrap();
        let _ = relay.post_event(event).await;
        tokio::time::sleep(interval).await;
    }
}
```

### 6.4 Discoverer (Design B)

```rust
// crates/sprout-mesh/src/discover.rs
pub async fn discover_listing(
    relay: &SproutRelayClient,
    channel: &str,
) -> Result<Option<MeshListing>, MeshError> {
    let filter = json!({
        "kinds": [32990],
        "#h": [channel],
        "#k": ["mesh-llm"],
        "limit": 50
    });
    let events = relay.query(filter).await?;          // POST /query
    // Pick latest by created_at per pubkey, take overall newest.
    let latest = latest_per_pubkey(events);
    let best = latest.into_iter().max_by_key(|e| e.created_at)?;
    let listing: MeshListing = serde_json::from_str(&best.content)?;
    Ok(Some(listing))
}
```

That's the whole protocol surface. ~200 LOC of new Sprout code; zero
upstream patches needed for Stage 2a; one optional upstream PR for
Stage 2b (token extraction).

---

## 7. Summary

| Layer | Stage 2a (Design A) | Stage 2b (Design B) |
|---|---|---|
| Discovery transport | Sprout relay, mesh-llm's native publisher | Sprout relay, our publisher |
| Discovery key | `~/.mesh-llm/nostr.nsec` (separate) | User's Sprout nsec |
| Event kind | 31990 (NIP-89, mesh-llm native) | 32990 (Sprout's own, schema-compatible) |
| Channel scoping | No (relay-wide) | Yes (via `h` tag) |
| ACL | None (token-only) | Inherits NIP-29 channel ACLs |
| Upstream patches | None | Optional (deterministic token output) |
| LOC | ~150 | ~400 |
| Convergence | mesh-llm's built-in `maybe_rejoin_larger_mesh` | Same |

The mesh-llm wire protocol is small enough that we can re-implement the
discovery side natively in Sprout (~200 LOC) once we want channel scoping.
But the smart play is to ship Stage 2a first — it requires no protocol
work and proves the rest of the integration — then migrate to Stage 2b
once it's earning its keep.

---

## Appendix — file index

**mesh-llm:**
- `crates/mesh-llm-host-runtime/src/network/nostr.rs` — entire discovery
  protocol (publisher, discoverer, key handling, filtering, scoring,
  rejoin-larger-mesh logic). 2412 LOC.
- `crates/mesh-llm-host-runtime/src/runtime/discovery.rs:279` —
  `nostr_relays()`: the `--nostr-relay` overrides defaults entirely.
- `crates/mesh-llm-host-runtime/src/mesh/mod.rs:3438` —
  `Node::mesh_id()`, `set_mesh_id()`, gossip convergence.
- `crates/mesh-llm-host-runtime/src/mesh/mod.rs:8096` —
  `save_last_mesh_id`/`load_last_mesh_id` (sticky mesh selection across
  restarts).
- `docs/MESHES.md` — operator-facing workflow docs.
- `crates/mesh-api/src/token.rs`, `crates/mesh-api/src/identity.rs` —
  embeddable `InviteToken` and `OwnerKeypair` types.

**Sprout (proposed):**
- `crates/sprout-core/src/kind.rs` — `MESH_LLM_LISTING = 32990`.
- `crates/sprout-mesh/` — supervisor + publisher + discoverer + token
  extractor.
