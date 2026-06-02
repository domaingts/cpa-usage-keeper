# Rust Rewrite of CPA Usage Keeper

## Context

`cpa-usage-keeper` is a Go service (~25.7k LOC across 166 files / 28 packages) that consumes a CPA Redis usage queue into SQLite, polls CPA management endpoints for metadata, aggregates usage/pricing/quota data, and serves a React dashboard plus REST APIs. The user wants a full rewrite in Rust as a **drop-in replacement**: same SQLite schema, same HTTP routes/payloads, same env vars, same Docker layout. The React frontend stays as-is and is served by the new Rust binary.

The `src/` tree already has empty directory scaffolding mirroring `internal/` (28 dirs) but only 3 stub `.rs` files (54 lines total) and a near-empty `Cargo.toml`. Effectively greenfield.

Stack decisions (confirmed): **Axum + Tower**, **SQLx with compile-time-checked queries**, drop-in API/schema compatibility, full plan first then implement in one pass.

## Goals & Non-Goals

**Goals**
- Same SQLite file format & all 17 migrations replayable in order; existing prod DB files must keep working.
- HTTP surface identical: paths, query params, JSON shapes, status codes, cookie semantics.
- Env vars and `.env` loading behavior identical (incl. relative-path resolution, TZ default).
- Docker image and `docker-entrypoint.sh` work unchanged (binary name `cpa-usage-keeper`, same paths under `/data`).
- React frontend embedded the same way (build output at `web/dist/`).

**Non-Goals**
- No schema redesign, no API redesign, no auth-mechanism upgrade (in-memory sessions stay; no DB-backed sessions, no bcrypt migration).
- Not porting Go tests verbatim — we'll write targeted Rust tests for the high-risk areas (migrations, time-bucket aggregation, RESP parser, dedup insert, quota normalization).
- No new features, no refactors beyond what the language difference forces.

## Dependency Choices (Cargo.toml)

```toml
[package]
name = "cpa-usage-keeper"
version = "1.6.0"
edition = "2024"

[dependencies]
# Runtime
tokio = { version = "1", features = ["full"] }
tokio-util = { version = "0.7", features = ["rt"] }       # CancellationToken
futures = "0.3"

# HTTP server
axum = { version = "0.7", features = ["macros", "http2"] }
tower = "0.5"
tower-http = { version = "0.6", features = ["cors", "trace", "set-header", "fs", "compression-gzip"] }
hyper = { version = "1", features = ["full"] }

# HTTP client
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls", "stream", "gzip"] }

# Database
sqlx = { version = "0.8", features = ["runtime-tokio", "tls-rustls", "sqlite", "chrono", "macros", "migrate"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = { version = "1", features = ["raw_value"] }  # equivalent of json.RawMessage
serde_with = "3"                                           # multi-key aliases for flexible field names

# Time
chrono = { version = "0.4", features = ["serde"] }
chrono-tz = "0.10"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt", "json"] }
tracing-appender = "0.2"                                   # daily rolling file

# Config / env
dotenvy = "0.15"

# Crypto / auth
rand = "0.8"
hex = "0.4"
subtle = "2"                                               # constant-time compare
sha2 = "0.10"                                              # for redact module

# Errors
anyhow = "1"
thiserror = "2"

# Static assets (React build)
rust-embed = { version = "8", features = ["interpolate-folder-path"] }
mime_guess = "2"

# Utilities
async-trait = "0.1"
url = "2"
regex = "1"
once_cell = "1"
parking_lot = "0.12"                                       # cheaper Mutex/RwLock

# Redis RESP (raw TCP)
# We implement RESP ourselves over tokio::net::TcpStream + optional tokio-rustls
tokio-rustls = "0.26"
rustls = { version = "0.23", default-features = false, features = ["std", "tls12"] }
webpki-roots = "0.26"

[build-dependencies]
# Optional: pull version from git tag like the Go build did
```

**Rationale notes**
- `rust-embed` for `web/dist/` mirrors Go's `embed.FS`. Build option to fall back to disk when `CPA_DEV_WEB_DIR` is set.
- Raw RESP over TCP (no `redis` crate) preserves the Go behavior: `AUTH` then `LPOP key count`, with HTTP fallback. The Go code is intentionally minimal here and adding `redis-rs` would change failure-mode shapes.
- `rustls` over `native-tls` for portability inside Alpine.
- `sqlx::migrate!` is *not* used for the 17 historical migrations (they include data backfills); we run them via a custom runner that matches Go's `schema_migrations` table exactly.

