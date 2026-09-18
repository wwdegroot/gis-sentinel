//! GIS probe engine (task 2.3).
//!
//! Phase 2.1 ships only the HTTP transport layer ([`client`]) so the
//! `POST /api/v1/alert-points/:id/test` endpoint can run on-demand probes.
//! Service-type-specific checks (GetCapabilities parsing for WMS/WFS/WMTS,
//! `f=pjson` for ArcGIS REST, OAF landing page) and alert state evaluation
//! are added in task 2.3 as `gis.rs` / `evaluate.rs`.

pub mod client;
