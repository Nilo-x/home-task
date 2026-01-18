# Stage 1: Builder (Rust - Alpine)
FROM rust:1.88.0-alpine AS builder

ARG BUILD_DATE

# Install only required build dependencies
RUN apk add --no-cache \
    pkgconf \
    openssl-dev \
    postgresql-dev \
    cmake \
    make \
    g++ \
    musl-dev

WORKDIR /app

# Copy Cargo files first for better layer caching
COPY Cargo.toml Cargo.lock* ./

# Create a dummy main.rs for dependency pre-compilation
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs

# Build dependencies (cached layer)
RUN cargo build --release && \
    rm -rf src

# Copy actual source code files
COPY src/ ./src/

# Build application (with cache busting)
RUN echo "Build timestamp: $BUILD_DATE" > /tmp/build.txt && \
    touch src/main.rs && \
    cargo build --release

# Stage 2: Runtime (Alpine)
FROM alpine:3.21.0 AS runtime

# Install only required runtime dependencies
RUN apk add --no-cache \
    libssl3 \
    postgresql-client \
    ca-certificates \
    wget

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/home-task /app/home-task

# Create non-root user for security
RUN addgroup -g 1000 appuser && \
    adduser -D -u 1000 -G appuser appuser && \
    chown -R appuser:appuser /app

# Switch to non-root user
USER appuser

# Expose application port
EXPOSE 3000

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget --no-verbose --tries=1 --spider http://localhost:3000/health || exit 1

# Run application
CMD ["/app/home-task"]
