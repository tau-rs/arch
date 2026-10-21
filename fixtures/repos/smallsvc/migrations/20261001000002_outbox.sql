-- a shared table used as a queue: app::pay / app::ship insert, worker dequeues (FOR UPDATE SKIP LOCKED)
CREATE TABLE outbox (
    id        BIGSERIAL PRIMARY KEY,
    kind      TEXT NOT NULL,
    order_id  UUID NOT NULL REFERENCES orders(id),
    payload   JSONB NOT NULL,
    attempts  INT NOT NULL DEFAULT 0,
    locked_at TIMESTAMPTZ
);

CREATE INDEX outbox_unlocked_idx ON outbox (id) WHERE locked_at IS NULL;
