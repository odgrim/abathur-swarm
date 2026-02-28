//! Events HTTP Server with SSE streaming.
//!
//! Provides real-time event streaming, historical event queries,
//! and replay capabilities via HTTP endpoints.

use axum::{
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Json,
    },
    routing::{get, post, delete},
    Router,
};
use futures::stream::{self, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::services::event_bus::{EventBus, EventCategory, SequenceNumber, UnifiedEvent};
use crate::services::event_store::{EventQuery, EventStore};

/// Configuration for the Events HTTP Server.
#[derive(Debug, Clone)]
pub struct EventsHttpConfig {
    /// Host to bind to.
    pub host: String,
    /// Port to listen on.
    pub port: u16,
    /// Whether to enable CORS.
    pub enable_cors: bool,
    /// Heartbeat interval for SSE streams (milliseconds).
    pub heartbeat_interval_ms: u64,
    /// Maximum events to return in history queries.
    pub max_history_limit: u32,
    /// Default page size for history queries.
    pub default_page_size: u32,
}

impl Default for EventsHttpConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 9102,
            enable_cors: true,
            heartbeat_interval_ms: 30000,
            max_history_limit: 1000,
            default_page_size: 100,
        }
    }
}

/// Shared state for the Events HTTP server.
pub struct EventsState {
    pub event_bus: Arc<EventBus>,
    pub event_store: Option<Arc<dyn EventStore>>,
    pub config: EventsHttpConfig,
}

impl EventsState {
    pub fn new(
        event_bus: Arc<EventBus>,
        event_store: Option<Arc<dyn EventStore>>,
        config: EventsHttpConfig,
    ) -> Self {
        Self {
            event_bus,
            event_store,
            config,
        }
    }
}

/// Events HTTP Server.
pub struct EventsHttpServer {
    state: Arc<EventsState>,
}

impl EventsHttpServer {
    pub fn new(
        event_bus: Arc<EventBus>,
        event_store: Option<Arc<dyn EventStore>>,
        config: EventsHttpConfig,
    ) -> Self {
        Self {
            state: Arc::new(EventsState::new(event_bus, event_store, config)),
        }
    }

    /// Build the router with all endpoints.
    fn build_router(&self) -> Router {
        let mut router = Router::new()
            .route("/events", get(stream_all_events))
            .route("/events/goals/{goal_id}", get(stream_goal_events))
            .route("/events/tasks/{task_id}", get(stream_task_events))
            .route("/events/replay", get(replay_events))
            .route("/events/history", get(query_history))
            .route("/events/stats", get(get_stats))
            .route("/ws/events", get(ws_events))
            .route("/api/v1/webhooks", post(create_webhook).get(list_webhooks))
            .route("/api/v1/webhooks/{id}", delete(delete_webhook))
            .route("/api/v1/webhooks/{id}/test", post(test_webhook))
            .route("/health", get(health_check))
            .with_state(self.state.clone())
            .layer(TraceLayer::new_for_http());

        if self.state.config.enable_cors {
            router = router.layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any),
            );
        }

        router
    }

    /// Start the server.
    pub async fn serve(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr: SocketAddr =
            format!("{}:{}", self.state.config.host, self.state.config.port).parse()?;
        let router = self.build_router();

        tracing::info!("Events HTTP server listening on {}", addr);

        let listener = TcpListener::bind(addr).await?;
        axum::serve(listener, router).await?;
        Ok(())
    }

    /// Start the server with a shutdown signal.
    pub async fn serve_with_shutdown<F>(
        self,
        shutdown: F,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let addr: SocketAddr =
            format!("{}:{}", self.state.config.host, self.state.config.port).parse()?;
        let router = self.build_router();

        tracing::info!("Events HTTP server listening on {}", addr);

        let listener = TcpListener::bind(addr).await?;
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown)
            .await?;
        Ok(())
    }
}

/// Error response structure.
#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
    code: String,
}

/// Stats response structure.
#[derive(Debug, Serialize)]
struct StatsResponse {
    current_sequence: u64,
    total_events: u64,
    subscriber_count: usize,
    oldest_event: Option<String>,
    newest_event: Option<String>,
    events_by_category: Vec<CategoryCount>,
}

#[derive(Debug, Serialize)]
struct CategoryCount {
    category: String,
    count: u64,
}

