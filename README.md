# GIS Sentinel

Proof of concept for gis monitoring application build with sveltekit and rust.

## Frontend

Sveltekit application with static adapter that hosts the web interface and connects trough websockets to the backend for status updates and alerts.

## Backend

Rust rest api build with axum. The frontend application is included in the binary when the backend is build.

### Structure

Backend application is divided in multiple services

1. Main webserver with websocket and ui
2. API service for crud Alert points
3. Scheduler service which fills the queue with alert jobs
4. Worker service which executes jobs from the queue and passes back the results to the main webserver

Then there are some other required applications for storing the queue and all monitoring alerts configured.

- Database is Postgres
- Queue is valkey/redis
