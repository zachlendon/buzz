# Buzz Attack Plan

1. Build a two-tenant collision harness.

   Run one relay process with two host-mapped communities, then create deliberately colliding objects in both tenants: the same pubkey, channel UUID, workflow UUID, media SHA-256, and git owner/repo path. Exercise `/events`, `/query`, `/count`, `/hooks/{id}`, media upload/read paths, and git clone/push/fetch through both hosts. The first goal is to prove that the host-derived community boundary survives collisions and that A never reads, writes, or confirms B's state.

2. Attack the authentication and host-binding seams.

   Reuse NIP-42, NIP-98, Blossom, and git auth events across hosts and endpoints. Mutate signed URL, method, payload hash, timestamp, `server` tag, `h` tag, and relay membership inputs, and try unmapped hosts and conflicting client-supplied tenant hints. Look for any path where caller-controlled data overrides the request host, where a token minted for A works on B, or where rejection bodies and status codes become a tenant-existence oracle.

3. Chase derived-state and asynchronous leakage.

   After the primary row boundaries hold, test the systems that copy, cache, fan out, or materialize data: Redis pub/sub, presence and typing, search, thread counters, notifications, workflow runs and approvals, audit records, and background execution sinks. These are the places where a correctly scoped database write can still leak through a global cache key, an unscoped job lookup, or a fan-out topic that omits `community_id`.

4. Push parser, race, and resource edges.

   Stress broad REQ/COUNT/search filters, oversized WebSocket frames, malformed workflow YAML and conditions, webhook bodies, media type sniffing and range requests, and git pack/hook/CAS behavior under concurrent pushes. The objective is to find denial-of-service paths, expensive work performed before authorization, orphaned or partially committed state, and race windows where a rejected operation still changes tenant-visible state.