## Directory Layout

The existing scaffold (`src/api`, `src/app`, `src/auth`, …) maps 1:1 to Go's `internal/*`. We'll fill it. Module conventions:

```
src/
├── main.rs                 # binary entry — parses --env flag, calls app::run
├── lib.rs                  # re-exports for tests
├── config/mod.rs           # Config struct + load()
├── logging/mod.rs          # tracing setup + daily rolling file
├── version/mod.rs          # const VERSION
├── redact/mod.rs           # sha256-based redaction helpers
├── entities/mod.rs         # plain structs (FromRow), no ORM
├── repository/
│   ├── mod.rs              # OpenDatabase equivalent (pool + pragmas + migrate)
│   ├── usage_events.rs     # insert/list/filter/snapshot
│   ├── redis_inbox.rs      # insert/mark*/list/cleanup
│   ├── identities.rs       # list/page/upsert
│   ├── pricing.rs          # list/upsert/delete
│   ├── aggregation.rs      # series, summary, health, analysis
│   ├── dto.rs              # repodto.* equivalents
│   └── migration/
│       ├── mod.rs          # runner: tx-wrapped, schema_migrations table
│       └── m20260503_*.rs  # one file per migration (17 total)
├── backup/mod.rs           # uses sqlite3 backup API via sqlx raw conn
├── cpa/
│   ├── mod.rs              # endpoint constants
│   ├── client.rs           # reqwest-based Client mirroring Go signatures
│   ├── redis_queue.rs      # raw RESP + HTTP fallback
│   └── dto/
│       ├── api_call.rs
│       ├── auth_files.rs
│       ├── external_keys.rs
│       ├── models.rs
│       ├── provider_config.rs
│       └── response.rs     # *Result wrappers (status, body, payload)
├── auth/mod.rs             # SessionManager + middleware extractor
├── service/
│   ├── mod.rs
│   ├── usage.rs            # UsageProvider
│   ├── pricing.rs
│   ├── identity.rs
│   ├── sync.rs             # SyncService (poller-facing)
│   └── dto.rs              # service-layer DTOs
├── quota/
│   ├── mod.rs              # Service + ProviderRegistry
│   ├── providers/
│   │   ├── mod.rs          # ProviderHandler trait
│   │   ├── antigravity.rs
│   │   ├── claude.rs
│   │   ├── codex.rs
│   │   ├── gemini_cli.rs
│   │   └── kimi.rs
│   └── refresh.rs          # async task pool, 20-min TTL, 5 workers
├── poller/
│   ├── mod.rs              # BackgroundPoller (RedisDrain)
│   ├── status.rs           # shared Status (RwLock)
│   ├── drain.rs            # pull + process loops
│   └── runners.rs          # MetadataSyncRunner, StorageCleanupRunner, DatabaseBackupRunner
├── updatecheck/mod.rs      # GitHub releases checker
├── app/
│   ├── mod.rs              # App struct, New/Run/Close lifecycle
│   └── shutdown.rs         # CancellationToken + JoinSet wiring
└── api/
    ├── mod.rs              # router(), base-path Router::nest
    ├── error.rs            # ApiError -> IntoResponse mapping
    ├── extractors.rs       # SessionAuth, custom query parsers
    ├── auth_handler.rs
    ├── status_handler.rs
    ├── sync_handler.rs
    ├── usage_handler.rs
    ├── pricing_handler.rs
    ├── identity_handler.rs
    ├── quota_handler.rs
    ├── update_handler.rs
    └── static_assets.rs    # index.html templating + assets serving
```

## Implementation Order

Each phase is bottom-up; downstream code can compile when its phase finishes.

1. **Foundation** (`config`, `version`, `redact`, `logging`)
2. **Entities** (`entities/*` plain Rust structs with `FromRow`)
3. **DB open + migration runner + 17 migrations** (`repository/migration/*`, `repository/mod.rs`)
4. **Repository queries** (`repository/{usage_events,redis_inbox,identities,pricing,aggregation,dto}.rs`)
5. **Backup writer** (`backup/`)
6. **CPA DTOs + HTTP client + Redis queue** (`cpa/`)
7. **Service layer** (`service/`) — depends on repository + cpa
8. **Quota providers + refresh pool** (`quota/`)
9. **Auth (SessionManager)** (`auth/`)
10. **Poller + runners** (`poller/`)
11. **API handlers + router + static assets** (`api/`)
12. **App wiring + main** (`app/`, `main.rs`)
13. **Update Dockerfile** (replace Go build stage with `cargo build --release`; runtime stage unchanged)

