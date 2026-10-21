-- orders, their lines, payments and shipments
CREATE TABLE orders (
    id             UUID PRIMARY KEY,
    customer_email TEXT NOT NULL,
    status         TEXT NOT NULL,
    placed_at      TIMESTAMPTZ NOT NULL
);

CREATE TABLE order_lines (
    order_id         UUID NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    position         INT  NOT NULL,
    sku              TEXT NOT NULL,
    quantity         INT  NOT NULL,
    unit_price_cents BIGINT NOT NULL,
    currency         TEXT NOT NULL,
    PRIMARY KEY (order_id, position)
);

CREATE TABLE payments (
    id           UUID PRIMARY KEY,
    order_id     UUID NOT NULL REFERENCES orders(id),
    amount_cents BIGINT NOT NULL,
    currency     TEXT NOT NULL,
    status       TEXT NOT NULL,
    provider_ref TEXT,
    created_at   TIMESTAMPTZ NOT NULL
);

CREATE TABLE shipments (
    id        UUID PRIMARY KEY,
    order_id  UUID NOT NULL REFERENCES orders(id),
    carrier   TEXT NOT NULL,
    tracking  TEXT NOT NULL,
    status    TEXT NOT NULL,
    booked_at TIMESTAMPTZ NOT NULL
);
