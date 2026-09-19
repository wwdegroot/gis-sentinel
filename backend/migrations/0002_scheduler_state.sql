-- Scheduler state (task 2.2): track when each alert point was last enqueued
-- for probing so the scheduler only selects due targets.

ALTER TABLE alert_points
    ADD COLUMN last_checked_at TIMESTAMPTZ;

-- Partial index: only enabled rows are ever scheduled, and due-ness is
-- decided by last_checked_at.
CREATE INDEX idx_alert_points_due
    ON alert_points (last_checked_at)
    WHERE enabled;
