-- Real-time alert hub (task 2.4): persist the evaluated health status of an
-- open alert so WebSocket snapshot-on-connect can report the true current
-- state (Healthy is never stored — open alerts are by definition not healthy).

ALTER TABLE active_alerts
    ADD COLUMN status TEXT NOT NULL DEFAULT 'DOWN'
    CHECK (status IN ('HEALTHY', 'DEGRADED', 'DOWN'));

-- Existing open alerts were all outage-style alerts; 'DOWN' is the safe
-- backfill (the DEFAULT above covers rows present at this point).
