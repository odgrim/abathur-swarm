# Human Cerebrate: ClickUp Federation Proxy

## Context

The swarm needs to delegate real-world tasks (opening bank accounts, registering businesses, cloud accounts, PO boxes) to humans. The human cerebrate is a standalone proxy service that speaks the existing federation JSON-RPC protocol on one side and creates/polls ClickUp tasks on the other. The overmind treats it like any other cerebrate — zero changes needed to the core federation code (except adding `send_result`, `send_progress`, and `send_accept` to `FederationHttpClient`).

## Architecture

```
Overmind ──JSON-RPC──> Human Cerebrate Proxy ──REST API──> ClickUp
                       (axum server)                        (human works here)
                       (SQLite state)
                       (poller loop) <──polls────────────── ClickUp
                       ──JSON-RPC──> Overmind (results/heartbeats/progress/accept)
```

## Workspace Setup

Current state: Root `Cargo.toml` is a standalone package (no `[workspace]`). No `crates/` directory exists.

Add at the TOP of the existing root `Cargo.toml`, before `[package]`:
```toml
[workspace]
members = [".", "crates/human-cerebrate"]
```

The root package becomes both workspace root and member (via `"."`).

### New Crate: `crates/human-cerebrate/Cargo.toml`

```toml
[package]
name = "human-cerebrate"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "human-cerebrate"
path = "src/main.rs"

[dependencies]
abathur = { path = "../.." }
# Direct deps (can't inherit — no workspace.dependencies yet)
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
clap = { version = "4", features = ["derive", "env"] }
axum = "0.8"
axum-server = { version = "0.7", features = ["tls-rustls"] }
reqwest = { version = "0.13.1", features = ["json"] }
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite"] }
libsqlite3-sys = { version = "0.30", features = ["bundled"] }
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
async-trait = "0.1"
thiserror = "2"
anyhow = "1"
rustls = "0.23"
rustls-pemfile = "2"
regex = "1"
tower = "0.5"
tower-http = { version = "0.6", features = ["cors", "trace"] }
futures = "0.3.31"

[dev-dependencies]
tempfile = "3"
```

## Module Structure

```
crates/human-cerebrate/
├── Cargo.toml
├── src/
│   ├── main.rs           # CLI, config load, spawn server + poller + heartbeat
│   ├── config.rs          # TOML config types
│   ├── server.rs          # Axum handlers for federation JSON-RPC endpoints
│   ├── state.rs           # SQLite task_mappings CRUD
│   ├── poller.rs          # Background loop: poll ClickUp, send results to overmind
│   ├── heartbeat.rs       # Background loop: send heartbeats to overmind
│   ├── parser.rs          # Best-effort parse human text → FederationResult
│   └── clickup/
│       ├── mod.rs
│       ├── client.rs      # reqwest ClickUp API v2 client (behind trait for testing)
│       └── models.rs      # ClickUp request/response types
└── migrations/
    └── 001_task_mappings.sql
```

## Core FederationHttpClient Changes

**File**: `src/services/federation/service.rs`

Add three new methods to `FederationHttpClient` (and ensure `new()` is pub so the proxy crate can construct one):

### `send_result`
```rust
pub async fn send_result(
    &self, url: &str, result: &FederationResult
) -> Result<(), String>
```
- POST to `{url}/federation/result`
- JSON-RPC body: `{"jsonrpc":"2.0","method":"federation/result","id":1,"params": <FederationResult serialized>}`
- The overmind handler (`handle_federation_result` in `a2a_http.rs:2484`) deserializes params directly as `FederationResult`

### `send_progress`
```rust
pub async fn send_progress(
    &self, url: &str, task_id: Uuid, cerebrate_id: &str,
    phase: &str, progress_pct: f64, summary: &str
) -> Result<(), String>
```
- POST to `{url}/federation/progress`
- JSON-RPC body: `{"jsonrpc":"2.0","method":"federation/progress","id":1,"params":{"task_id":..,"cerebrate_id":..,"phase":..,"progress_pct":..,"summary":..}}`
- Matches the `Params` struct in `a2a_http.rs:2446-2456`

### `send_accept`
```rust
pub async fn send_accept(
    &self, url: &str, task_id: Uuid, cerebrate_id: &str
) -> Result<(), String>
```
- POST to `{url}/federation/accept`
- JSON-RPC body: `{"jsonrpc":"2.0","method":"federation/accept","id":1,"params":{"task_id":..,"cerebrate_id":..}}`

