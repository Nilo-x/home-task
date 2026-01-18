// Integration test - requires running docker compose stack
// Run with: cargo test --test integration_test -- --ignored

use axum::{
    body::Body,
    http::{header, Method, Request},
};
use http_body_util::BodyExt;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::message::BorrowedMessage;
use serde_json::json;
use std::time::Duration;
use tower::ServiceExt;

#[tokio::test]
#[ignore = "requires running docker compose stack"]
async fn test_create_item_end_to_end() {
    // Load config for Kafka connection
    let config = home_task::Config::from_env();

    // Setup database connection
    let db_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect("postgresql://postgres:postgres@localhost:5432/hometask")
        .await
        .expect("Failed to connect to database - is docker compose running?");

    // Create the items table
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
    .await
    .expect("Failed to create items table");

    // Clear existing data
    sqlx::query("TRUNCATE TABLE items")
        .execute(&db_pool)
        .await
        .ok();

    // Build the app router
    let app = build_test_app(db_pool);

    // Test 1: Create item with W3C trace header
    let traceparent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    let request = Request::builder()
        .method(Method::POST)
        .uri("/items")
        .header(header::CONTENT_TYPE, "application/json")
        .header("traceparent", traceparent)
        .body(Body::from(json!({"name": "Test Item", "value": 123}).to_string()))
        .unwrap();

    let response = app
        .oneshot(request)
        .await
        .expect("Failed to get response");

    assert_eq!(response.status(), 201, "Expected 201 Created");

    let body = response
        .into_body()
        .collect()
        .await
        .expect("Failed to read body")
        .to_bytes();

    let item: serde_json::Value =
        serde_json::from_slice(&body).expect("Failed to parse JSON");

    assert!(item.get("id").is_some());
    assert_eq!(item["name"], "Test Item");
    assert_eq!(item["value"], 123);
    assert!(item.get("created_at").is_some());

    let item_id = item["id"].as_str().expect("No ID in response");

    // Test 2: Get item by ID
    let request = Request::builder()
        .method(Method::GET)
        .uri(&format!("/items/{}", item_id))
        .body(Body::empty())
        .unwrap();

    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("Failed to get response");

    assert_eq!(response.status(), 200);

    let body = response
        .into_body()
        .collect()
        .await
        .expect("Failed to read body")
        .to_bytes();

    let retrieved_item: serde_json::Value =
        serde_json::from_slice(&body).expect("Failed to parse JSON");

    assert_eq!(retrieved_item["id"], item_id);
    assert_eq!(retrieved_item["name"], "Test Item");
    assert_eq!(retrieved_item["value"], 123);

    // Test 3: Get non-existent item returns 404
    let fake_id = "00000000-0000-0000-0000-000000000000";
    let request = Request::builder()
        .method(Method::GET)
        .uri(&format!("/items/{}", fake_id))
        .body(Body::empty())
        .unwrap();

    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("Failed to get response");

    assert_eq!(response.status(), 404);

    // Test 4: Validate name - empty name
    let request = Request::builder()
        .method(Method::POST)
        .uri("/items")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"name": ""}).to_string()))
        .unwrap();

    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("Failed to get response");

    assert_eq!(response.status(), 400);

    let body = response
        .into_body()
        .collect()
        .await
        .expect("Failed to read body")
        .to_bytes();

    let error: serde_json::Value =
        serde_json::from_slice(&body).expect("Failed to parse JSON");

    assert!(error.get("error").is_some());

    // Test 5: Verify Kafka message was published with W3C headers
    // Consume from Kafka to verify the message and trace headers
    let kafka_message = consume_kafka_message_with_trace_header(
        &config.kafka_brokers,
        "items.created",
        item_id,
        &traceparent,
    )
    .await
    .expect("Failed to consume Kafka message");

    // Verify the message content
    let event: serde_json::Value = serde_json::from_slice(&kafka_message.payload)
        .expect("Failed to parse Kafka message payload");

    assert_eq!(event["type"], "item_created");
    assert_eq!(event["id"], item_id);
    assert_eq!(event["name"], "Test Item");
    assert_eq!(event["value"], 123);

    // Verify W3C traceparent header was injected
    let trace_header = kafka_message
        .headers
        .as_ref()
        .and_then(|h| h.iter().find(|h| h.key == "traceparent"));

    assert!(
        trace_header.is_some(),
        "Kafka message should contain traceparent header"
    );

    let traceparent_value = trace_header
        .unwrap()
        .value
        .as_ref()
        .expect("traceparent header value should not be empty");

    // Verify traceparent format
    assert!(
        traceparent_value.starts_with("00-"),
        "traceparent should start with '00-'"
    );
    assert_eq!(
        traceparent_value, traceparent,
        "traceparent in Kafka should match request traceparent"
    );

    // Test 6: Create item without value (should use random)
    let request = Request::builder()
        .method(Method::POST)
        .uri("/items")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"name": "Random Value Item"}).to_string()))
        .unwrap();

    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("Failed to get response");

    assert_eq!(response.status(), 201);

    let body = response
        .into_body()
        .collect()
        .await
        .expect("Failed to read body")
        .to_bytes();

    let item: serde_json::Value =
        serde_json::from_slice(&body).expect("Failed to parse JSON");

    assert!(item["value"].is_number());

    // Test 7: Check metrics endpoint
    let request = Request::builder()
        .method(Method::GET)
        .uri("/metrics")
        .body(Body::empty())
        .unwrap();

    let response = app
        .oneshot(request)
        .await
        .expect("Failed to get response");

    assert_eq!(response.status(), 200);

    let body = response
        .into_body()
        .collect()
        .await
        .expect("Failed to read body")
        .to_bytes();

    let metrics = String::from_utf8(body.to_vec()).expect("Invalid UTF-8");

    // Check for Prometheus metrics format
    assert!(
        metrics.contains("# TYPE") || metrics.contains("http_server_duration") || metrics.contains("up"),
        "Metrics endpoint did not return Prometheus format data"
    );
}

