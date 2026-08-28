## Database
- A shared PostgreSQL instance runs as the Docker container `postgres-gs`, reachable at `postgres-gs:5432` from this container.
- Do NOT install, start, or assume a local postgres. Use `DATABASE_URL` from `backend/.env`.
- `SQLX_OFFLINE=true` is used at build time; for live queries use `sqlx migrate` / `sqlx query` against the shared instance.