/// Health check response.
#[derive(Debug, Serialize)]
struct HealthResponse {
    status: String,
    service: String,
    current_sequence: u64,
    subscriber_count: usize,
}

/// Query parameters for replay endpoint.
#[derive(Debug, Deserialize)]
struct ReplayQuery {
    since: Option<u64>,
    limit: Option<u32>,
}

/// Query parameters for history endpoint.
#[derive(Debug, Deserialize)]
struct HistoryQuery {
    since_sequence: Option<u64>,
    until_sequence: Option<u64>,
    goal_id: Option<Uuid>,
    task_id: Option<Uuid>,
    correlation_id: Option<Uuid>,
    category: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
    order: Option<String>,
}

/// SSE stream of all events with `Last-Event-ID` replay support.
///
/// If the client sends a `Last-Event-ID` header (standard SSE reconnection),
/// events since that sequence are replayed from the journal first, then the
/// stream switches to live events.
async fn stream_all_events(
    State(state): State<Arc<EventsState>>,
    headers: HeaderMap,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let last_event_id = parse_last_event_id(&headers);
    let receiver = state.event_bus.subscribe();
    let heartbeat = Duration::from_millis(state.config.heartbeat_interval_ms);

    let replay_events = get_replay_events(&state, last_event_id, None, None).await;
    let stream = create_event_stream_with_replay(replay_events, receiver, None, None);

    Sse::new(stream).keep_alive(KeepAlive::new().interval(heartbeat))
}

/// SSE stream of events filtered by goal ID with `Last-Event-ID` support.
async fn stream_goal_events(
    State(state): State<Arc<EventsState>>,
    Path(goal_id): Path<Uuid>,
    headers: HeaderMap,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let last_event_id = parse_last_event_id(&headers);
    let receiver = state.event_bus.subscribe();
    let heartbeat = Duration::from_millis(state.config.heartbeat_interval_ms);

    let replay_events = get_replay_events(&state, last_event_id, Some(goal_id), None).await;
    let stream = create_event_stream_with_replay(replay_events, receiver, Some(goal_id), None);

    Sse::new(stream).keep_alive(KeepAlive::new().interval(heartbeat))
}

/// SSE stream of events filtered by task ID with `Last-Event-ID` support.
async fn stream_task_events(
    State(state): State<Arc<EventsState>>,
    Path(task_id): Path<Uuid>,
    headers: HeaderMap,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let last_event_id = parse_last_event_id(&headers);
    let receiver = state.event_bus.subscribe();
    let heartbeat = Duration::from_millis(state.config.heartbeat_interval_ms);

    let replay_events = get_replay_events(&state, last_event_id, None, Some(task_id)).await;
    let stream = create_event_stream_with_replay(replay_events, receiver, None, Some(task_id));

    Sse::new(stream).keep_alive(KeepAlive::new().interval(heartbeat))
}

/// Parse the `Last-Event-ID` header from an SSE reconnection.
fn parse_last_event_id(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
}

/// Query the event store for events since `last_event_id` for replay.
async fn get_replay_events(
    state: &EventsState,
    last_event_id: Option<u64>,
    goal_filter: Option<Uuid>,
    task_filter: Option<Uuid>,
) -> Vec<UnifiedEvent> {
    let since = match last_event_id {
        Some(seq) => seq,
        None => return Vec::new(),
    };

    let store = match &state.event_store {
        Some(s) => s,
        None => return Vec::new(),
    };

    let mut query = EventQuery::new()
        .since_sequence(SequenceNumber(since))
        .limit(state.config.max_history_limit)
        .ascending();

    if let Some(goal_id) = goal_filter {
        query = query.goal_id(goal_id);
    }
    if let Some(task_id) = task_filter {
        query = query.task_id(task_id);
    }

    store.query(query).await.unwrap_or_default()
}