## Key Components

### 1. Configuration (`config.rs`)

TOML config struct with sections: `server`, `identity`, `parent`, `clickup`, `polling`, `database`, `tls`.

```rust
#[derive(Debug, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub identity: IdentityConfig,
    pub parent: ParentConfig,
    pub clickup: ClickUpConfig,
    pub polling: PollingConfig,
    pub database: DatabaseConfig,
    #[serde(default)]
    pub tls: TlsConfig,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub bind_address: String,  // "0.0.0.0"
    pub port: u16,             // 8443
}

#[derive(Debug, Deserialize)]
pub struct IdentityConfig {
    pub cerebrate_id: String,
    pub display_name: String,
    pub capabilities: Vec<String>,
    pub max_concurrent_tasks: u32,
}

#[derive(Debug, Deserialize)]
pub struct ParentConfig {
    pub overmind_url: String,
    pub heartbeat_interval_secs: u64,  // 30
}

#[derive(Debug, Deserialize)]
pub struct ClickUpConfig {
    // api_token from CLICKUP_API_TOKEN env var (not in file)
    pub workspace_id: String,
    pub list_id: String,
    pub completed_statuses: Vec<String>,  // ["complete", "closed", "done"]
    pub failed_statuses: Vec<String>,     // ["cancelled", "rejected"]
}

#[derive(Debug, Deserialize)]
pub struct PollingConfig {
    pub interval_secs: u64,         // 60
    pub task_deadline_secs: u64,    // 1209600 (2 weeks)
    pub progress_interval_secs: u64, // 900 (15 min)
}

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    pub path: String,  // "human-cerebrate.db"
}

#[derive(Debug, Default, Deserialize)]
pub struct TlsConfig {
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
    pub ca_path: Option<String>,
    #[serde(default)]
    pub allow_self_signed: bool,
}
```

Example TOML:
```toml
[server]
bind_address = "0.0.0.0"
port = 8443

[identity]
cerebrate_id = "cerebrate-human"
display_name = "Human Operator"
capabilities = ["real-world", "banking", "registration", "cloud-accounts"]
max_concurrent_tasks = 10

[parent]
overmind_url = "https://overmind.internal:8443"
heartbeat_interval_secs = 30

[clickup]
# api_token read from CLICKUP_API_TOKEN env var
workspace_id = "12345678"
list_id = "901234567"
completed_statuses = ["complete", "closed", "done"]
failed_statuses = ["cancelled", "rejected"]

[polling]
interval_secs = 60
task_deadline_secs = 1209600  # 2 weeks
progress_interval_secs = 900  # 15 min

[database]
path = "human-cerebrate.db"

[tls]
cert_path = "/etc/certs/proxy.crt"
key_path = "/etc/certs/proxy.key"
# ca_path = "/etc/certs/ca.crt"  # optional
# allow_self_signed = false       # default
```

Load function: read path from CLI arg (`--config`), parse TOML, read `CLICKUP_API_TOKEN` from env (exit with error if missing).

### 2. SQLite State (`state.rs`)

**Migration** (`migrations/001_task_mappings.sql`):
```sql
CREATE TABLE IF NOT EXISTS task_mappings (
    federation_task_id TEXT PRIMARY KEY,
    correlation_id     TEXT NOT NULL,
    clickup_task_id    TEXT NOT NULL,
    title              TEXT NOT NULL,
    status             TEXT NOT NULL DEFAULT 'pending',
    priority           TEXT NOT NULL DEFAULT 'normal',
    parent_goal_id     TEXT,
    envelope_json      TEXT NOT NULL,
    clickup_status     TEXT NOT NULL DEFAULT '',
    human_response     TEXT,
    created_at         TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at         TEXT NOT NULL DEFAULT (datetime('now')),
    deadline_at        TEXT NOT NULL,
    result_sent        INTEGER NOT NULL DEFAULT 0
);
```

**TaskMapping struct**: mirrors the table columns with proper Rust types (`Uuid` for IDs stored as TEXT, `chrono::DateTime<Utc>` for timestamps stored as ISO 8601 TEXT).

