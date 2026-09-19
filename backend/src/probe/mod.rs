//! GIS probe engine (task 2.3).
//!
//! [`client`] is the HTTP transport (introduced in task 2.1 for the on-demand
//! `/test` endpoint), [`gis`] adds service-type-specific checks
//! (GetCapabilities for WMS/WFS/WMTS, `f=pjson` for ArcGIS REST, OAF landing
//! page), and [`evaluate`] implements the per-target alert state machine.

pub mod client;
pub mod evaluate;
pub mod gis;