/// Create an SSE stream that replays missed events first, then streams live.
fn create_event_stream_with_replay(
    replay: Vec<UnifiedEvent>,
    receiver: broadcast::Receiver<UnifiedEvent>,
    goal_filter: Option<Uuid>,
    task_filter: Option<Uuid>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    // Phase 1: replay buffered events
    let replay_stream = stream::iter(replay.into_iter().map(|event| {
        let sse_event = Event::default()
            .event(format!("{}", event.category))
            .id(event.sequence.0.to_string())
            .data(serde_json::to_string(&event).unwrap_or_default());
        Ok(sse_event)
    }));

    // Phase 2: live events from broadcast
    let live_stream = stream::unfold(receiver, move |mut rx| {
        let goal_filter = goal_filter;
        let task_filter = task_filter;

        async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if let Some(goal_id) = goal_filter
                            && event.goal_id != Some(goal_id) {
                                continue;
                            }
                        if let Some(task_id) = task_filter
                            && event.task_id != Some(task_id) {
                                continue;
                            }

                        let sse_event = Event::default()
                            .event(format!("{}", event.category))
                            .id(event.sequence.0.to_string())
                            .data(serde_json::to_string(&event).unwrap_or_default());

                        return Some((Ok(sse_event), rx));
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        return None;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        let warning = Event::default()
                            .event("warning")
                            .data(format!("{{\"type\":\"lagged\",\"missed_events\":{}}}", n));
                        return Some((Ok(warning), rx));
                    }
                }
            }
        }
    });

    // Chain replay -> live
    replay_stream.chain(live_stream)
}

// ============================================================================
// WebSocket Streaming
// ============================================================================

/// Query parameters for WebSocket event stream.
#[derive(Debug, Deserialize)]
struct WsEventParams {
    /// Filter by event category.
    category: Option<String>,
    /// Only events since this sequence number (for reconnection).
    since_sequence: Option<u64>,
}

/// WebSocket event upgrade handler.
async fn ws_events(
    ws: WebSocketUpgrade,
    Query(params): Query<WsEventParams>,
    State(state): State<Arc<EventsState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws_events(socket, params, state))
}

/// Handle a WebSocket event stream connection.
async fn handle_ws_events(
    mut socket: WebSocket,
    params: WsEventParams,
    state: Arc<EventsState>,
) {
    let category_filter = params.category.as_deref().and_then(parse_category);

    // Replay missed events if since_sequence is provided
    if let Some(since) = params.since_sequence
        && let Some(ref store) = state.event_store {
            let query = EventQuery::new()
                .since_sequence(SequenceNumber(since))
                .limit(state.config.max_history_limit)
                .ascending();

            if let Ok(events) = store.query(query).await {
                for event in &events {
                    if let Some(cat) = category_filter
                        && event.category != cat {
                            continue;
                        }
                    let json = serde_json::to_string(event).unwrap_or_default();
                    if socket.send(Message::Text(json.into())).await.is_err() {
                        return;
                    }
                }
            }
        }

    // Stream live events
    let mut receiver = state.event_bus.subscribe();

    loop {
        tokio::select! {
            result = receiver.recv() => {
                match result {
                    Ok(event) => {
                        if let Some(cat) = category_filter
                            && event.category != cat {
                                continue;
                            }
                        let json = serde_json::to_string(&event).unwrap_or_default();
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        let warning = format!("{{\"type\":\"lagged\",\"missed_events\":{}}}", n);
                        if socket.send(Message::Text(warning.into())).await.is_err() {
                            break;
                        }
                    }
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    _ => {} // Ignore other messages
                }
            }
        }
    }
}

// ============================================================================
// Webhook Management
// ============================================================================

/// Request to create a webhook subscription.
#[derive(Debug, Deserialize)]
struct CreateWebhookRequest {
    url: String,
    secret: Option<String>,
    filter_category: Option<String>,
    max_failures: Option<u32>,
}

/// Webhook subscription response.
#[derive(Debug, Serialize)]
struct WebhookResponse {
    id: String,
    url: String,
    filter_category: Option<String>,
    active: bool,
    failure_count: u32,
    max_failures: u32,
    created_at: String,
}

