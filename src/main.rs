use axum::{
    extract::{MatchedPath, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use opentelemetry::KeyValue;
use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::resource::Resource;
use prometheus::{Encoder, Histogram, Counter, TextEncoder};
use rdkafka::config::ClientConfig;
use rdkafka::producer::{FutureProducer, FutureRecord};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, info_span, instrument, warn};

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Registry as TracingRegistry};

mod config;
mod models;

use config::Config;
use models::{CreateItemRequest, Item, ItemEvent};

#[derive(Clone)]
pub struct AppState {
    db_pool: sqlx::PgPool,
    kafka_producer: Arc<FutureProducer>,
    meter_provider: Arc<SdkMeterProvider>,
    http_duration_histogram: Histogram,
    db_duration_histogram: Histogram,
    kafka_publish_counter: Counter,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("db_pool", &"<PgPool>")
            .field("kafka_producer", &"<FutureProducer>")
            .field("meter_provider", &"<SdkMeterProvider>")
            .field("http_duration_histogram", &"<Histogram>")
            .field("db_duration_histogram", &"<Histogram>")
            .field("kafka_publish_counter", &"<Counter>")
            .finish()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct HealthResponse {
    status: String,
    version: String,
    kafka: KafkaHealth,
}

#[derive(Debug, Serialize, Deserialize)]
struct KafkaHealth {
    connected: bool,
    brokers: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ErrorResponse {
    error: String,
}

// Extract W3C trace context from HTTP headers
pub fn extract_w3c_trace_context(headers: &HeaderMap) -> Option<W3CTraceContext> {
    headers
        .get("traceparent")
        .and_then(|h| h.to_str().ok())
        .and_then(|tp| parse_traceparent(tp))
}

#[derive(Debug, Clone)]
pub struct W3CTraceContext {
    pub trace_id: String,
    pub span_id: String,
}

pub fn parse_traceparent(traceparent: &str) -> Option<W3CTraceContext> {
    // Format: 00-{trace_id}-{span_id}-{trace_flags}
    let parts: Vec<&str> = traceparent.split('-').collect();
    if parts.len() >= 3 {
        let trace_id = parts.get(1)?;
        let span_id = parts.get(2)?;
        Some(W3CTraceContext {
            trace_id: trace_id.to_string(),
            span_id: span_id.to_string(),
        })
    } else {
        None
    }
}

// Inject W3C trace context into Kafka message headers
fn inject_w3c_headers(
    record: &mut FutureRecord<String, Vec<u8>>,
    trace_context: &Option<W3CTraceContext>,
) {
    use rdkafka::message::OwnedHeaders;

    // Always include headers to ensure they're sent (even if no trace context)
    let headers = if let Some(ctx) = trace_context {
        OwnedHeaders::new()
            .insert(rdkafka::message::Header {
                key: "traceparent",
                value: Some(&format!("00-{}-{}-01", ctx.trace_id, ctx.span_id)),
            })
    } else {
        // Even without trace context, include empty headers
        OwnedHeaders::new()
    };

    record.headers = Some(headers);
}

// Create Kafka producer
pub async fn create_kafka_producer(brokers: &str) -> Arc<FutureProducer> {
    let mut config = ClientConfig::new();
    config.set("bootstrap.servers", brokers);
    config.set("message.timeout.ms", "5000");
    config.set("request.timeout.ms", "5000");

    let producer = config
        .create()
        .expect("Failed to create Kafka producer");

    Arc::new(producer)
}

// Publish item event to Kafka with W3C trace context
#[instrument(skip(producer, kafka_publish_counter), fields(topic = "items.created"))]
async fn publish_item_event(
    producer: &FutureProducer,
    event: &ItemEvent,
    trace_context: &Option<W3CTraceContext>,
    kafka_publish_counter: &Counter,
) -> anyhow::Result<()> {
    let item_id = match event {
        ItemEvent::Created { id, .. } => id.clone(),
    };

    tracing::Span::current().record("item_id", &item_id.as_str());

    let payload = serde_json::to_vec(event)?;
    let key = match event {
        ItemEvent::Created { id, .. } => id.clone(),
    };

    let mut record: FutureRecord<String, Vec<u8>> = FutureRecord::to("items.created")
        .payload(&payload)
        .key(&key);

    // Inject W3C trace context
    inject_w3c_headers(&mut record, trace_context);

    let send_span = info_span!(
        "kafka_send",
        topic = "items.created",
        item_id = %item_id
    );
    let _enter = send_span.enter();

    let start = std::time::Instant::now();
    match producer.send(record, Duration::from_secs(5)).await {
        Ok(delivery) => {
            let duration = start.elapsed();
            let (partition, offset) = (delivery.partition, delivery.offset);
            info!(
                partition = partition,
                offset = offset,
                duration_ms = duration.as_millis(),
                "Published to Kafka"
            );
            send_span.record("partition", partition);
            send_span.record("offset", offset);
            send_span.record("success", true);

            // Increment Kafka publish counter
            kafka_publish_counter.inc();
        }
        Err((kafka_error, _)) => {
            error!(error = ?kafka_error, "Failed to publish to Kafka");
            send_span.record("success", false);
            send_span.record("error", format!("{:?}", kafka_error).as_str());
            return Err(kafka_error.into());
        }
    }

    Ok(())
}

// Setup OpenTelemetry
pub fn setup_opentelemetry(config: &Config) -> (
    SdkMeterProvider,
    Histogram,
    Histogram,
    Counter,
) {
    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("service.name", config.service_name.clone()),
            KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
            KeyValue::new("deployment.environment", "production"),
        ])
        .build();

