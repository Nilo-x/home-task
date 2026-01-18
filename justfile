# Justfile for home-task

default:
    @just --list

# Quick test
test:
    @just health
    @just test-item
    @just metrics
    @just test-kafka

# App Commands
health:
    @curl -s http://localhost:3000/health

metrics:
    @curl -s http://localhost:3000/metrics

test-item:
    @curl -X POST http://localhost:3000/items \
        -H "Content-Type: application/json" \
        -d '{"name": "Test Item", "value": 42}'

# OpenTelemetry Commands
test-otel:
    @docker compose logs otlp-collector | tail -10

# Redpanda Commands
test-kafka:
    @docker exec home-task-redpanda rpk cluster info || echo "Redpanda not responding"

kafka-topics:
    @docker exec home-task-redpanda rpk topic list

kafka-consume:
    @echo "=== Latest 5 items ==="
    @docker exec home-task-redpanda rpk topic consume items.created --offset -5 -n 5

# Tracing tests
test-traced:
    @curl -X POST http://localhost:3000/items \
        -H "Content-Type: application/json" \
        -H "traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01" \
        -d '{"name": "Traced Item", "value": 100}'

test-kafka-verify:
    #!/usr/bin/env sh
    TRACE_ID="4bf92f3577b34da6a3ce929d0e0e4736"
    SPAN_ID="00f067aa0ba902b7"
    TRACEPARENT="00-${TRACE_ID}-${SPAN_ID}-01"
    RESPONSE=$(curl -s -X POST http://localhost:3000/items \
        -H "Content-Type: application/json" \
        -H "traceparent: $TRACEPARENT" \
        -d '{"name": "Kafka Verify Item", "value": 456}')
    ITEM_ID=$(echo $RESPONSE | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
    sleep 1
    MESSAGE=$(docker exec home-task-redpanda rpk topic consume items.created --offset -1 -n 1)
    echo "$MESSAGE" | grep -q "$ITEM_ID" && echo "✓ Kafka has item ID"
    echo "$MESSAGE" | grep -q "traceparent" && echo "✓ Kafka has traceparent"
    echo "$MESSAGE" | grep -q "$TRACEPARENT" && echo "✓ Traceparent matches"

# Utility Commands
logs:
    @docker compose logs -f

logs-app:
    @docker compose logs -f app

clean:
    @docker compose down
    @docker rmi -f home-task-app

demo:
    ./demo.sh