/// Create a webhook subscription.
async fn create_webhook(
    State(state): State<Arc<EventsState>>,
    Json(req): Json<CreateWebhookRequest>,
) -> Result<(StatusCode, Json<WebhookResponse>), (StatusCode, Json<ErrorResponse>)> {
    let store = state.event_store.as_ref().ok_or_else(|| {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse {
                error: "Event store not configured".to_string(),
                code: "STORE_NOT_CONFIGURED".to_string(),
            }),
        )
    })?;

    let id = Uuid::new_v4();
    let now = chrono::Utc::now();
    let filter_json = serde_json::json!({
        "category": req.filter_category,
    }).to_string();
    let max_failures = req.max_failures.unwrap_or(10);

    store.create_webhook(
        &id.to_string(),
        &req.url,
        req.secret.as_deref(),
        &filter_json,
        max_failures,
        &now.to_rfc3339(),
    ).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "CREATE_ERROR".to_string(),
            }),
        )
    })?;

    Ok((StatusCode::CREATED, Json(WebhookResponse {
        id: id.to_string(),
        url: req.url,
        filter_category: req.filter_category,
        active: true,
        failure_count: 0,
        max_failures,
        created_at: now.to_rfc3339(),
    })))
}

/// List all webhook subscriptions.
async fn list_webhooks(
    State(state): State<Arc<EventsState>>,
) -> Result<Json<Vec<WebhookResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.event_store.as_ref().ok_or_else(|| {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse {
                error: "Event store not configured".to_string(),
                code: "STORE_NOT_CONFIGURED".to_string(),
            }),
        )
    })?;

    let webhooks = store.list_webhooks().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "LIST_ERROR".to_string(),
            }),
        )
    })?;

    Ok(Json(webhooks.into_iter().map(|w| WebhookResponse {
        id: w.id,
        url: w.url,
        filter_category: w.filter_category,
        active: w.active,
        failure_count: w.failure_count,
        max_failures: w.max_failures,
        created_at: w.created_at,
    }).collect()))
}

/// Delete a webhook subscription.
async fn delete_webhook(
    State(state): State<Arc<EventsState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let store = state.event_store.as_ref().ok_or_else(|| {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse {
                error: "Event store not configured".to_string(),
                code: "STORE_NOT_CONFIGURED".to_string(),
            }),
        )
    })?;

    store.delete_webhook(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "DELETE_ERROR".to_string(),
            }),
        )
    })?;

    Ok(StatusCode::NO_CONTENT)
}

/// Send a test event to a webhook.
async fn test_webhook(
    State(state): State<Arc<EventsState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.event_store.as_ref().ok_or_else(|| {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse {
                error: "Event store not configured".to_string(),
                code: "STORE_NOT_CONFIGURED".to_string(),
            }),
        )
    })?;

    let webhook = store.get_webhook(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "FETCH_ERROR".to_string(),
            }),
        )
    })?;

    let webhook = webhook.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Webhook not found".to_string(),
                code: "NOT_FOUND".to_string(),
            }),
        )
    })?;

    // Send a test event
    let test_payload = serde_json::json!({
        "type": "test",
        "webhook_id": id,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "message": "This is a test event from Abathur",
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let mut request = client.post(&webhook.url)
        .header("Content-Type", "application/json")
        .header("X-Abathur-Event", "test");

    if let Some(ref secret) = webhook.secret {
        let body = test_payload.to_string();
        let signature = compute_hmac_signature(secret, &body);
        request = request.header("X-Abathur-Signature", signature);
    }

    let response = request.json(&test_payload).send().await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(ErrorResponse {
                error: format!("Failed to deliver test event: {}", e),
                code: "DELIVERY_ERROR".to_string(),
            }),
        )
    })?;

    let status = response.status().as_u16();
    Ok(Json(serde_json::json!({
        "delivered": (200..300).contains(&status),
        "response_status": status,
    })))
}

/// Compute HMAC-SHA256 signature for webhook payloads.
fn compute_hmac_signature(secret: &str, body: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(body.as_bytes());
    let result = mac.finalize();
    format!("sha256={}", hex::encode(result.into_bytes()))
}

/// Replay events from a sequence number.
async fn replay_events(
    State(state): State<Arc<EventsState>>,
    Query(params): Query<ReplayQuery>,
) -> Result<Json<Vec<UnifiedEvent>>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.event_store.as_ref().ok_or_else(|| {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse {
                error: "Event store not configured".to_string(),
                code: "STORE_NOT_CONFIGURED".to_string(),
            }),
        )
    })?;

    let since = params.since.unwrap_or(0);
    let limit = params
        .limit
        .unwrap_or(state.config.default_page_size)
        .min(state.config.max_history_limit);

    let query = EventQuery::new()
        .since_sequence(SequenceNumber(since))
        .limit(limit)
        .ascending();

    let events = store.query(query).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "QUERY_ERROR".to_string(),
            }),
        )
    })?;

    Ok(Json(events))
}