## Per-Module Porting Details

### config (drop-in)
Single `Config` struct with all 26 fields documented in the analysis (`AppPort`, `AppBasePath`, `CPABaseURL`, `CPAManagementKey`, `RedisQueue*`, `MetadataSyncInterval` const 30s, `WorkDir`, `SQLitePath`, `Backup*`, `RequestTimeout`, `TLSSkipVerify`, `LogLevel`, `LogFile*`, `LogDir`, `LogRetentionDays`, `AuthEnabled`, `LoginPassword`, `AuthSessionTTL`).
- `load(env_file: Option<&Path>) -> anyhow::Result<Config>` — tries `.env` in CWD, then exe dir, then env-only.
- Default TZ `Asia/Shanghai` via `chrono_tz::Asia::Shanghai` (only affects log timestamps; DB times stored UTC like Go does).
- Validate at boot; required fields error out (`CPA_BASE_URL`, `CPA_MANAGEMENT_KEY`, `LOGIN_PASSWORD` when auth enabled).
- All durations parsed by `humantime::parse_duration` then converted, or hand-roll `parse_duration_go` matching Go's `time.ParseDuration` for `"30s"`, `"24h"` strings.

### logging
- `tracing_subscriber::registry()` with `EnvFilter::new(cfg.LogLevel)`, fmt layer to stderr.
- If `LogFileEnabled`: add `tracing_appender::rolling::daily(cfg.LogDir, "cpa-usage-keeper")` layer; non-blocking writer kept in `LogCloser` returned to `App` for graceful flush on shutdown.
- Background task: every 24h prune files older than `LogRetentionDays`.

### entities
Plain `#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]` structs. Field names mapped exactly to column names. `time.Time` → `DateTime<Utc>`. Nullable Go pointers → `Option<T>`. Soft-delete `DeletedAt: *time.Time` → `Option<DateTime<Utc>>`.

### repository::open
Mirror `OpenDatabase`:
- Build `SqliteConnectOptions` with `journal_mode=WAL`, `busy_timeout=5000`, `foreign_keys=true`, `create_if_missing=true`.
- `SqlitePoolOptions::new().max_connections(1).min_connections(1)` (Go uses MaxOpenConns=1 — SQLite single-writer).
- Run migration runner.

