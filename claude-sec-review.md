# Buzz Security Architecture Review

_Architecture- and design-level review of the general approach — not a point-in-time
vulnerability scan. The question this answers: are we building Buzz securely, and is
the protocol robust?_

## Verdict up front

**Buzz is being built securely, and the protocol core is sound.** The identity model,
the tenant boundary, and the data layer reflect genuine security engineering —
mandatory signature verification, a fail-closed host-derived tenancy chokepoint, fully
parameterized SQL, formal models, and code that shows real adversarial thinking. This is
well above the norm for a Nostr relay.

The systemic risk is not in the cryptography or the protocol design. It is a **recurring
pattern: rigorous authentication and verification at the edges, but weaker enforcement
and confinement behind them.** Trust that should be structural is often implicit;
invariants that should be enforced by construction are enforced by convention; and one
subsystem (agents) puts a cryptographically strong front door on a completely unconfined
room. Two stated goals — audit tamper-evidence and formal-methods assurance — are not
actually delivered by the implementation.

---

## What's genuinely strong

- **Identity & event verification.** `verify_event` recomputes the NIP-01 event ID *and*
  checks the Schnorr signature, and it is mandatory on every external ingestion door (WS,
  `POST /events`, ephemeral, observer frames, mesh). Because the ID hash covers the tags,
  the `h`/`p`/`d`/`e` tags that drive authorization are inside the signed digest — so
  "trust after verify" is sound and tags cannot be forged to escalate. No hand-rolled
  crypto; it delegates to rust-nostr.
- **Authentication has no bearer-secret surface.** Every request reduces to a
  Schnorr-verified pubkey. No sessions, JWTs, or server-issued tokens to steal or replay.
  NIP-42 challenges are 32 bytes of CSPRNG, connection-scoped, single-attempt, and bound
  to challenge + relay URL + ±60s timestamp. The signed URL is checked against the
  *resolved tenant host*, not a global config value — closing cross-community token replay
  in both directions.
- **The multi-tenant boundary is the best-engineered part of the system.** One
  `bind_community()` chokepoint derives the community from the connection `Host`, fails
  closed on unmapped/empty/error (no default tenant, no host echo to prevent probing), and
  client-supplied tags can *narrow* but never *override* it. It has red-team tests and a
  named formal invariant (`Inv_RowZero`). Every HTTP path re-binds identically.
- **Data layer.** Exhaustively parameterized SQL (the only dynamic fragments are
  hardcoded column names); `community_id` leads every key, index, and query predicate; a
  build-failing migration lint enforces that schema shape; bounded query limits.
