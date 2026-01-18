#!/bin/bash
# Demo script for home-task API
# Creates sample items with distributed tracing enabled

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  Home Task API Demo${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Check if API is accessible
echo -e "${YELLOW}Checking API health...${NC}"
if ! curl -s http://localhost:3000/health > /dev/null; then
    echo -e "${RED}Error: API not accessible at http://localhost:3000${NC}"
    echo "Make sure the service is running with: docker compose up -d"
    exit 1
fi
echo -e "${GREEN}✓ API is healthy${NC}"
echo ""

# Function to generate random trace ID (32 hex chars)
generate_trace_id() {
    cat /dev/urandom | tr -dc 'a-f0-9' | fold -w 32 | head -n 1
}

# Function to generate random span ID (16 hex chars)
generate_span_id() {
    cat /dev/urandom | tr -dc 'a-f0-9' | fold -w 16 | head -n 1
}

# Create 5 sample items
NUM_ITEMS=5
echo -e "${BLUE}Creating $NUM_ITEMS sample items with distributed tracing...${NC}"
echo ""

for i in $(seq 1 $NUM_ITEMS); do
    TRACE_ID=$(generate_trace_id)
    SPAN_ID=$(generate_span_id)
    TRACEPARENT="00-${TRACE_ID}-${SPAN_ID}-01"
    VALUE=$((RANDOM % 1000))
    
    echo -e "${YELLOW}Item $i:${NC}"
    echo "  Trace ID: $TRACE_ID"
    echo "  Traceparent: $TRACEPARENT"
    echo -e "  Value: $VALUE"
    
    RESPONSE=$(curl -s -X POST http://localhost:3000/items \
        -H "Content-Type: application/json" \
        -H "traceparent: $TRACEPARENT" \
        -d "{\"name\": \"Demo Item $i\", \"value\": $VALUE}")
    
    # Extract ID from response
    ITEM_ID=$(echo $RESPONSE | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
    
    echo -e "  ${GREEN}✓ Created${NC} Item ID: $ITEM_ID"
    echo ""
    
    # Small delay between requests
    sleep 0.5
done

echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}  Demo Complete!${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo -e "${BLUE}View traces in Jaeger:${NC}"
echo "  http://localhost:16686"
echo ""
echo -e "${BLUE}View Kafka in Redpanda Console:${NC}"
echo "  http://localhost:8080"
echo ""
echo -e "${BLUE}View metrics:${NC}"
echo "  http://localhost:3000/metrics"
echo ""
echo -e "${YELLOW}Try creating an item with tracing:${NC}"
echo -e "${NC}  curl -X POST http://localhost:3000/items \\${NC}"
echo -e "${NC}    -H \"Content-Type: application/json\" \\${NC}"
echo -e "${NC}    -H \"traceparent: 00-\$(uuidgen | tr -d '-')-\$(uuidgen | cut -c1-16)-01\" \\${NC}"
echo -e "${NC}    -d '{\"name\": \"My Item\", \"value\": 42}'${NC}"
echo ""