#[tokio::test]
#[ignore = "requires running docker compose stack"]
async fn test_health_endpoint() {
    let db_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect("postgresql://postgres:postgres@localhost:5432/hometask")
        .await
        .expect("Failed to connect to database - is docker compose running?");

    let app = build_test_app(db_pool);

    let request = Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(Body::empty())
        .unwrap();

    let response = app
        .oneshot(request)
        .await
        .expect("Failed to get response");

    assert_eq!(response.status(), 200);

    let body = response
        .into_body()
        .collect()
        .await
        .expect("Failed to read body")
        .to_bytes();

    let health: serde_json::Value =
        serde_json::from_slice(&body).expect("Failed to parse JSON");

    assert_eq!(health["status"], "healthy");
    assert!(health.get("version").is_some());
    assert_eq!(health["kafka"]["connected"], true);
}

fn build_test_app(db_pool: sqlx::PgPool) -> axum::Router {
    // Import the main module to access internal items for testing
    use home_task::Config;
    use rdkafka::config::ClientConfig;
    use rdkafka::producer::FutureProducer;
    use opentelemetry::KeyValue;
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use opentelemetry_sdk::resource::Resource;
    use std::sync::Arc;

    let config = Config::from_env();

    // Setup metrics using prometheus directly
    let http_duration_histogram = prometheus::Histogram::with_opts(
        prometheus::HistogramOpts::new("http_server_duration", "HTTP request duration")
            .namespace("home_task")
            .buckets(prometheus::exponential_buckets(0.005, 2.0, 10).expect("Invalid buckets"))
    ).unwrap();

    let db_duration_histogram = prometheus::Histogram::with_opts(
        prometheus::HistogramOpts::new("db_query_duration", "Database query duration")
            .namespace("home_task")
            .buckets(prometheus::exponential_buckets(0.001, 2.0, 10).expect("Invalid buckets"))
    ).unwrap();

    let kafka_publish_counter = prometheus::Counter::with_opts(
        prometheus::Opts::new("kafka_publish_count", "Number of Kafka messages published")
            .namespace("home_task")
    ).unwrap();

    // Register metrics with default registry
    prometheus::default_registry().register(Box::new(http_duration_histogram.clone())).unwrap();
    prometheus::default_registry().register(Box::new(db_duration_histogram.clone())).unwrap();
    prometheus::default_registry().register(Box::new(kafka_publish_counter.clone())).unwrap();

    // Try to create Kafka producer
    let kafka_producer = tokio::runtime::Handle::current()
        .block_on(async {
            let mut kafka_config = ClientConfig::new();
            kafka_config.set("bootstrap.servers", &config.kafka_brokers);
            kafka_config.set("message.timeout.ms", "5000");
            kafka_config.set("request.timeout.ms", "5000");
            let producer = kafka_config.create().expect("Failed to create Kafka producer");
            Arc::new(producer)
        });

    // Setup minimal OTLP meter provider (not using in tests)
    let resource = Resource::new(vec![
        KeyValue::new("service.name", config.service_name.clone()),
        KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
        KeyValue::new("deployment.environment", "test"),
    ]);

    let meter_provider = SdkMeterProvider::builder()
        .with_resource(resource)
        .build();

    // Create test AppState
    let state = home_task::AppState {
        db_pool,
        kafka_producer,
        meter_provider: Arc::new(meter_provider),
        http_duration_histogram,
        db_duration_histogram,
        kafka_publish_counter,
    };

    axum::Router::new()
        .route("/health", axum::routing::get(home_task::health))
        .route("/metrics", axum::routing::get(home_task::metrics))
        .route("/items", axum::routing::post(home_task::create_item))
        .route("/items/:id", axum::routing::get(home_task::get_item))
        .with_state(state)
}

