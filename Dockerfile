# Build stage
FROM golang:1.25.5-alpine3.23 AS builder

WORKDIR /app

# Install build dependencies
RUN apk add --no-cache git ca-certificates tzdata

# Copy go mod files
COPY go.mod go.sum ./
RUN go mod download

# Copy source code
COPY . .

# Build the application
RUN CGO_ENABLED=0 GOOS=linux GOEXPERIMENT=jsonv2 go build \
    -ldflags "-X CleanupDb/cmd.Version=1.0.0 -X CleanupDb/cmd.BuildTime=$(date -u '+%Y-%m-%d_%H:%M:%S')" \
    -o cleanupdb .

# Runtime stage
FROM alpine:3.23

WORKDIR /app

# Install runtime dependencies
RUN apk add --no-cache ca-certificates tzdata

# Copy binary from builder
COPY --from=builder /app/cleanupdb .

# Copy default config (will be overridden by volume mount)
COPY config.json .

# Create logs directory and non-root user
RUN mkdir -p /app/logs && \
    adduser -D -u 1000 cleanupdb && \
    chown -R cleanupdb:cleanupdb /app

# Declare volume for logs
VOLUME ["/app/logs"]

USER cleanupdb

# Default command: start the scheduler daemon
ENTRYPOINT ["./cleanupdb"]
CMD ["serve", "-c", "/app/config.json"]