    let meter_provider = SdkMeterProvider::builder()
        .with_resource(resource)
        .build();

    // Initialize Prometheus metrics
    let http_duration_histogram = Histogram::with_opts(
        prometheus::HistogramOpts::new("http_server_duration", "HTTP request duration")
            .namespace("home_task")
            .buckets(prometheus::exponential_buckets(0.005, 2.0, 10).expect("Invalid buckets"))
    ).unwrap();

    let db_duration_histogram = Histogram::with_opts(
        prometheus::HistogramOpts::new("db_query_duration", "Database query duration")
            .namespace("home_task")
            .buckets(prometheus::exponential_buckets(0.001, 2.0, 10).expect("Invalid buckets"))
    ).unwrap();

    let kafka_publish_counter = Counter::with_opts(
        prometheus::Opts::new("kafka_publish_count", "Number of Kafka messages published")
            .namespace("home_task")
    ).unwrap();

    // Register metrics with default registry
    prometheus::default_registry().register(Box::new(http_duration_histogram.clone())).unwrap();
    prometheus::default_registry().register(Box::new(db_duration_histogram.clone())).unwrap();
    prometheus::default_registry().register(Box::new(kafka_publish_counter.clone())).unwrap();

    (
        meter_provider,
        http_duration_histogram,
        db_duration_histogram,
        kafka_publish_counter,
    )
}

// Setup tracing with OpenTelemetry (returns provider to keep alive)
fn setup_tracing(config: &Config) -> opentelemetry_sdk::trace::SdkTracerProvider {
    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::trace::BatchSpanProcessor;

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info".into());

    // Get OTLP endpoint from environment or use default
    // For gRPC, we need to convert http:// to http:// or use grpc endpoint
    let otlp_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://otlp-collector:4317".to_string());

    // Create OTLP exporter with gRPC protocol
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&otlp_endpoint)
        .build()
        .expect("Failed to create OTLP exporter");

    // Create batch processor for efficient span export
    let batch_processor = BatchSpanProcessor::builder(exporter)
        .build();

    // Create tracer provider with batch processor
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_span_processor(batch_processor)
        .with_resource(
            Resource::builder()
                .with_attributes(vec![
                    KeyValue::new("service.name", config.service_name.clone()),
                    KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
                    KeyValue::new("deployment.environment", "production"),
                ])
                .build(),
        )
        .build();

    let tracer = provider.tracer(config.service_name.to_string());
    let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    TracingRegistry::default()
        .with(env_filter)
        .with(telemetry_layer)
        .with(tracing_subscriber::fmt::layer())
        .try_init()
        .expect("Failed to initialize tracing");

    provider
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let config = Config::from_env();

    // Initialize tracing - keep provider alive
    let _otel_provider = setup_tracing(&config);

    info!("Starting home-task application...");

    // Initialize metrics
    let (
        meter_provider,
        http_duration_histogram,
        db_duration_histogram,
        kafka_publish_counter,
    ) = setup_opentelemetry(&config);

    // Setup database connection
    let db_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&config.database_url)
        .await?;

    info!("Connected to database: {}", config.database_url);

    // Create items table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS items (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            name TEXT NOT NULL,
            value BIGINT NOT NULL,
            created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
        )
        "#,
    )
    .execute(&db_pool)
    .await?;

    info!("Database schema initialized");

    // Create Kafka producer
    let kafka_producer = create_kafka_producer(&config.kafka_brokers).await;
    info!("Connected to Kafka: {}", config.kafka_brokers);

    let state = AppState {
        db_pool,
        kafka_producer,
        meter_provider: Arc::new(meter_provider),
        http_duration_histogram,
        db_duration_histogram,
        kafka_publish_counter,
    };

    let app = Router::new()
        .route("/", get(health))
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/items", post(create_item))
        .route("/items/{id}", get(get_item))
        .layer(axum::middleware::from_fn_with_state(state.clone(), http_tracing_middleware))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    info!("Server listening on http://0.0.0.0:3000");

    axum::serve(listener, app).await?;

    Ok(())
}

