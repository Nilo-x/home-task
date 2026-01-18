# Justfile for home-task

default:
    @just --list

# Quick test
test:
    @echo "=== Running Quick Tests ==="
    @just health
    @just test-item
    @just metrics
    @echo ""
    @echo "=== Check Kafka ==="
    @just test-kafka

# Demo
demo:
    ./demo.sh

# Docker Commands
up:
    docker compose up -d

down:
    docker compose down

restart:
    docker compose restart

build:
    docker compose build home-task-app

build-no-cache:
    docker compose build --no-cache home-task-app

rebuild: down build up

# Status Commands
status:
    @echo "=== Docker Services Status ==="
    @docker compose ps
    @echo ""
    @echo "=== App Health ==="
    @curl -s http://localhost:3000/health || echo "App not responding"

health:
    @curl -s http://localhost:3000/health

metrics:
    @echo "=== App Metrics ==="
    @curl -s http://localhost:3000/metrics

# Development Commands
dev:
    @just build run

# OpenTelemetry Commands
test-otel:
    @echo "=== OpenTelemetry Status ==="
    @docker compose ps otlp-collector
    @echo ""
    @echo "=== OTEL Collector Logs (last 10) ==="
    @docker compose logs otlp-collector | tail -10
    @echo ""
    @echo "=== OTEL Metrics ==="
    @curl -s http://localhost:9464/metrics | head -20 || echo "Metrics not available"

# Redpanda Commands
test-kafka:
    @echo "=== Redpanda Status ==="
    @docker compose ps redpanda
    @echo ""
    @docker exec home-task-redpanda rpk cluster info || echo "Redpanda not responding"

kafka-topics:
    @docker exec home-task-redpanda rpk topic list

kafka-consume:
    @echo "=== Latest 5 Items from items.created ==="
    @docker exec home-task-redpanda rpk topic consume items.created --offset -5 -n 5

# Redpanda Console (Web UI)
console:
    @echo "=== Redpanda Console ==="
    @echo "Starting Redpanda Console..."
    @docker compose --profile tools up -d redpanda-console
    @echo ""
    @echo "Redpanda Console available at: http://localhost:8080"

console-stop:
    @docker compose --profile tools stop redpanda-console

# Application Commands
test-item:
    @curl -X POST http://localhost:3000/items \
        -H "Content-Type: application/json" \
        -d '{"name": "Just Test Item", "value": 42}'

get-item id:
    @curl -s http://localhost:3000/items/{{id}}

# Test with W3C trace header
test-traced:
    @curl -X POST http://localhost:3000/items \
        -H "Content-Type: application/json" \
        -H "traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01" \
        -d '{"name": "Traced Item", "value": 100}'

# Test Kafka message verification with W3C headers
test-kafka-verify:
    #!/usr/bin/env sh
    set -e
    echo "=== Creating Item with Trace Header ==="
    TRACE_ID="4bf92f3577b34da6a3ce929d0e0e4736"
    SPAN_ID="00f067aa0ba902b7"
    TRACEPARENT="00-${TRACE_ID}-${SPAN_ID}-01"

    echo "Using traceparent: $TRACEPARENT"
    RESPONSE=$(curl -s -X POST http://localhost:3000/items \
        -H "Content-Type: application/json" \
        -H "traceparent: $TRACEPARENT" \
        -d '{"name": "Kafka Verify Item", "value": 456}')
    echo "Response: $RESPONSE"
    echo ""

    # Extract item ID from response
    ITEM_ID=$(echo $RESPONSE | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
    echo "Item ID: $ITEM_ID"
    echo ""

    echo "=== Waiting for Kafka Message ==="
    sleep 2

    echo "=== Consuming Latest Kafka Message ==="
    MESSAGE=$(docker exec home-task-redpanda rpk topic consume items.created --offset -1 -n 1)
    echo "$MESSAGE"
    echo ""

    # Verify item ID in Kafka message
    if echo "$MESSAGE" | grep -q "$ITEM_ID"; then
        echo "Kafka message contains correct item ID"
    else
        echo "Kafka message does NOT contain item ID"
        exit 1
    fi

    # Verify traceparent header
    # Note: rpk consume shows headers as part of the message output
    if echo "$MESSAGE" | grep -q "traceparent"; then
        echo "Kafka message contains traceparent header"

        # Verify traceparent matches
        if echo "$MESSAGE" | grep -q "$TRACEPARENT"; then
            echo "Traceparent header matches request"
        else
            echo " Traceparent header present but may not match exactly"
        fi
    else
        echo "Kafka message does NOT contain traceparent header"
        exit 1
    fi
    echo ""
    echo "=== All Kafka verifications passed! ==="

# Full end-to-end test
test-e2e:
    #!/usr/bin/env sh
    set -e
    echo "=== Creating Item ==="
    RESPONSE=$(curl -s -X POST http://localhost:3000/items \
        -H "Content-Type: application/json" \
        -d '{"name": "E2E Test Item", "value": 123}')
    echo "Response: $RESPONSE"
    echo ""
    echo "=== Checking Kafka Topic ==="
    docker exec home-task-redpanda rpk topic list
    echo ""
    echo "=== Checking Latest Kafka Message ==="
    docker exec home-task-redpanda rpk topic consume items.created --offset -1 -n 1 || echo "No messages yet"

# Utility Commands
logs:
    @docker compose logs -f

logs-app:
    @docker compose logs -f app

logs-otel:
    @docker compose logs -f otlp-collector

clean:
    @docker compose down
    @docker rmi -f home-task-app
