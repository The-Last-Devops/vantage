-- History of alert state transitions (fired / recovered) for the alerts UI.
CREATE TABLE alert_events (
    id       BIGSERIAL PRIMARY KEY,
    alert_id UUID NOT NULL REFERENCES alerts(id) ON DELETE CASCADE,
    at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    firing   BOOLEAN NOT NULL,
    message  TEXT
);
CREATE INDEX idx_alert_events_at ON alert_events (at DESC);
CREATE INDEX idx_alert_events_alert ON alert_events (alert_id, at DESC);