// Helper struct to store consumed Kafka message with headers
struct ConsumedMessage {
    payload: Vec<u8>,
    headers: Option<rdkafka::message::OwnedHeaders>,
}

// Helper function to consume Kafka message with trace headers
async fn consume_kafka_message_with_trace_header(
    brokers: &str,
    topic: &str,
    expected_key: &str,
    expected_traceparent: &str,
) -> anyhow::Result<ConsumedMessage> {
    use rdkafka::consumer::{CommitMode, Consumer};
    use rdkafka::message::Message;

    // Create consumer configuration
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", brokers)
        .set("group.id", "test-consumer-group")
        .set("auto.offset.reset", "latest")
        .create()?;

    // Subscribe to topic
    consumer.subscribe(&[topic])?;

    println!("Waiting for Kafka message on topic '{}' with key '{}'...", topic, expected_key);

    // Poll for messages with timeout
    let start_time = std::time::Instant::now();
    let timeout = Duration::from_secs(10);

    loop {
        if start_time.elapsed() > timeout {
            return Err(anyhow::anyhow!("Timeout waiting for Kafka message"));
        }

        match consumer.poll(Duration::from_millis(500)) {
            Some(Ok(message)) => {
                // Check if this is our message by comparing the key
                if let Some(key) = message.key() {
                    let key_str = std::str::from_utf8(key)
                        .map_err(|e| anyhow::anyhow!("Invalid key UTF-8: {}", e))?;

                    if key_str == expected_key {
                        // Found our message
                        let payload = message
                            .payload()
                            .ok_or_else(|| anyhow::anyhow!("Message has no payload"))?
                            .to_vec();

                        let headers = message.headers().map(|h| h.clone());

                        println!("Found matching Kafka message!");
                        if let Some(ref h) = headers {
                            for header in h.iter() {
                                println!("  Header: {} = {:?}", header.key, header.value);
                            }
                        }

                        // Commit the offset
                        consumer.commit_message(&message, CommitMode::Sync)?;

                        return Ok(ConsumedMessage { payload, headers });
                    }
                }
            }
            Some(Err(e)) => {
                eprintln!("Kafka consumer error: {}", e);
            }
            None => {
                // No message, continue polling
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}