- **Media / git / pairing subsystems are individually well-built.** Media never trusts
  client Content-Type (magic-byte sniffing + image-bomb pre-checks + `nosniff`/attachment
  /CSP on serve). Git push authz is server-side and structurally non-bypassable (the
  pre-receive hook runs inside the relay's own `receive-pack`, HMAC-bound, fail-closed)
  with thorough command-injection/traversal defense. Device pairing is transport-untrusting:
  SAS bound to the ECDH secret, dual consent, ephemeral keys, formally modeled.
- **Client key handling.** On desktop the nsec stays in the Rust backend; signing happens
  behind the Tauri IPC boundary; keys go to the OS keyring; nothing secret is logged;
  markdown rendering uses no `rehype-raw` and blocks `javascript:`/`data:` URLs.

---

## Cross-cutting weaknesses, ranked

### 1. The agent execution surface: strong authorization, absent isolation

Highest real-world risk, and the component the codebase itself calls "the most dangerous
part." The **authorization** layer is genuinely good — crypto-verified NIP-OA owner
gating, `OwnerOnly` default that fails closed, siblings re-verified by signature. If the
only threat were "an unauthorized user tasks my agent," the defense holds.

But once a turn is admitted, the **execution** layer has the classic prompt-injection
lethal trifecta with no mitigations:

- **Untrusted content reaches the model with no boundary** — message content and thread
  context are concatenated verbatim into the prompt (`queue.rs:1037`), and the base prompt
  actively tells the agent to read channels, feeds, and repos (all attacker-influenceable).
- **The tools are unconfined** — `buzz-dev-mcp` runs `bash -c` directly on the host with
  no sandbox, no egress control, no path jail (`shell.rs:150`; `paths.rs` explicitly
  documents "no containment").
- **The identity key is in-scope and exfiltratable** — `BUZZ_PRIVATE_KEY` is deliberately
  inherited into that shell (`shell.rs:154`), and permissions are auto-approved
  (`bypass-permissions` default + unconditional `allow_once` at `acp.rs:1314`).

Prompt injection converts "the agent read a message" into "arbitrary code ran on the host
and the relay identity key was stolen." Telling asymmetry: the git signing key is
carefully moved to a `0600` keyfile and zeroized, while the relay identity key is passed
in plaintext straight into an arbitrary shell. The model rests on the LLM's judgment as
the only control.

**Fixes (architectural):** real sandboxing (extend the Codex-style Seatbelt confinement to
all runtimes), broker relay calls through a mediating process so the raw key never enters
the shell, and a real per-tool permission policy instead of blanket auto-approve.

### 2. The internal trust boundary is implicit and unverified

The system verifies every event at the edge, then trusts everything behind it:

- **Redis pub/sub events are not re-verified.** The receiving pod deserializes and fans
  out events with no signature check (`subscriber.rs:148`), and the community label comes
  from the *Redis channel name*, not the event. Anyone with Redis write access (or a
  compromised pod) can inject arbitrary unsigned events into any community's fan-out,
  delivered to clients as relay-endorsed, gated only by access filtering.
- **DB rehydration marks events `verified=true` without recomputing**, and the `verified`
  bit is an advisory convention, not a type-enforced invariant — nothing stops a future
  handler from persisting/fanning-out an unverified event.

Defensible for a single-operator deployment, but **undocumented and load-bearing**, and
it is the one place a signature-free event reaches end users. Re-verifying on the bus is
cheap (`verify_event` is already `spawn_blocking`-ready), and a `Verified<Event>`
type-state would make the invariant structural. **Highest-leverage cheap fix in the review.**

### 3. The audit log does not meet its stated goal

`SECURITY.md` claims "SOX-grade compliance and eDiscovery," but the implementation is a
provenance log, not a tamper-evident one:

- **The hash chain is self-certifying** — unkeyed SHA-256, no signed head, no external
  anchoring/notarization. An operator or attacker with DB write access recomputes the whole
  chain and `verify_chain` reports it as pristine. That is precisely the adversary an audit
  log exists to catch.
- **No append-only enforcement** — plain table, no restricted role or trigger; immutability
  is convention.
- **Verification is never run** — no operator/admin entry point; only tests call
  `verify_chain`.
- **Only 2 of 11 defined actions are emitted** — deletions, membership changes, and auth
  success/failure are defined in the enum but never logged.

`SECURITY.md` *does* honestly state the chain is "tamper-evident but not tamper-resistant."
The gap is that even that weaker claim is not operationalized (nothing verifies it), and
the compliance framing oversells it. **Fixes:** anchor the head under a key outside the DB
trust domain (or add an append-only DB role), emit the full action set, and give operators
a verify command.

### 4. Abuse/DoS resistance is underbuilt

Robust on integrity and confidentiality but weak on availability:

- **The per-identity/per-IP rate limiter is fully implemented and never wired in.**
  `buzz-auth::rate_limit` and `RedisRateLimiter` exist, but `AppState` has no rate-limiter
  field and nothing in the relay constructs or calls it. Actual protection is global
  semaphores — meaning **no per-pubkey flood control and no per-tenant fairness** (one
  abusive authenticated client competes with every tenant for the same permit pool). The
  connection-flood IP limiter is likewise defined but unused.
- **Query cost is bounded on row count but not on fan-out** — `#e`-tag filters expand to
  an uncapped OR of GIN probes, there is no ceiling on filters-per-REQ, and the non-keyset
  path uses O(offset) pagination.
- **No durable media storage quota** (self-flagged `TODO(v2)`).

Wiring the existing limiter into the hot path is low-effort and the single biggest
availability win.

### 5. Enforcement by convention rather than by construction

The recurring meta-weakness behind several findings:

- **Read-side authz is re-implemented across ~5 endpoints.** The write path is centrally
  gated in `ingest_event`; the read side calls the same helper functions from `req.rs`,
  `/query` (five sub-paths), `/count`, etc. Nothing forces a new read endpoint to apply all
  four gates.
- **Tenant isolation in the DB is app-layer discipline with no Postgres RLS backstop** — a
  single query that forgets `WHERE community_id = $1` leaks cross-tenant, with nothing below
  the app to catch it.
- **The p-gate is a per-kind denylist** — a new sensitive kind that is not classified is
  world-readable within the community by default.
- **Audit append-only, the `verified` bit** — same shape.

The mitigations (migration lint, tests, conformance tracers) are good but they enforce
*shape*, not the *predicate*. The durable fix is to make invariants structural: RLS as
defense-in-depth, one traced read chokepoint, type-states, allowlist-by-default
classification.

### 6. Transport & client hardening is deferred to deployment

- The relay does not enforce TLS (intentional, for proxy termination), clients accept
  `ws://` with no cert pinning. Because NIP-42 binds auth to a *connection* while individual
  reads ride that connection, a plaintext MITM can hijack an authenticated session and read
  private channels — confidentiality rests entirely on the operator configuring TLS.
- **Desktop ships `csp: null`** alongside a live-but-unused `get_nsec` IPC command. Today
  the markdown-escaping discipline is the only thing between an XSS and full key
  exfiltration — one `rehype-raw` regression and the key is gone. The web client (which
  renders untrusted repo content) ships no CSP or framing headers at all.

---

## Protocol robustness

The protocol *design* — event-sourced Nostr/NIP-29, signature-anchored identity,
host-derived tenancy — is coherent and sound. Three robustness (correctness, not just
security) snags surfaced:

- **Rejected git pushes still advance the manifest pointer** (`formal.md`, confirmed):
  because `parent` is folded into the manifest bytes, even a policy-denied or no-op push
  changes the digest and wins the CAS, letting a rejected pusher grief concurrent legitimate
  writers. This is exactly the `SkipPublish` branch the TLA+ model proves and the code does
  not implement.
- **Cached-membership stale-positive window**: a member removed from a private channel can
  still read/write for the cache TTL (~10s) on a pod that has not seen the invalidation. The
  stale-*negative* direction is handled; the revocation direction is not symmetric.
- **Ephemeral events have no timestamp-freshness check** (unlike durable ±15min and
  observer ±5min), so an old validly-signed presence/typing event can be replayed and
  re-fanned-out. Low impact, but an inconsistency.

**On assurance:** the formal models do not match the shipped code in several documented
places (the git pointer issue above; NIP-AB plaintext exposed before dual consent in both
the mobile and Rust targets; the Tamarin buffering rule binds plaintext before approval;
`Inv_Closed` does not actually prove the reconstruction theorem). Having `formal.md` write
this down is excellent practice — the risk is that a green "proven" label creates
confidence the implementation has not earned. Treat the models as design intent to be
reconciled with code, not as current guarantees.

---

## If prioritizing

1. **Sandbox the agent shell and get `BUZZ_PRIVATE_KEY` out of it** (#1) — highest
   real-world impact.
2. **Re-verify events on the Redis bus, or document the trust assumption explicitly** (#2)
   — cheapest high-leverage fix.
3. **Wire in the rate limiter that's already written** (#4) — low effort, closes the
   biggest availability gap.
4. **Make the audit log meet its claim or soften the claim** (#3) — anchor the head, emit
   all actions, add a verify command.
5. **Add RLS + a single read-authz chokepoint** (#5) and **a strict desktop CSP + remove
   `get_nsec`** (#6) as defense-in-depth.

---

## Scope & method

Reviewed six trust domains in parallel against the source tree: event
cryptography/verification (`buzz-core`), auth & multi-tenant boundary (`buzz-auth`,
`buzz-relay` tenant/auth), the AI agent execution surface (`buzz-acp`, `buzz-agent`,
`buzz-dev-mcp`, `buzz-workflow`), the data layer (`buzz-db`, `buzz-pubsub`,
`buzz-search`), media/git/pairing/audit (`buzz-media`, `git-*`, `buzz-pair-relay`,
`buzz-audit`), and client key management (`desktop`, `mobile`, `web`, `buzz-ws-client`).
Findings are architectural — code-line references (e.g. `shell.rs:154`) are anchors for
the design pattern, not an exhaustive bug list.