/// Query historical events with filters.
async fn query_history(
    State(state): State<Arc<EventsState>>,
    Query(params): Query<HistoryQuery>,
) -> Result<Json<Vec<UnifiedEvent>>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.event_store.as_ref().ok_or_else(|| {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse {
                error: "Event store not configured".to_string(),
                code: "STORE_NOT_CONFIGURED".to_string(),
            }),
        )
    })?;

    let limit = params
        .limit
        .unwrap_or(state.config.default_page_size)
        .min(state.config.max_history_limit);

    let mut query = EventQuery::new().limit(limit);

    if let Some(since) = params.since_sequence {
        query = query.since_sequence(SequenceNumber(since));
    }
    if let Some(until) = params.until_sequence {
        query = query.until_sequence(SequenceNumber(until));
    }
    if let Some(goal_id) = params.goal_id {
        query = query.goal_id(goal_id);
    }
    if let Some(task_id) = params.task_id {
        query = query.task_id(task_id);
    }
    if let Some(corr_id) = params.correlation_id {
        query = query.correlation_id(corr_id);
    }
    if let Some(ref cat) = params.category
        && let Some(category) = parse_category(cat) {
            query = query.category(category);
        }
    if let Some(offset) = params.offset {
        query = query.offset(offset);
    }

    query = match params.order.as_deref() {
        Some("asc") | Some("ascending") => query.ascending(),
        _ => query.descending(),
    };

    let events = store.query(query).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "QUERY_ERROR".to_string(),
            }),
        )
    })?;

    Ok(Json(events))
}

/// Get event store statistics.
async fn get_stats(
    State(state): State<Arc<EventsState>>,
) -> Result<Json<StatsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let current_sequence = state.event_bus.current_sequence().0;
    let subscriber_count = state.event_bus.subscriber_count();

    let (total_events, oldest, newest, by_category) =
        if let Some(ref store) = state.event_store {
            let stats = store.stats().await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: e.to_string(),
                        code: "STATS_ERROR".to_string(),
                    }),
                )
            })?;

            (
                stats.total_events,
                stats.oldest_event.map(|dt| dt.to_rfc3339()),
                stats.newest_event.map(|dt| dt.to_rfc3339()),
                stats
                    .events_by_category
                    .into_iter()
                    .map(|(cat, count)| CategoryCount {
                        category: format!("{}", cat),
                        count,
                    })
                    .collect(),
            )
        } else {
            (0, None, None, vec![])
        };

    Ok(Json(StatsResponse {
        current_sequence,
        total_events,
        subscriber_count,
        oldest_event: oldest,
        newest_event: newest,
        events_by_category: by_category,
    }))
}

/// Health check endpoint.
async fn health_check(State(state): State<Arc<EventsState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        service: "events-http".to_string(),
        current_sequence: state.event_bus.current_sequence().0,
        subscriber_count: state.event_bus.subscriber_count(),
    })
}

/// Parse category string to EventCategory.
fn parse_category(s: &str) -> Option<EventCategory> {
    match s.to_lowercase().as_str() {
        "orchestrator" => Some(EventCategory::Orchestrator),
        "goal" => Some(EventCategory::Goal),
        "task" => Some(EventCategory::Task),
        "execution" => Some(EventCategory::Execution),
        "agent" => Some(EventCategory::Agent),
        "verification" => Some(EventCategory::Verification),
        "escalation" => Some(EventCategory::Escalation),
        "memory" => Some(EventCategory::Memory),
        "scheduler" => Some(EventCategory::Scheduler),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::event_bus::EventBusConfig;

    #[test]
    fn test_events_http_config_default() {
        let config = EventsHttpConfig::default();
        assert_eq!(config.port, 9102);
        assert!(config.enable_cors);
    }

    #[test]
    fn test_parse_category() {
        assert_eq!(parse_category("task"), Some(EventCategory::Task));
        assert_eq!(parse_category("GOAL"), Some(EventCategory::Goal));
        assert_eq!(parse_category("invalid"), None);
    }

    #[tokio::test]
    async fn test_events_state_creation() {
        let bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let state = EventsState::new(bus.clone(), None, EventsHttpConfig::default());
        assert!(state.event_store.is_none());
    }
}