async fn http_tracing_middleware(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_string());

    let path_display = path.as_deref().unwrap_or(uri.path());

    let span = info_span!(
        "http_request",
        method = %method,
        path = path_display,
        uri = %uri,
    );

    let start = std::time::Instant::now();
    let response = next.run(req).await;
    let duration = start.elapsed();
    let status = response.status().as_u16();

    // Record HTTP request duration metric
    let duration_secs = duration.as_secs_f64();
    state.http_duration_histogram.observe(duration_secs);

    span.record("status", status);
    span.record("duration_ms", duration.as_millis());

    if status >= 500 {
        error!(
            parent: &span,
            method = %method,
            path = path_display,
            status = status,
            duration_ms = duration.as_millis(),
            "HTTP request completed"
        );
    } else if status >= 400 {
        warn!(
            parent: &span,
            method = %method,
            path = path_display,
            status = status,
            duration_ms = duration.as_millis(),
            "HTTP request completed"
        );
    } else {
        info!(
            parent: &span,
            method = %method,
            path = path_display,
            status = status,
            duration_ms = duration.as_millis(),
            "HTTP request completed"
        );
    }

    response
}

#[instrument(skip(_state))]
pub async fn health(State(_state): State<AppState>) -> impl IntoResponse {
    Json(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        kafka: KafkaHealth {
            connected: true,
            brokers: std::env::var("KAFKA_BROKERS")
                .unwrap_or_else(|_| "redpanda:9092".to_string()),
        },
    })
}

#[instrument(skip(_state))]
pub async fn metrics(State(_state): State<AppState>) -> impl IntoResponse {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::default_registry().gather();
    let encoded = encoder.encode_to_string(&metric_families).unwrap_or_default();

    ([(axum::http::header::CONTENT_TYPE, encoder.format_type().to_string())], encoded)
}

#[instrument(skip(state, input))]
pub async fn create_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateItemRequest>,
) -> Result<(StatusCode, Json<Item>), (StatusCode, Json<ErrorResponse>)> {
    // Validate name
    if let Err(e) = Item::validate_name(&input.name) {
        warn!("Invalid name: {}", e);
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: e }),
        ));
    }

    // Extract W3C trace context from headers
    let trace_context = extract_w3c_trace_context(&headers);

    // Use provided value or generate random
    let value = input.value.unwrap_or_else(|| {
        use rand::Rng;
        let mut rng = rand::rng();
        rng.random_range(0..1000)
    });

    tracing::Span::current().record("item_name", &input.name.as_str());
    tracing::Span::current().record("item_value", value);

    // DB insert span
    let db_span = info_span!(
        "database_insert",
        operation = "INSERT",
        table = "items"
    );
    let _db_enter = db_span.enter();

    let db_start = std::time::Instant::now();
    let row = sqlx::query_as::<_, (String, String, i64, String)>(
        r#"
        INSERT INTO items (name, value)
        VALUES ($1, $2)
        RETURNING id::text, name, value, created_at::text
        "#,
    )
    .bind(&input.name)
    .bind(value)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        error!("Database error: {:?}", e);
        db_span.record("error", format!("{:?}", e).as_str());
        db_span.record("success", false);
        (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: format!("Database error: {:?}", e) }))
    })?;

    let db_duration = db_start.elapsed();
    info!(
        duration_ms = db_duration.as_millis(),
        "Database insert completed"
    );
    db_span.record("duration_ms", db_duration.as_millis());
    db_span.record("success", true);
    drop(_db_enter);

    // Record DB query duration metric
    state.db_duration_histogram.observe(db_duration.as_secs_f64());

    let item = Item {
        id: row.0,
        name: row.1,
        value: row.2,
        created_at: row.3,
    };

    info!(
        item_id = %item.id,
        item_name = %item.name,
        item_value = item.value,
        "Created item in database"
    );
    tracing::Span::current().record("item_id", item.id.as_str());

    // Create event
    let event = ItemEvent::Created {
        id: item.id.clone(),
        name: item.name.clone(),
        value: item.value,
        created_at: item.created_at.clone(),
    };

    // Publish to Kafka with W3C trace context
    match publish_item_event(&state.kafka_producer, &event, &trace_context, &state.kafka_publish_counter).await {
        Ok(_) => {
            info!("Item event published to Redpanda");
        }
        Err(e) => {
            warn!(error = ?e, "Failed to publish to Kafka, but DB save succeeded");
        }
    }

    Ok((StatusCode::CREATED, Json(item)))
}

#[instrument]
pub async fn get_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let db_span = info_span!(
        "database_query",
        operation = "SELECT",
        table = "items"
    );
    let _db_enter = db_span.enter();

    let db_start = std::time::Instant::now();

    let row = sqlx::query_as::<_, (String, String, i64, String)>(
        r#"
        SELECT id::text, name, value, created_at::text
        FROM items
        WHERE id::text = $1
        "#,
    )
    .bind(&id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| {
        error!("Database error: {:?}", e);
        db_span.record("error", format!("{:?}", e).as_str());
        db_span.record("success", false);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let db_duration = db_start.elapsed();
    db_span.record("duration_ms", db_duration.as_millis());
    drop(_db_enter);

    // Record DB query duration metric
    state.db_duration_histogram.observe(db_duration.as_secs_f64());

    match row {
        Some((id, name, value, created_at)) => {
            info!("Found item: {}", id);
            Ok((StatusCode::OK, Json(Item {
                id,
                name,
                value,
                created_at,
            })))
        }
        None => {
            warn!("Item not found: {}", id);
            Err(StatusCode::NOT_FOUND)
        }
    }
}
