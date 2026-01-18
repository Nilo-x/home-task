use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub kafka_brokers: String,
    pub otlp_endpoint: String,
    pub service_name: String,
}

impl Config {
    pub fn from_env() -> Self {
        Config {
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgresql://postgres:postgres@postgres:5432/hometask".to_string()),
            kafka_brokers: env::var("KAFKA_BROKERS")
                .unwrap_or_else(|_| "redpanda:9092".to_string()),
            otlp_endpoint: env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
                .unwrap_or_else(|_| "http://otlp-collector:4318".to_string()),
            service_name: env::var("OTEL_SERVICE_NAME")
                .unwrap_or_else(|_| "home-task".to_string()),
        }
    }
}
