-- Initial GIS Sentinel schema: monitoring targets, probe history, active alerts.

CREATE TABLE alert_points (
    -- App-generated UUID v7 on normal inserts; the default keeps ad-hoc/SQL inserts working.
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name          TEXT NOT NULL,
    url           TEXT NOT NULL,
    service_type  TEXT NOT NULL CHECK (service_type IN ('WMS', 'WFS', 'WMTS', 'OAF', 'ArcGIS_REST', 'HTTP')),
    check_interval_seconds     INT  NOT NULL CHECK (check_interval_seconds > 0),
    expected_response_time_ms  INT  NOT NULL CHECK (expected_response_time_ms > 0),
    http_method   TEXT NOT NULL DEFAULT 'GET' CHECK (http_method IN ('GET', 'POST')),
    custom_headers JSONB NOT NULL DEFAULT '{}'::jsonb,
    auth_config   JSONB,
    enabled       BOOLEAN NOT NULL DEFAULT TRUE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_alert_points_enabled ON alert_points (enabled) WHERE enabled;

CREATE TABLE probe_results (
    id             BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    alert_point_id UUID NOT NULL REFERENCES alert_points (id) ON DELETE CASCADE,
    timestamp      TIMESTAMPTZ NOT NULL DEFAULT now(),
    response_time_ms INT,
    status_code    INT,
    is_up          BOOLEAN NOT NULL,
    error_message  TEXT,
    raw_response_snippet TEXT
);

CREATE INDEX idx_probe_results_point_time ON probe_results (alert_point_id, timestamp DESC);

CREATE TABLE active_alerts (
    id             BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    alert_point_id UUID NOT NULL REFERENCES alert_points (id) ON DELETE CASCADE,
    alert_type     TEXT NOT NULL CHECK (alert_type IN ('New', 'Update', 'Remove')),
    reason         TEXT NOT NULL,
    triggered_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at    TIMESTAMPTZ
);

-- At most one open (unresolved) alert per alert point.
CREATE UNIQUE INDEX uq_active_alerts_point ON active_alerts (alert_point_id) WHERE resolved_at IS NULL;

-- Keep alert_points.updated_at current automatically.
CREATE OR REPLACE FUNCTION set_updated_at() RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_alert_points_updated_at
    BEFORE UPDATE ON alert_points
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