**CRUD operations** (all async, using `sqlx::SqlitePool`):
- `insert_mapping(pool, mapping: &TaskMapping) -> Result<()>`
- `get_mapping(pool, federation_task_id: &str) -> Result<Option<TaskMapping>>`
- `get_active_mappings(pool) -> Result<Vec<TaskMapping>>` — WHERE `result_sent = 0 AND status IN ('pending', 'in_progress')`
- `get_all_unsent(pool) -> Result<Vec<TaskMapping>>` — WHERE `result_sent = 0` (for reconcile)
- `update_status(pool, federation_task_id: &str, status: &str, clickup_status: &str) -> Result<()>` — also updates `updated_at`
- `update_human_response(pool, federation_task_id: &str, response: &str) -> Result<()>` — also updates `updated_at`
- `mark_result_sent(pool, federation_task_id: &str) -> Result<()>` — sets `result_sent = 1` and updates `updated_at`
- `get_active_task_ids(pool) -> Result<Vec<String>>` — for reconcile handler, returns federation_task_ids where `result_sent = 0`
- `count_active(pool) -> Result<i64>` — for capacity checks and load calculation

### 3. Federation Server (`server.rs`)

Axum router with shared state:
```rust
pub struct AppState {
    pub config: Config,
    pub db: SqlitePool,
    pub clickup: Arc<dyn ClickUpApi>,
    pub federation_client: FederationHttpClient,
}
```

