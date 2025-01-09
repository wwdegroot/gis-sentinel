# GIS Sentinel
Proof of concept for gis monitoring application build with sveltekit and rust.

## Frontend
Sveltekit application with static adapter that hosts the web interface and connects trough websockets to the backend for status updates and alerts.

## Backend
Rust rest api build with axum. The frontend application is included in the binary when the backend is build.
