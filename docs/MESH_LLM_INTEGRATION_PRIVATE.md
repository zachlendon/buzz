# Sprout × mesh-llm — Private, Members-Only Meshes

**Companion to** `MESH_LLM_INTEGRATION.md` and `MESH_LLM_INTEGRATION_DEEP.md`.
**Constraint:** The mesh must be **private**. Nothing published to public
relays; only authenticated members of a Sprout channel/relay can discover
the mesh, learn the invite token, or connect to mesh nodes.

This supersedes Design A (public Nostr discovery) and Design B (plaintext
in-channel listing) from the previous doc.

---

## TL;DR

Three layers of privacy, defense-in-depth:

1. **Transport privacy.** Mesh discovery never touches public Nostr.
   `mesh-llm` runs with neither `--publish` nor `--nostr-relay`. The
   listing only ever crosses the Sprout relay.
2. **Read privacy.** The listing event is **NIP-44 encrypted** to the
   current channel-member set (one ciphertext copy per member pubkey,
   same pattern Sprout's `observer` module already uses). The relay
   stores ciphertext; even a malicious relay operator can't read it.
3. **Cryptographic mesh-side enforcement.** Reading the invite token is
   not enough to join — joiners must present a **node ownership
   certificate** signed by the channel's mesh owner key. mesh-llm's
   existing `--trust-policy allowlist --trust-owner <id>` enforces this
   at the mesh layer. A stolen invite token is useless without the
   signed cert.

All three layers exist in code today; this design just composes them.

---

## 1. Threat model

| Adversary | Defended by |
|---|---|
| Random user on the public internet | Nothing published anywhere public. No discovery. |
| Random user on the Sprout relay (not a channel member) | NIP-29 channel ACLs: relay refuses to return events with the channel's `h` tag to non-members. |
| Curious / malicious Sprout relay operator | NIP-44 encryption of the listing; operator only sees ciphertext. **They can still see *that* an encrypted mesh listing exists, and the publisher's pubkey** — metadata, not content. |
| Ex-member with stale invite token | Owner key rotation + cert revocation. New listings encrypted to new member set; mesh refuses connections from revoked certs. |
| Channel member who shares the token externally | mesh-llm trust policy: external joiner doesn't have a signed cert, mesh refuses connection. |
| Compromised member device | Equivalent to channel-member access. No defense beyond revocation. |

The middle row — what the relay operator sees — is the realistic worry,
and NIP-44 fully addresses content; only metadata leaks. Acceptable for
the "everyone in the channel is on the mesh" use case.

---

## 2. Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│  Sprout Channel  (NIP-29, h-tag = <channel-id>)                  │
│  Members: A (admin), B, C                                        │
│                                                                  │
│  Member A runs:                                                  │
│    sprout-mesh (supervisor)                                      │
│      │                                                           │
│      ├── boots & owns:  mesh-llm serve --headless --port 9337    │
│      │                  --owner-key <channel-owner.keystore>     │
│      │                  --trust-policy allowlist                 │
│      │                  --trust-owner <channel-owner-id>         │
│      │                  (no --publish, no --nostr-relay)         │
│      │                                                           │
│      ├── extracts invite_token from mesh-llm                     │
│      │                                                           │
│      └── publishes Sprout event:                                 │
│            kind: 32990 (MESH_LLM_LISTING)                        │
│            tags: ["h", <channel>], ["d", "mesh-llm"],            │
│                  ["k", "mesh-llm"], ["p", B], ["p", C]           │
│            content: NIP-44(per-member, {invite_token, ...})      │
│                                                                  │
│  Member B runs:                                                  │
│    sprout-mesh (joiner)                                          │
│      │                                                           │
│      ├── subscribes: REQ {kinds:[32990], "#h":[<channel>],       │
│      │                    "#p":[<B's pubkey>]}                   │
│      ├── decrypts ciphertext with B's nsec                       │
│      ├── obtains: invite_token, channel-owner-id                 │
│      │                                                           │
│      └── boots:  mesh-llm serve --join <token>                   │
│                  --owner-key <B's-keystore-derived-from-channel> │
│                  --trust-policy allowlist                        │
│                  --trust-owner <channel-owner-id>                │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
                              ↓
                    QUIC, iroh, llama.cpp
                  (this layer is mesh-llm's;
                   sprout-mesh doesn't touch it)
```

---

## 3. The channel mesh-owner key

This is the keystone of the whole design.

A NIP-29 group already has an **admin set** (`kind:39001`,
`KIND_NIP29_GROUP_ADMINS`) and a **member set** (`kind:39002`,
`KIND_NIP29_GROUP_MEMBERS`) — both relay-signed. We mint **one mesh
owner keypair per channel**, and use it as the `--trust-owner` anchor.

### Generation

When a channel admin enables "channel mesh" for the first time:

1. `sprout-mesh` calls `mesh-llm auth init --owner-key <path>`
   (or programmatically with `mesh_api::OwnerKeypair::generate()`).
2. The generated keystore is **encrypted at rest** (mesh-llm prompts for
   a passphrase or stores in the OS keychain by default — see
   `AuthCommand::Init` in `crates/mesh-llm-host-runtime/src/cli/mod.rs`).
3. The **public owner ID** is broadcast in a new Sprout event:
   `kind:32991` (`MESH_LLM_OWNER`), `h`-tag scoped, replaceable, content:
   `{ "owner_id": "...", "alg": "ed25519", "created_at": ... }`. Public —
   it's just a pubkey, like the channel itself.
4. The **owner secret** is *distributed only to admins*: encrypted via
   NIP-44, one ciphertext copy per admin pubkey, in event `kind:32992`
   (`MESH_LLM_OWNER_SHARE`), tagged `["p", <admin-pubkey>]`. Schema:
   `{ "owner_id": "...", "signing_bytes": "<hex>", "encryption_bytes": "<hex>" }`.

This gives us:

- Every admin can mint new node certificates (e.g., for new members) by
  decrypting the owner share and running
  `mesh-llm auth sign-node --owner-key <decrypted>`.
- Non-admin members never see the owner secret. They get a node cert
  *signed by* the owner key from an admin (issuance flow below).
- Admin set is governed by NIP-29 — the channel's existing admin
  machinery doubles as the mesh's PKI root.

### Issuance flow (admin grants a member a cert)

When a new member joins the channel (NIP-29 membership event arrives):

1. An admin's `sprout-mesh` watches `kind:39002` (members list).
2. On new member `M`, the admin:
   - Decrypts the owner share locally.
   - Calls `mesh-llm auth sign-node` (or the equivalent `mesh-api` Rust
     function) targeting `M`'s mesh node-id — but `M`'s node-id doesn't
     exist yet. So we flip it: **`M` generates a node identity locally,
     publishes its node public key in `kind:32993`
     (`MESH_LLM_NODE_KEY`), and the admin signs it.**
3. The admin publishes the **signed node certificate** in `kind:32994`
   (`MESH_LLM_NODE_CERT`), `p`-tagged to `M`, NIP-44 encrypted to `M`'s
   pubkey. Content:
   `{ "claim": NodeOwnershipClaim, "signature": "<hex>" }`
   — which is exactly the `SignedNodeOwnership` struct from
   `crates/mesh-llm-host-runtime/src/crypto/ownership.rs:42`.
4. `M`'s `sprout-mesh` decrypts it, drops the cert at
   `<runtime-root>/node-ownership.json`, and starts `mesh-llm serve
   --join <token> --owner-key … --trust-policy allowlist --trust-owner
   <channel-owner-id>`.

mesh-llm enforces the rest. From `ownership.rs:473` and around the
control-plane handshake: a node missing or with an invalid cert under
`Allowlist` policy is **rejected at connect time**, before any inference
traffic flows.

### Rotation / revocation

- **Member leaves the channel:** an admin publishes a `kind:32995`
  (`MESH_LLM_REVOCATION`) for that member's node-endpoint-id and
  cert-id. The next listing republish (~60s) propagates a new invite
  token and a fresh `--trust-owner` if the admin chooses to rotate.
  Every active mesh-llm reads the revocation event from its local
  `sprout-mesh` and updates its trust store via
  `mesh-llm auth trust revoke-node`.
- **Owner key compromised:** mint a new channel mesh owner key, publish
  new `kind:32991` + `kind:32992`, every member rotates their cert
  against the new owner via the issuance flow above. Old token becomes
  worthless because the new mesh runs with the new `--trust-owner`.

---

## 4. The listing event (encrypted)

Schema unchanged from Design B except `content` is now ciphertext.

```rust
// crates/sprout-core/src/kind.rs
pub const MESH_LLM_LISTING:     u32 = 32990;  // encrypted invite + metadata
pub const MESH_LLM_OWNER:       u32 = 32991;  // channel mesh-owner pubkey (plaintext)
pub const MESH_LLM_OWNER_SHARE: u32 = 32992;  // owner secret, NIP-44'd to admin
pub const MESH_LLM_NODE_KEY:    u32 = 32993;  // member's node pubkey (plaintext)
pub const MESH_LLM_NODE_CERT:   u32 = 32994;  // signed cert, NIP-44'd to member
pub const MESH_LLM_REVOCATION:  u32 = 32995;  // cert/owner revocation (plaintext)
```

`MESH_LLM_LISTING` event structure:

```json
{
  "kind": 32990,
  "pubkey": "<publisher's Sprout pubkey>",
  "tags": [
    ["h", "<channel-id>"],
    ["d", "mesh-llm"],
    ["k", "mesh-llm"],
    ["p", "<member-1-pubkey>"],
    ["p", "<member-2-pubkey>"],
    ["expiration", "<ts>"]
  ],
  "content": "<NIP-44 ciphertext>"
}
```

Plaintext of `content`, after decryption with the recipient's nsec:

```json
{
  "invite_token": "...",
  "mesh_id": "...",
  "owner_id": "...",
  "serving": ["Qwen3-8B-Q4_K_M"],
  "total_vram_bytes": 24000000000,
  "node_count": 1,
  "schema_version": 1
}
```

### Per-recipient encryption pattern

NIP-44 is asymmetric (sender→recipient via ECDH); we can't broadcast one
ciphertext to N recipients. Two options:

- **Option I — N events.** One event per recipient. Publisher signs N
  events, each with one `p` tag and content encrypted to that recipient.
  Simple, scales to ~100 members fine.
- **Option II — 1 event + ephemeral envelope.** Generate an ephemeral
  symmetric key, encrypt the listing payload with it, then NIP-44 the
  symmetric key separately for each recipient inside an envelope
  structure. One event, smaller relay footprint, but adds an envelope
  format we have to invent.

**Pick Option I.** Sprout's existing `observer` module follows this
exact pattern. For the typical channel size, the relay footprint
difference is irrelevant. If a channel grows past ~500 members, revisit.

The discovery filter on the joiner side then becomes:

```json
{
  "kinds": [32990],
  "#h": ["<channel-id>"],
  "#p": ["<my pubkey>"],
  "limit": 100
}
```

Only the events addressed to me come back; relay-side `#p` filtering is
universal Nostr.

---

## 5. mesh-llm CLI surface used

Verbatim, all already exist (verified in
`crates/mesh-llm-host-runtime/src/cli/mod.rs`):

```bash
# Admin: provision channel owner key
mesh-llm auth init --owner-key /path/to/channel-owner.keystore

# Admin: sign a member's node identity
mesh-llm auth sign-node \
  --owner-key /path/to/channel-owner.keystore \
  --node-key  /path/to/member-node.key \
  --out       /path/to/member-node-cert.json \
  --node-label "alice@channel" \
  --expires-in-hours 168

# Server (admin's node):
mesh-llm serve \
  --headless \
  --port 9337 \
  --owner-key /path/to/channel-owner.keystore \
  --trust-policy allowlist \
  --trust-owner  <channel-owner-id> \
  --model Qwen3-8B-Q4_K_M
# (no --publish, no --nostr-relay)

# Joiner (member's node):
mesh-llm serve \
  --headless \
  --join <invite-token> \
  --owner-key /path/to/member.keystore \
  --trust-policy allowlist \
  --trust-owner <channel-owner-id>
```

The only mesh-llm primitive we'd ideally want but don't have today:
**deterministic invite-token output.** mesh-llm prints the token on
startup but in a human-formatted block. Two cheap upstream patches that
would make the integration cleaner:

1. **`--emit-invite-token`** flag that, on startup, writes the token to
   a file path (or fd 3, or `MESH_LLM_INVITE_TOKEN_FILE` env var) on a
   line by itself.
2. **`GET /api/runtime/invite-token`** on the management API
   (already :3131) returning `{ "invite_token": "..." }`.

Neither is required — stdout scrape works today — but both are <100 LOC
and would be welcome upstream. The deep-dive doc has the same suggestion.

---

## 6. `sprout-mesh` crate sketch (revised)

```text
crates/sprout-mesh/
├── Cargo.toml
└── src/
    ├── lib.rs            # MeshHandle::spawn, MeshConfig, top-level orchestration
    ├── owner.rs          # channel owner keypair: generate, distribute via NIP-44
    ├── cert.rs           # node cert issuance (admin side) + acceptance (member side)
    ├── listing.rs        # encrypt + publish + decrypt + discover the listing event
    ├── revoke.rs         # publish / consume revocation events; sync mesh-llm trust store
    ├── token.rs          # invite-token extraction (stdout scrape fallback)
    └── relay.rs          # thin client: POST /events, REQ over WS, subscribe
```

External deps: `nostr` (workspace), `sprout-core` (for kinds + observer
encrypt/decrypt helpers — reuse, don't reimplement), `tokio`, `reqwest`,
`serde`, `tracing`. Optionally `mesh-api` (path dep) for in-process
owner-key generation; can also shell out to `mesh-llm auth init` if we'd
rather not pull mesh-llm into our build graph.

**Operator-facing config:**

```env
SPROUT_MESH_ENABLED=true
SPROUT_MESH_BIN=mesh-llm
SPROUT_MESH_PORT=9337
SPROUT_MESH_CONSOLE_PORT=3131
SPROUT_MESH_MODELS=Qwen3-8B-Q4_K_M
SPROUT_MESH_CHANNEL=<channel-id>          # which channel this mesh belongs to
SPROUT_MESH_ROLE=auto                     # auto | admin | member | client
SPROUT_MESH_RUNTIME_ROOT=~/.sprout/mesh/<workspace>/<channel>/
SPROUT_MESH_OWNER_KEYSTORE=$SPROUT_MESH_RUNTIME_ROOT/owner.keystore
SPROUT_MESH_NODE_KEYSTORE=$SPROUT_MESH_RUNTIME_ROOT/node.key
SPROUT_MESH_TRUST_POLICY=allowlist        # never overridable to "off" in private mode
```

Note the **per-(workspace, channel)** runtime root: lets one Sprout
client participate in meshes for multiple channels without trust-store
collisions.

---

## 7. State machines

### 7.1 Admin bootstrap

```
[no owner key for this channel]
  │
  │  admin runs "create channel mesh"
  ▼
[generate OwnerKeypair locally]
  │
  ├──→ publish kind:32991 (MESH_LLM_OWNER)         ── plaintext, h-scoped
  └──→ publish kind:32992 (MESH_LLM_OWNER_SHARE)   ── one per other admin, p-tagged
       NIP-44(other-admin, {owner secret bytes})
  │
  ▼
[boot mesh-llm serve --owner-key … --trust-policy allowlist
                     --trust-owner <self-owner-id>]
  │
  ▼
[mesh-llm prints invite-token → sprout-mesh captures]
  │
  ▼
[for each current member M ≠ self in kind:39002]:
  │  if M has no cert (no kind:32994 from us):
  │    issue cert via cert.rs (sign M's posted node pubkey)
  │
  ▼
[publish kind:32990 (MESH_LLM_LISTING)]
  │  for each M in members:
  │    sign one event, p-tagged to M, NIP-44(M, listing)
  │
  ▼
[publish_loop every 60s; refresh listing, watch for new members & revocations]
```

### 7.2 Member bootstrap

```
[member sees they're in kind:39002 for channel C]
  │
  ▼
[generate node keypair if absent; publish kind:32993 (MESH_LLM_NODE_KEY)]
  │  plaintext, h-scoped, content = {node_endpoint_id, node_public_key}
  │
  ▼
[wait for kind:32994 (MESH_LLM_NODE_CERT) p-tagged to me]
  │  decrypt with my nsec → SignedNodeOwnership
  │  drop file at <runtime-root>/node-ownership.json
  │
  ▼
[REQ for kind:32990, #h:[C], #p:[me]]
  │  decrypt content with my nsec → {invite_token, owner_id, ...}
  │
  ▼
[boot mesh-llm serve --join <token> --owner-key <derive> 
                     --trust-policy allowlist --trust-owner <owner_id>]
  │
  ▼
[sprout-agent: OPENAI_COMPAT_BASE_URL=http://localhost:9337/v1]
```

### 7.3 Eviction

```
[admin sees member M removed from kind:39002 (NIP-29 membership change)]
  │
  ▼
[publish kind:32995 (MESH_LLM_REVOCATION)]
  │  plaintext, h-scoped: { cert_id, node_endpoint_id, reason }
  │
  ▼
[admin's mesh-llm: trust-store revoke; existing M sessions drop on next handshake]
  │
  ▼
[next listing republish: omit M from p-tag set → M can't decrypt new tokens]
  │
  ▼
[optional: admin rotates owner key entirely if M was hostile — re-bootstrap]
```

---

## 8. Sprout's existing infrastructure that pays for itself here

| Need | Existing Sprout primitive |
|---|---|
| Channel membership ACL | NIP-29 (`kind:39002`, relay-enforced) |
| Channel admin set | NIP-29 (`kind:39001`) |
| NIP-44 v2 encrypt/decrypt | `sprout-core::observer::{encrypt_observer_payload, decrypt_observer_payload}` — reuse verbatim |
| Replaceable events | already supported by `sprout-relay/sprout-db` |
| `h`-tag scoping | already enforced everywhere |
| Event WS + `POST /events` + `POST /query` | the dual API surface |
| Per-relay/workspace lifecycle on desktop | `resetWorkspaceState()` in `useWorkspaceInit.ts` |
| Subprocess supervision pattern | `sprout-acp::AcpClient::spawn` (process groups, idle timeout, respawn) |

The *only* truly new thing is the five event kinds (32990–32995) and
the small state machines in §7. Everything else composes existing
pieces.

---

## 9. What we lose by being private

- ❌ **No `--auto` join.** Meshes are not discoverable; you must be in
  the channel. This is the point.
- ❌ **No cross-relay roaming.** A mesh is bound to one Sprout relay
  (well — to a channel on that relay). If a member switches relays,
  they need a new mesh provisioned on the new relay.
- ❌ **No mesh-llm-native `mesh-llm discover` listing.** The CLI's
  discover command queries the public default relays; it won't see
  ours. Users will use Sprout's own UI to see "Channel mesh: 2 nodes,
  Qwen3-8B".
- ❌ **No public-mesh capacity boost.** Members can only use compute
  contributed by other channel members; can't ride on a strangers' GPU.
  Add an opt-in `SPROUT_MESH_ALSO_JOIN_PUBLIC=true` later if anyone wants
  to layer both.

Net trade: discoverability ↓, control ↑. Matches the user's brief.

---

## 10. Recommended rollout

A revised, three-stage plan that replaces Stages 2a/2b from the
previous doc:

### Stage 1 — `sprout-mesh` skeleton + Option A internal test (1 PR)

- New crate `crates/sprout-mesh` with `MeshHandle::spawn`,
  stdout-scrape `token.rs`.
- `sprout-acp` integration: feature-flagged, injects
  `OPENAI_COMPAT_BASE_URL` after readiness probe.
- Boots mesh-llm in **private-no-discovery** mode for a single user
  (admin-only), no listing publication yet. Proves the local plumbing
  end-to-end. Useful on its own: any Sprout user with mesh-llm
  installed gets a local OpenAI endpoint backing their agent.

### Stage 2 — Owner key + listing publish/discover (1 PR)

- Define event kinds 32990–32995 in `sprout-core::kind`.
- Implement `owner.rs` (generate + share), `listing.rs` (publish-loop
  + per-member encrypt + discover-loop + decrypt).
- Wire `sprout-mesh` boot mode: `admin` publishes, `member` discovers
  and `--join`s.
- Skip cert issuance for now: run mesh-llm with `--trust-policy off`
  inside this PR. Token gates access; encryption gates token. Good
  enough to validate the flow.

### Stage 3 — Owner-attested trust (1 PR)

- Implement `cert.rs` (admin signs, member receives).
- Flip mesh-llm to `--trust-policy allowlist --trust-owner <id>` by
  default in private mode.
- Implement `revoke.rs` and wire it to NIP-29 membership-change
  events.
- This is the PR that delivers the actual cryptographic guarantee.

### Stage 4 (optional, later) — Polish

- Upstream PRs to mesh-llm: deterministic invite-token output,
  `/api/runtime/invite-token`.
- Desktop UI: "Channel mesh" panel showing serving models, node
  count, your role (admin/member), buttons to rotate owner or revoke
  a member.
- Metrics: expose mesh-llm's `/api/status` proxied through Sprout for
  ops visibility.

---

## 11. Open questions

1. **Owner-share key custody for solo admins.** If a channel has a
   single admin and they lose their device, the channel mesh owner key
   is gone forever and the mesh must be re-bootstrapped. Should we
   require ≥2 admins before allowing mesh creation, or accept that
   single-admin channels accept this risk? (Suggest: warn but allow.)

2. **Quorum issuance vs. any-admin issuance.** Any admin can sign a
   member's node cert in this design. We could require k-of-n by
   wrapping the owner secret in Shamir shares, but that's a big
   complexity step for an unclear threat model. (Suggest: defer.)

3. **mesh-llm runtime root separation.** mesh-llm assumes
   `~/.mesh-llm/` is the global root. We need to override with
   `MESH_LLM_RUNTIME_ROOT` per channel. Confirm that env var is
   respected for *all* state (trust store, node key, last-mesh-id, owner
   keystore) — initial grep says yes but worth a one-shot test.

4. **NIP-29 admin signal latency.** NIP-29 admin/member lists are
   replaceable events. There's a window where an evicted member still
   has a valid invite token cached. The revocation event closes this
   for new connections but not in-flight QUIC sessions. Is that
   acceptable, or do we need an explicit mesh-side kick? mesh-llm's
   trust store consultation is per-connect; existing connections survive
   until next handshake (~minutes). Likely fine; document.

5. **GPU-less members.** Most channel members won't have GPUs. They
   should run `mesh-llm client --join` instead of `serve --join`. The
   distinction matters because clients don't need a node ownership
   cert (verify this — control-plane auth applies to clients too in
   `RequireOwned`, but may not in `Allowlist`; needs a quick read of
   `ownership.rs:506`). If clients also need certs, that's the same
   flow; if not, simpler.

6. **First member to arrive in an empty channel.** Bootstrap requires
   *someone* to be the admin who runs `mesh-llm`. If no admin is online,
   non-admin members can't get certs issued. Solution: certs are
   long-lived (default 168h = 1 week in `auth sign-node`) and replayable
   from the relay's event store — once issued, the encrypted
   `kind:32994` event sits there until the member fetches it. So admins
   need to be online *at least once after a new member joins*. Document.

---

## 12. Summary

This design uses mesh-llm exactly as it ships: private mesh
(`--owner-key … --trust-policy allowlist --trust-owner …`), no
public discovery (`--publish` and `--nostr-relay` both omitted).
Sprout owns rendezvous via five new event kinds, all `h`-tag scoped to
a NIP-29 channel, with the invite token + node cert NIP-44-encrypted to
the current member set. The cryptographic root is **per-channel** —
one mesh owner key per channel, custodied by the channel's admin set,
distributed via standard NIP-44 envelopes that Sprout's `observer`
module already handles.

End result: when two members of the same Sprout channel are online with
`SPROUT_MESH_ENABLED=true`, they end up on the same mesh-llm mesh
within ~60s, sharing GPU capacity, without ever touching a public
Nostr relay or accepting a join request from anyone outside the
channel. Stolen invite tokens are useless without a signed cert; lost
member access is closed by NIP-29 eviction + revocation event. A
malicious relay operator sees encrypted bytes and a count of how many
mesh participants there are, nothing more.

---

## Appendix — file index (delta vs. previous docs)

**Sprout (new in this design):**
- `crates/sprout-core/src/kind.rs` — add `MESH_LLM_LISTING` (32990),
  `MESH_LLM_OWNER` (32991), `MESH_LLM_OWNER_SHARE` (32992),
  `MESH_LLM_NODE_KEY` (32993), `MESH_LLM_NODE_CERT` (32994),
  `MESH_LLM_REVOCATION` (32995).
- `crates/sprout-mesh/` — new crate (see §6).
- Reuses `crates/sprout-core/src/observer.rs::{encrypt_observer_payload,
  decrypt_observer_payload}` unchanged.

**mesh-llm (referenced, no patches required for v1):**
- `crates/mesh-llm-host-runtime/src/crypto/ownership.rs:14-100` —
  `TrustPolicy`, `NodeOwnershipClaim`, `SignedNodeOwnership`, `TrustStore`.
- `crates/mesh-llm-host-runtime/src/crypto/ownership.rs:473` — allowlist
  enforcement at control-plane handshake.
- `crates/mesh-llm-host-runtime/src/cli/mod.rs:41+` — `AuthCommand`
  surface (`Init`, `SignNode`, `RenewNode`, `RevokeNode`, `RevokeOwner`,
  trust store mgmt).
- `crates/mesh-api/src/identity.rs::OwnerKeypair` — embeddable
  generation if we want it in-process.

**Optional upstream mesh-llm PRs (nice-to-have, not blocking):**
- Deterministic invite-token output via flag or env-var file path.
- `GET /api/runtime/invite-token` on management API.
