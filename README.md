# Live Demo

https://jan-horak.xyz

## What it is

A microservices demo that shows distributed tracing and observability. When you create an item through the form, it goes through these services:

- API (Rust) - Handles request and stores data
- PostgreSQL - Persists item
- Redpanda (Kafka) - Publishes an event
- OpenTelemetry Collector - Collects traces
- Jaeger - Visualizes trace

You can see the entire trace flow in Jaeger, with each span representing a step in the process.

## Links

- Demo: https://jan-horak.xyz
- Redpanda Console: https://console.jan-horak.xyz
- Jaeger UI: https://jaeger.jan-horak.xyz
- API Health: https://jan-horak.xyz/health
- Metrics: https://jan-horak.xyz/metrics

## How to run locally

```bash
docker compose up -d
```

Then visit http://localhost:3000

| Service | Port | Endpoints |
|---------|-------|-----------|
| App | 3000 | /health, /metrics, /items, /items/{id} |
| PostgreSQL | 5432 | - |
| Redpanda | 9092 | - |
| Jaeger | 16686 | / |
| Redpanda Console | 8080 | / |
| OTEL Collector | 4318/4317 | / |
