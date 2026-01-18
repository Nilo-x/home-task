# Live Demo

https://jan-horak.xyz

## What it is

A microservices demo that shows distributed tracing and observability. When you create an item through the form, it goes through these services:

- API (Rust) - Handles the request and stores data
- PostgreSQL - Persists the item
- Redpanda (Kafka) - Publishes an event
- OpenTelemetry Collector - Collects traces
- Jaeger - Visualizes the trace

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