**Routes**:
- `POST /` — JSON-RPC dispatcher (matches overmind's pattern in `a2a_http.rs`)
- `GET /health` — returns 200

**JSON-RPC types** (reuse from `abathur::adapters::mcp::a2a_http` if pub, otherwise define locally):
```rust
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}
```

**JSON-RPC method dispatch**:

| Method | Handler |
|--------|---------|
| `federation/discover` | Return `FederationCard` |
| `federation/delegate` | Accept task, create ClickUp task |
| `federation/register` | ACK (no-op for the proxy) |
| `federation/disconnect` | ACK (no-op, state persists in SQLite) |
| `federation/reconcile` | Return active task IDs from SQLite |
| (unknown method) | JSON-RPC error -32601 MethodNotFound |

#### `federation/discover` handler

Construct and return a `FederationCard`:
```rust
FederationCard {
    card: A2AAgentCard {
        agent_id: config.identity.cerebrate_id.clone(),
        display_name: config.identity.display_name.clone(),
        description: "Human operator proxy — delegates tasks to humans via ClickUp".into(),
        tier: "human".into(),
        capabilities: config.identity.capabilities.clone(),
        accepts: vec![],
        handoff_targets: vec![],
        available: true,
        load: compute_load(&db).await,
    },
    parent_id: None,
    hive_id: None,
    federation_role: FederationRole::Cerebrate,
    max_accepted_tasks: config.identity.max_concurrent_tasks,
    heartbeat_ok: true,
}
```

`compute_load`: `count_active(pool) as f64 / config.identity.max_concurrent_tasks as f64`

#### `federation/delegate` handler — step by step

1. Deserialize params as `FederationTaskEnvelope`
2. Check for duplicate: query SQLite by `task_id`. If found, return `{"status":"accepted","clickup_task_id":"..."}`
3. Check capacity: `count_active(pool)`. If >= `max_concurrent_tasks`, return JSON-RPC error (code -32001, message "at capacity")
4. Map `MessagePriority` to ClickUp priority number: `Urgent→1, High→2, Normal→3, Low→4`
5. Compute deadline: `Utc::now() + chrono::Duration::seconds(config.polling.task_deadline_secs as i64)`
6. Format ClickUp task description from envelope fields (see ClickUp Client section for template)
7. Call `clickup.create_task(list_id, request)`. On failure, return JSON-RPC error (code -32603 InternalError)
8. Insert `TaskMapping` into SQLite with all fields, `envelope_json = serde_json::to_string(&envelope)`
9. Spawn (fire-and-forget) a task to send `federation/accept` to overmind: `federation_client.send_accept(overmind_url, task_id, cerebrate_id)`. If it fails, the heartbeat keeps the connection alive anyway.
10. Return JSON-RPC success: `{"status":"accepted","clickup_task_id":"..."}`

#### `federation/reconcile` handler

1. Deserialize params: `{"cerebrate_id": String, "local_task_ids": Vec<Uuid>}`
2. Query `get_active_task_ids` from SQLite (all federation_task_ids where result_sent = 0)
3. Return JSON-RPC success: `{"task_ids": [list of active task IDs]}`

#### `federation/register` handler

Return JSON-RPC success: `{"status":"registered","cerebrate_id": params.cerebrate_id}`

#### `federation/disconnect` handler

Return JSON-RPC success: `{"status":"disconnected"}`

### 4. ClickUp Client (`clickup/client.rs`)

**Trait** (for testability):
```rust
#[async_trait]
pub trait ClickUpApi: Send + Sync {
    async fn create_task(&self, list_id: &str, req: &CreateTaskRequest) -> Result<CreateTaskResponse>;
    async fn get_task(&self, task_id: &str) -> Result<Option<ClickUpTask>>;
    async fn get_comments(&self, task_id: &str) -> Result<Vec<ClickUpComment>>;
}
```

**`ClickUpClient` struct**:
- Constructor: `new(api_token: String)` — creates `reqwest::Client` with `Authorization: {api_token}` default header and `Content-Type: application/json`
- **ClickUp API v2 endpoints**:
  - Create task: `POST https://api.clickup.com/api/v2/list/{list_id}/task`
  - Get task: `GET https://api.clickup.com/api/v2/task/{task_id}`
  - Get comments: `GET https://api.clickup.com/api/v2/task/{task_id}/comment`
- **get_task**: Returns `Ok(None)` on HTTP 404 (task deleted), propagates other errors
- **get_comments**: Returns deserialized `Vec<ClickUpComment>` from the `comments` array in the API response

**Retry with exponential backoff**: Implement a `retry_with_backoff` helper function (no external crate). Retries on HTTP 429 (rate limit) and 5xx errors. Delays: 1s, 2s, 4s, 8s, 16s, 32s, max 60s. Max 6 attempts. Applied to all three API methods.

**Task description formatting** (Markdown template):
```markdown
## Federation Task: {title}

**Task ID**: {task_id}
**Priority**: {priority}
**Parent Goal**: {parent_goal_summary or "N/A"}
**Deadline**: {deadline_at formatted as human-readable}

### Description
{description}

### Constraints
{constraints as bullet list, or "None" if empty}

### Context
{hints as bullet list, or "None" if empty}

### Related Artifacts
{related_artifacts as bullet list, or "None" if empty}

---
**Instructions**: Complete this task and change the status to "complete".
Add a comment with your results. Include any URLs, account numbers,
or relevant details. You can use a ```json block for structured data.
```

### 5. ClickUp Models (`clickup/models.rs`)

```rust
#[derive(Debug, Serialize)]
pub struct CreateTaskRequest {
    pub name: String,
    pub description: String,   // Markdown-formatted
    pub priority: u8,          // 1=Urgent, 2=High, 3=Normal, 4=Low
    pub due_date: Option<i64>, // Unix ms timestamp (for 2-week deadline)
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClickUpTask {
    pub id: String,
    pub name: String,
    pub status: ClickUpStatus,
    pub date_created: String,
    pub due_date: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClickUpStatus {
    pub status: String,  // lowercase status name
}

#[derive(Debug, Deserialize)]
pub struct ClickUpCommentsResponse {
    pub comments: Vec<ClickUpComment>,
}

#[derive(Debug, Deserialize)]
pub struct ClickUpComment {
    pub id: String,
    pub comment_text: String,
    pub date: String,
    pub user: ClickUpUser,
}

#[derive(Debug, Deserialize)]
pub struct ClickUpUser {
    pub id: u64,
    pub username: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateTaskResponse {
    pub id: String,
    pub name: String,
    pub status: ClickUpStatus,
}
```

### 6. Poller (`poller.rs`)

```rust
pub async fn run_poller(
    state: Arc<AppState>,
    shutdown: tokio::sync::broadcast::Receiver<()>,
)
```

Background `tokio::spawn` loop every `config.polling.interval_secs` seconds:

1. `tokio::select!` between the interval tick and shutdown signal
2. Query `get_active_mappings(pool)` — all rows where `result_sent = 0` and status is `pending`/`in_progress`
3. For each mapping (concurrently via `futures::future::join_all`, bounded to 10 concurrent with `futures::stream::StreamExt::buffer_unordered`):
   a. Call `clickup.get_task(clickup_task_id)`
   b. **Task deleted** (returns `None`):
      - Build `FederationResult::failed(task_id, correlation_id, "ClickUp task deleted", "Task was deleted from ClickUp")`
      - Update SQLite status to `"failed"`
      - Send result to overmind
   c. **Status in `completed_statuses`** (case-insensitive comparison):
      - Call `clickup.get_comments(clickup_task_id)`
      - Run `parse_human_response(comments)` (see parser section)
      - Build `FederationResult::completed(task_id, correlation_id, parsed.summary)` with `.with_artifact(a)` for each parsed artifact
      - Update SQLite: status to `"completed"`, store raw comment text as `human_response`
      - Send result to overmind
   d. **Status in `failed_statuses`** (case-insensitive):
      - Build `FederationResult::failed(task_id, correlation_id, "Task rejected in ClickUp", format!("Task marked as '{}' in ClickUp", clickup_status))`
      - Update SQLite status to `"failed"`
      - Send result to overmind
   e. **Deadline exceeded** (`Utc::now() > deadline_at`):
      - Build `FederationResult::failed(task_id, correlation_id, "Task timed out", format!("Timed out after {} seconds", task_deadline_secs))`
      - Update SQLite status to `"failed"`
      - Send result to overmind
   f. **Still active**:
      - Update `clickup_status` in SQLite if it changed
      - No result sent
4. **Periodic progress updates** (defeats overmind stall detection):
   - Track `last_progress_sent_at` per task in an in-memory `HashMap<String, DateTime<Utc>>` (persisted across loop iterations, not across process restarts — acceptable because the first poll cycle will send progress for all active tasks)
   - If `now - last_progress_sent >= config.polling.progress_interval_secs` (default 900s / 15 min), send `federation/progress` to overmind:
     - `task_id`: the federation task ID
     - `cerebrate_id`: from config
     - `phase`: `"awaiting_human"`
     - `progress_pct`: `0.0`
     - `summary`: `"Waiting for human to complete ClickUp task {clickup_task_id}, current status: {clickup_status}"`
   - This must be < 1800s (overmind default `stall_timeout_secs`) to prevent stall detection
5. **Result delivery**: Call `federation_client.send_result(overmind_url, &result)`.
   - On success: call `mark_result_sent(pool, task_id)`
   - On failure: log warning, leave `result_sent = 0` — will retry next poll cycle

### 7. Heartbeat (`heartbeat.rs`)

```rust
pub async fn run_heartbeat(
    state: Arc<AppState>,
    shutdown: tokio::sync::broadcast::Receiver<()>,
)
```

Background loop every `config.parent.heartbeat_interval_secs` (default 30s):
1. `tokio::select!` between interval tick and shutdown signal
2. Compute load: `count_active(pool) as f64 / config.identity.max_concurrent_tasks as f64`
3. Call `federation_client.send_heartbeat(overmind_url, cerebrate_id, load)`
4. On failure: log warning at `tracing::warn!` level, continue (next tick will retry)

This keeps the cerebrate `Connected` in the overmind, preventing orphan timeout (default 3600s) from firing.

### 8. Response Parser (`parser.rs`)

```rust
pub struct ParsedResponse {
    pub summary: String,
    pub artifacts: Vec<Artifact>,
    pub structured_data: Option<serde_json::Value>,
}

pub fn parse_human_response(comments: &[ClickUpComment]) -> ParsedResponse
```

**Parsing strategy** (applied to comments sorted newest-first):

1. **JSON block extraction**: Regex `(?s)\x60\x60\x60json\s*\n(.*?)\n\s*\x60\x60\x60` (triple backtick json blocks). If found, attempt `serde_json::from_str`. Store as `structured_data`. Extract any `"summary"`, `"artifacts"`, `"notes"` fields if present in the JSON object.

2. **URL extraction**: Regex `https?://[^\s<>")\]]+`. For each URL, classify artifact type:
   - Contains `github.com` + `/pull/` → `"pr_url"`
   - Contains `docs.google.com` or ends in `.pdf`/`.docx` → `"doc_link"`
   - Contains `console.aws.com` or `console.cloud.google.com` or `portal.azure.com` → `"cloud_console_url"`
   - Otherwise → `"link"`

3. **Keyword line extraction**: Regex `(?mi)^(Status|Result|Notes|Account|Reference|URL):\s*(.+)$` (case-insensitive, multiline). Capture key-value pairs. Use `Result:` value as summary if present.

4. **Fallback**: If no structured data, no keyword lines, and no URLs found, use the full text of the most recent comment as the summary (truncated to 2000 chars).

**Priority for summary field**: `Result:` keyword line > `summary` field from JSON block > most recent comment text (truncated to 2000 chars).

**Artifacts**: Merged from all extraction steps (JSON block artifacts + URL-extracted artifacts), deduplicated by value.

### 9. Main Entry Point (`main.rs`)

```rust
#[derive(Parser)]
#[command(name = "human-cerebrate", about = "Federation proxy for human task delegation via ClickUp")]
struct Cli {
    /// Path to config file
    #[arg(long, default_value = "human-cerebrate.toml")]
    config: PathBuf,
}
```

**Startup sequence**:
1. Parse CLI args via clap
2. Load config from TOML file
3. Read `CLICKUP_API_TOKEN` from env (exit with error message if missing)
4. Initialize tracing subscriber (`tracing_subscriber::fmt::init()` with env filter)
5. Create SQLite pool: `SqlitePool::connect_with(SqliteConnectOptions::from_str(&config.database.path)?.create_if_missing(true))`. Run migrations via `sqlx::migrate!("./migrations")`
6. Construct `ClickUpClient::new(api_token)`, `FederationHttpClient::new()`, wrap in `Arc<AppState>`
7. Create `tokio::sync::broadcast::channel::<()>(1)` for shutdown signal
8. Spawn heartbeat loop: `tokio::spawn(run_heartbeat(state.clone(), shutdown_rx.resubscribe()))`
9. Spawn poller loop: `tokio::spawn(run_poller(state.clone(), shutdown_rx.resubscribe()))`
10. Build Axum router with routes and `state` as extension
11. **TLS setup**: If `config.tls.cert_path` and `config.tls.key_path` are both `Some`:
    - Load cert and key via `rustls_pemfile`
    - Build `axum_server::tls_rustls::RustlsConfig` from them
    - Bind with `axum_server::bind_rustls(addr, tls_config)`
    - Otherwise: bind with `axum_server::bind(addr)` (plaintext, for development)
12. Serve with `with_graceful_shutdown` wired to a shutdown future

**Graceful shutdown**:
- `tokio::signal::ctrl_c().await` triggers shutdown
- Broadcast shutdown signal to heartbeat and poller loops
- Heartbeat and poller loops exit via `tokio::select!`
- Axum server drains in-flight requests
- SQLite pool closes after all handles are dropped

## Overmind Registration

Add to the overmind's `abathur.toml`:
```toml
[[federation.cerebrates]]
id = "cerebrate-human"
display_name = "Human Operator"
url = "https://human-proxy.internal:8443"
capabilities = ["real-world", "banking", "registration", "cloud-accounts"]
max_concurrent_delegations = 10
auto_connect = true
```

The overmind's `auto_connect = true` will call `federation/discover` and `federation/register` on the proxy at startup. No self-registration needed from the proxy side.

## Timeout Strategy

The overmind has two timeout mechanisms that would incorrectly kill slow human tasks:
- `task_orphan_timeout_secs` (default 3600): tasks on unreachable cerebrates
- `stall_timeout_secs` (default 1800): no progress received

How the proxy defeats these:
1. **Heartbeat loop** (every 30s) keeps the cerebrate `Connected`, so orphan timeout never fires
2. **Periodic progress updates** (every 15 min, < 30 min stall threshold) reset the stall timer
3. **Internal 2-week deadline** is enforced by the proxy itself, not the overmind

## Error Handling

- **ClickUp API failures**: Retry with exponential backoff (1s, 2s, 4s, 8s, 16s, 32s, max 60s, max 6 attempts). Retries on HTTP 429 and 5xx. Heartbeat loop continues independently.
- **Task creation failure during delegate**: Return JSON-RPC error (-32603 InternalError) so overmind can redelegate or fail.
- **Overmind unreachable when sending results**: Leave `result_sent = 0` in SQLite, retry next poll cycle.
- **Overmind unreachable for heartbeat**: Log warning, retry next tick.
- **Overmind unreachable for accept**: Fire-and-forget — heartbeat maintains connection anyway.
- **Duplicate delegations**: Check SQLite by task_id first, return existing state instead of creating duplicate ClickUp task.
- **Capacity exceeded**: Return JSON-RPC error (-32001) when active tasks >= max_concurrent_tasks.
- **Process restart**: All state is in SQLite — on startup, resume polling for all unfinished mappings. In-memory progress timestamps reset, triggering immediate progress updates for all active tasks.
- **ClickUp task deleted**: Treat as failed, send `FederationResult::failed` with reason "Task was deleted from ClickUp".

## Critical Files (in abathur core)

| File | Role |
|------|------|
| `src/domain/models/a2a.rs` | `FederationTaskEnvelope`, `FederationResult` (with `::completed`/`::failed` constructors, `with_artifact()`/`with_note()`/`with_suggestion()` builders), `FederationCard`, `A2AAgentCard`, `Artifact`, `FederationTaskStatus`, `MessagePriority`, `FederationRole`, `FederationTaskContext`, `ConnectionState` |
| `src/services/federation/service.rs` | `FederationHttpClient` (add `send_result`, `send_progress`, `send_accept` methods) |
| `src/adapters/mcp/a2a_http.rs` | `JsonRpcRequest`/`JsonRpcResponse`/`JsonRpcError`/`A2AErrorCode` types, server-side handler patterns |
| `src/services/federation/config.rs` | `CerebrateConfig`, `FederationConfig`, `FederationTlsConfig` (reference for TLS pattern) |
| `Cargo.toml` | Must add `[workspace]` section |

## Implementation Order

1. **Workspace + crate scaffold**: Modify root `Cargo.toml`, create `crates/human-cerebrate/` with `Cargo.toml`, `src/main.rs` (stub with clap)
2. **Core client extensions**: Add `send_result`, `send_progress`, `send_accept` to `FederationHttpClient` in `src/services/federation/service.rs`
3. **Config + SQLite**: `config.rs`, `state.rs`, migration SQL
4. **ClickUp client**: `clickup/models.rs`, `clickup/client.rs` with trait + impl + retry logic
5. **Server handlers**: `server.rs` — JSON-RPC dispatch, discover, delegate, register, disconnect, reconcile
6. **Parser**: `parser.rs` — regex-based extraction
7. **Poller**: `poller.rs` — background loop with progress updates
8. **Heartbeat**: `heartbeat.rs` — background loop
9. **Main wiring**: `main.rs` — CLI, startup, TLS, shutdown
10. **Tests**: Unit tests per module, integration test with mock ClickUp trait impl

## Files Modified/Created

| Action | File |
|--------|------|
| **Modify** | `Cargo.toml` (add workspace section) |
| **Modify** | `src/services/federation/service.rs` (add 3 methods to FederationHttpClient) |
| **Create** | `crates/human-cerebrate/Cargo.toml` |
| **Create** | `crates/human-cerebrate/src/main.rs` |
| **Create** | `crates/human-cerebrate/src/config.rs` |
| **Create** | `crates/human-cerebrate/src/state.rs` |
| **Create** | `crates/human-cerebrate/src/server.rs` |
| **Create** | `crates/human-cerebrate/src/poller.rs` |
| **Create** | `crates/human-cerebrate/src/heartbeat.rs` |
| **Create** | `crates/human-cerebrate/src/parser.rs` |
| **Create** | `crates/human-cerebrate/src/clickup/mod.rs` |
| **Create** | `crates/human-cerebrate/src/clickup/client.rs` |
| **Create** | `crates/human-cerebrate/src/clickup/models.rs` |
| **Create** | `crates/human-cerebrate/migrations/001_task_mappings.sql` |

## Verification

1. `cargo build -p human-cerebrate` — compiles with no errors
2. `cargo test -p human-cerebrate` — unit + integration tests pass
3. `cargo build` (root workspace) — existing abathur code still compiles
4. `cargo test` (root workspace) — existing tests still pass
5. Manual: start proxy, `curl -X POST http://localhost:8443/ -d '{"jsonrpc":"2.0","method":"federation/discover","id":1}'` returns valid FederationCard
6. Manual: send delegate request with curl, verify ClickUp task created in configured list
7. Manual: complete the ClickUp task + add comment, verify FederationResult sent to overmind within poll interval
8. Verify heartbeats are sent at the configured interval (check overmind logs)
9. Verify 2-week timeout produces a failed result
10. Verify progress updates are sent every 15 minutes for active tasks
