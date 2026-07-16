-- Bound the durable push matcher independently for each community. Matching is
-- enabled explicitly by the relay process after configuration is loaded, so a
-- disabled deployment does not keep accumulating work from the events trigger.
CREATE TABLE push_match_runtime_state (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    enabled BOOLEAN NOT NULL DEFAULT FALSE
);
INSERT INTO push_match_runtime_state (singleton, enabled) VALUES (TRUE, FALSE);

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('push_match_runtime_state', 'deployment push matcher switch')
ON CONFLICT (table_name) DO UPDATE SET reason = EXCLUDED.reason;

CREATE TABLE push_match_community_state (
    community_id UUID PRIMARY KEY REFERENCES communities(id),
    queued_jobs BIGINT NOT NULL DEFAULT 0 CHECK (queued_jobs >= 0),
    max_queued_jobs BIGINT NOT NULL DEFAULT 10000 CHECK (max_queued_jobs > 0),
    dropped_jobs BIGINT NOT NULL DEFAULT 0 CHECK (dropped_jobs >= 0),
    last_claimed_at TIMESTAMPTZ NOT NULL DEFAULT '-infinity'
);

INSERT INTO push_match_community_state (community_id, queued_jobs)
SELECT community_id, count(*)
FROM push_match_queue
GROUP BY community_id;

CREATE INDEX push_match_queue_community_due
    ON push_match_queue (community_id, next_attempt_at, created_at);
CREATE INDEX push_leases_match_admission
    ON push_leases (community_id)
    WHERE active AND endpoint_enabled;

-- Every queue insertion passes through this trigger, including future internal
-- producers. Returning NULL drops only the push job; the accepted source event
-- remains durable. The per-community row lock makes the cap race-safe without
-- serializing unrelated communities.
CREATE FUNCTION admit_push_match_job() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM push_match_queue
        WHERE community_id = NEW.community_id AND event_id = NEW.event_id
    ) THEN
        RETURN NULL;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM push_match_runtime_state
        WHERE singleton AND enabled
    ) THEN
        RETURN NULL;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM push_leases
        WHERE community_id = NEW.community_id
          AND active
          AND endpoint_enabled
          AND expires_at > EXTRACT(EPOCH FROM now())::bigint
    ) THEN
        RETURN NULL;
    END IF;

    INSERT INTO push_match_community_state (community_id)
    VALUES (NEW.community_id)
    ON CONFLICT (community_id) DO NOTHING;

    UPDATE push_match_community_state
    SET queued_jobs = queued_jobs + 1
    WHERE community_id = NEW.community_id
      AND queued_jobs < max_queued_jobs;

    IF NOT FOUND THEN
        UPDATE push_match_community_state
        SET dropped_jobs = dropped_jobs + 1
        WHERE community_id = NEW.community_id;
        RETURN NULL;
    END IF;

    RETURN NEW;
END
$$;

CREATE TRIGGER push_match_queue_admission
BEFORE INSERT ON push_match_queue
FOR EACH ROW EXECUTE FUNCTION admit_push_match_job();

-- Keep queue accounting exact for completion, poison-job cleanup, source-event
-- deletion, and disabled-mode draining. A statement trigger avoids one counter
-- update per row when a backlog is removed in a batch.
CREATE FUNCTION account_push_match_deletes() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE push_match_community_state state
    SET queued_jobs = GREATEST(0, state.queued_jobs - deleted.count)
    FROM (
        SELECT community_id, count(*) AS count
        FROM deleted_push_match_jobs
        GROUP BY community_id
    ) deleted
    WHERE state.community_id = deleted.community_id;
    RETURN NULL;
END
$$;

CREATE TRIGGER push_match_queue_delete_accounting
AFTER DELETE ON push_match_queue
REFERENCING OLD TABLE AS deleted_push_match_jobs
FOR EACH STATEMENT EXECUTE FUNCTION account_push_match_deletes();