### repository::migration runner
Match Go's behavior exactly:
- Create `schema_migrations(version TEXT PRIMARY KEY, applied_at TIMESTAMP)` if missing.
- Each migration is a Rust function `pub async fn up(tx: &mut Transaction<'_, Sqlite>) -> Result<()>` registered in an ordered vec.
- Versions match the Go filenames exactly (e.g., `20260503_add_usage_event_redis_fields`).
- For each: `SELECT 1 FROM schema_migrations WHERE version=?` — if present, log "skipped"; else run in a single `tx`, then `INSERT INTO schema_migrations`.
- The 17 migrations include schema DDL **and** data backfills (extract JSON fields, recompute identity stats). For backfills we'll mirror the Go code field-for-field; these are one-time and well-tested.
- **Critical**: first-boot path. Go uses `gorm.AutoMigrate` to bootstrap fresh DBs. We instead author **bootstrap migration `00000000_initial_schema`** that creates all final tables (the schema as it stands today after migration #17), and the runner records all 17 historical versions plus the bootstrap so that a fresh DB ends in the same state and an existing DB still has the 17 entries it expects. The runner detects "fresh DB" by absence of `usage_events` table.

### repository queries
Use `sqlx::query!` / `query_as!` for compile-time checking — requires `DATABASE_URL` to a prepared schema during `cargo build`. We'll commit `.sqlx/` offline metadata so builds don't need a live DB. Build script step: `cargo sqlx prepare --check` is a CI gate.
- `InsertUsageEvents`: batched `INSERT OR IGNORE` (matches GORM's `OnConflict DoNothing` on `event_key`). Return `(inserted, deduped)`.
- `BuildUsageSnapshot`, `ListUsageEventsWithFilter`, `ListUsageEventFilterOptionsWithFilter`: complex aggregation SQL — port verbatim. Will likely need `query_as!` plus dynamic `where` clauses constructed with `sqlx::QueryBuilder` since filters are optional.
- `ListProcessableRedisUsageInbox`: `WHERE status IN ('pending', 'process_failed') AND attempt_count < 3`.

### backup
SQLite Backup API is exposed by `sqlx` via `pool.acquire().await?.lock_handle().await?` -> raw `libsqlite3_sys`. Simpler: use `rusqlite::backup::Backup` against a freshly-opened `rusqlite::Connection` on the same DB path (since we're WAL mode, this is safe).
- Add `rusqlite = { version = "0.32", features = ["bundled"] }` *only* for the backup module to avoid dragging it into queries.
- Atomic write: backup to `database_<ts>.db.tmp`, fsync, rename.
- Day-directory retention (`backups/YYYY-MM-DD/…`) per Go.

### cpa::client
Mirror Go method-for-method:
- `Client { base_url, management_key, http: reqwest::Client }`
- `new(base_url, management_key, timeout, tls_skip_verify) -> Self`. When `tls_skip_verify`: `reqwest::ClientBuilder::danger_accept_invalid_certs(true)`.
- 10 public methods returning `*Result { status_code, body: Bytes, payload }`.
- All endpoint strings from `endpoints.rs` (constants), incl. `ManagementUsageQueueKey = "queue"`.
- `fetch_models` does the two-stage flow (get external API key → call `/v1/models`).
- Error type: `thiserror` enum (BuildRequest, Io, BadStatus { code, body }, Json).

### cpa::dto with flexible JSON
Several Go DTOs accept multiple JSON key names (camelCase, snake_case, kebab-case). Use `#[serde(alias = "...")]`:
```rust
#[serde(rename = "apiKey", alias = "api-key", alias = "key")]
api_key: String,
```
And `#[serde(alias = "auth-index", alias = "auth_index", alias = "authIndex")]`. For `ProjectIDCamel` / `ChatGPTAccountId*` doubles, deserialize into a temp struct then merge via custom `Deserialize`.

### cpa::redis_queue
Raw RESP over TCP:
- `pop_usage(ctx) -> Vec<String>` (batch up to `batch_size`).
- Connect: `tokio::net::TcpStream::connect`, wrap with `tokio_rustls` when TLS scheme detected from `RedisQueueAddr` or `CPA_BASE_URL`.
- Default port 8317 if missing.
- Wire commands: `*2\r\n$4\r\nAUTH\r\n$<n>\r\n<key>\r\n`, then `*3\r\n$4\r\nLPOP\r\n$<n>\r\n<queueKey>\r\n$<n>\r\n<batchSize>\r\n`.
- RESP parser: state-machine over `BufReader` recognizing `+`, `-`, `$`, `*`. Null bulk `$-1` and empty array `*0` distinguished.
- Mutex-guarded `sync_mode` (Redis | Http) cached between calls (Go uses `sync.Mutex` + enum).
- Failure → fall back to `client.fetch_usage_queue(batch_size)`; filter `""` and `"null"`.

### auth
- `SessionManager { sessions: RwLock<HashMap<String, DateTime<Utc>>>, ttl: Duration }`.
- `create()` → 32 random bytes via `rand::thread_rng()` → hex; insert with `now + ttl`.
- `validate(token)` → check expiry; drop if expired; return bool.
- `delete`, `cleanup_expired`.
- Password compare via `subtle::ConstantTimeEq`.
- Failed-login rate limit: separate `IpFailureTracker { map: Mutex<HashMap<IpAddr, FailureState>> }`, 5/IP, sliding window matching Go.
- Axum extractor: `SessionAuth` — pulls cookie `cpa_usage_keeper_session`, calls `validate`, rejects with 401 if invalid (or skipped when `auth.enabled == false`).
- Cookie attributes match Go exactly: `HttpOnly`, `SameSite=Lax`, `Secure` when `X-Forwarded-Proto=https` or actual TLS, `Path=<basePath>/`, `Expires=<ttl>`.

### service
Ports of `UsageProvider`, `PricingProvider`, `UsageIdentityProvider` and `SyncService` from Go. Mostly thin wrappers around repository functions plus DTO mapping. Time-window logic for `UsageOverviewSnapshot` (hourly/daily series, health blocks) must match Go bit-for-bit — port the bucketing algorithm verbatim and add a focused unit test fixed against a captured set of inputs.

### quota
- `ProviderHandler` async trait, one impl per provider in `providers/*.rs`. Each parses provider-specific JSON and returns a normalized `Vec<QuotaRow>`.
- `Service`:
  - `check`: single auth_index, dispatch via registry, return rows. Cache write-through.
  - `get_cached_quota`: batch, in-memory cache (no provider call).
  - `refresh`: spawn refresh tasks; semaphore of 5; per-task `tokio::time::timeout(20s)`; tasks stored in `RwLock<HashMap<TaskId, RefreshTaskState>>` with 20-min TTL janitor.
  - `get_refresh_task`: poll status.
- Refresh source enum: `Manual`, `BackgroundSync`, `Unknown`.

### poller
- `BackgroundPoller::run(cancel: CancellationToken)`:
  - Spawns **pull loop**: every `IdleInterval` calls `redis_queue.pop_usage`, batch-inserts into `redis_usage_inboxes` (status `pending`). Backoff `ErrorBackoff` on failure.
  - Spawns **process loop**: every 5s calls `repository::list_processable_redis_usage_inbox(limit)`, decodes each, on success inserts into `usage_events` and marks `processed`; decode/process failures bump `attempt_count` and store last error.
- Shared `Arc<RwLock<Status>>` exposed for `/api/v1/status`.
- `MetadataSyncRunner` (interval = 30s default): hits CPA endpoints, upserts `usage_identities`.
- `StorageCleanupRunner` (daily): inbox cleanup + `VACUUM`.
- `DatabaseBackupRunner`: scheduled at 04:00 local + on-interval; retry up to 3× at 15-min spacing.
- Each runner takes `CancellationToken` and is `tokio::spawn`ed by `App`.

### api (drop-in HTTP)
- Router: `Router::new().nest(&cfg.app_base_path, api_router).fallback(spa_fallback)`.
- Endpoints (must match exactly):
  - `GET /healthz`, `GET /api/v1/ping`
  - `GET /api/v1/status`
  - `POST /api/v1/sync` (rate-limited 1/sec via `parking_lot::Mutex<Option<Instant>>`)
  - `GET /api/v1/update/check`
  - Auth: `GET /api/v1/auth/session`, `POST /api/v1/auth/login`, `POST /api/v1/auth/logout`
  - Usage: `GET /api/v1/usage/overview`, `/analysis`, `/events`, `/events/filters/models`, `/events/filters/sources`
  - Pricing: `GET /api/v1/models/used`, `GET|PUT /api/v1/pricing`, `PUT /api/v1/pricing/:model`, `DELETE /api/v1/pricing`
  - Identities: `GET /api/v1/usage/identities`, `GET /api/v1/usage/identities/page`
  - Quota: `POST /api/v1/quota/check`, `/cache`, `/refresh`, `GET /api/v1/quota/refresh/:task_id`
- Query parsing: a `UsageFilter` extractor reading `range` (`all|today|4h|8h|12h|24h|7d|30d|custom`), `start/end` (RFC3339 or `YYYY-MM-DD`), `page`, `page_size`/`limit` (whitelist: 20/50/100/500/1000, default 100), `model`, `source`, `auth_index`, `result`.
- Error model: `ApiError` enum → `IntoResponse` mapping the exact status codes Go returns (incl. 409 sync conflict, 422 quota unprocessable, 429 rate-limit, 501 for not-implemented pricing edges).
- Static assets (`web/dist/`) embedded with `rust-embed`:
  - `GET /` (and SPA fallback) → `index.html` with `__APP_BASE_PATH__` replaced at request time; `Cache-Control: no-store`.
  - `GET|HEAD /assets/*path` → `Cache-Control: public, max-age=31536000, immutable`.
  - SPA fallback preserves `/api/*` 404s.

### app + main
- `App::new_with_options(EnvFile)`:
  1. `config::load`
  2. `logging::init`
  3. `repository::open` (runs migrations)
  4. construct services, sync, poller, runners, session manager
  5. build router
- `App::run`:
  - `CancellationToken` + `JoinSet`
  - Spawn poller, runners, log-cleanup
  - `axum::serve(listener, router).with_graceful_shutdown(token.cancelled())`
  - On SIGINT/SIGTERM (`tokio::signal`): cancel token, `join_set.join_all().await`.
- `App::close`: idempotent — flush log appender, close pool.

### Dockerfile
Replace stage 2 only:
```dockerfile
FROM rust:1.84-alpine AS rust-builder
RUN apk add --no-cache musl-dev pkgconf openssl-dev
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY .sqlx ./.sqlx
COPY src ./src
COPY --from=web-builder /app/web/dist ./web/dist
ENV SQLX_OFFLINE=true
RUN cargo build --release --locked
```
Stage 3 (runtime alpine) keeps `app` user, `/data` volume, healthcheck, and entrypoint exactly as today. Binary path becomes `target/release/cpa-usage-keeper`.

## Risks & Gotchas

- **Migration backfills**: migrations #2, #6, #8, #12 do non-trivial JSON parsing and aggregation on existing rows. Port the Go code line-for-line and test on a captured snapshot of a real DB. Don't paraphrase the SQL.
- **Trim-expression indexes**: SQLite ≥3.31. `sqlx`'s bundled SQLite is new enough; verify on the Alpine image we ship.
- **GORM-normalized schema vs. our bootstrap**: must diff Go's `AutoMigrate` output (column order, NULL defaults, index names) against our bootstrap DDL. Plan: dump schema from a Go-initialized DB to `sqlite_schema.snapshot.sql` and write a test that asserts our fresh DB matches it.
- **Flexible JSON aliases**: easy to miss one (`projectId` vs `project_id` etc.). Round-trip every CPA DTO through real captured payloads stored under `tests/fixtures/cpa/`.
- **Cookie `Secure` decision**: must check both real TLS and `X-Forwarded-Proto: https` exactly like Go.
- **In-memory sessions don't survive restarts** — preserved, not "fixed".
- **Static asset placeholder replacement**: `__APP_BASE_PATH__` substitution must happen at request time, not embed time, since the binary is built once but deployed with varying `APP_BASE_PATH`.
- **SQLite single-writer**: keep pool at size 1; otherwise `database is locked` under load.
- **Compile-time checked queries**: complex dynamic filters require `QueryBuilder` (loses compile-time checks for the dynamic parts) — acceptable, but document which queries are dynamic and add runtime tests.

## Verification

End-to-end, executed after the rewrite is in:

1. **Build**: `cargo build --release` and `cargo build --tests` clean. `cargo clippy -- -D warnings`.
2. **Unit tests**:
   - `cargo test -p cpa-usage-keeper --lib` covers RESP parser, config parsing, time-bucket aggregation, redaction, migration runner, quota normalization (one fixture per provider), cpa DTO round-trips.
3. **Migration parity**: a test that takes a Go-produced DB snapshot (committed under `tests/fixtures/db/`), runs the Rust migration runner against it, and asserts `schema_migrations` ends with the same 17 versions and that querying a known row returns expected aggregates.
4. **Fresh-DB parity**: spin up empty SQLite, run migrations, dump schema, `diff` against `tests/fixtures/db/fresh_schema.sql` (captured from Go).
5. **HTTP contract**: integration tests under `tests/http.rs` boot the app on a random port and hit every route with golden JSON request/response files captured from the Go server.
6. **Manual smoke**:
   - `docker compose up --build` against the example file; hit `/healthz`, log in via the React UI, observe usage events ingested from a fake Redis (we'll script a tiny RESP server fixture) and from HTTP fallback, run a sync, check quota for each provider with mocked CPA, trigger and observe a backup file.
7. **Cross-binary swap test**: stop the Go binary against a real prod-ish DB, start the Rust binary against the same `WorkDir`. Hit `/api/v1/usage/overview` and `/api/v1/usage/events?range=24h` and diff JSON responses (after dropping volatile fields like `lastRunAt`). Must be byte-identical for stable fields.
8. **Frontend**: load the dashboard in a browser against the Rust binary; click through Usage / Pricing / Identities / Quota tabs; confirm no console errors and that auth login + logout work.

## Critical Files to Modify

- `Cargo.toml` (replace)
- `src/main.rs`, `src/lib.rs` (replace)
- Everything under `src/*` (greenfield)
- `Dockerfile` (replace Go build stage)
- `.gitignore` (add `target/`, `.sqlx/.cache`)
- `Makefile` (add `cargo build`/`test`/`clippy` targets; keep `verify-frontend`)

## Out of Scope for This PR

- Removing Go sources: keep `internal/`, `cmd/`, `go.mod`, `go.sum` in place during the rewrite so reviewers can diff behavior. A follow-up PR deletes them once the Rust binary is in production for a release cycle.
