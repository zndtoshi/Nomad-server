# Build stage
FROM rust:1.83-bookworm AS builder

WORKDIR /build

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace and source files
COPY Cargo.toml ./
COPY app/server ./app/server

# Build the binary in release mode
RUN cargo build --release --bin balancebridge-server

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary from builder
COPY --from=builder /build/target/release/balancebridge-server /app/balancebridge-server

# Ensure binary is executable
RUN chmod +x /app/balancebridge-server

# Create data directory
RUN mkdir -p /data && chmod 755 /data

# Persist data volume
VOLUME ["/data"]

# Expose port 3829
EXPOSE 3829

# Set environment variables (can be overridden by Umbrel)
ENV UMBREL_APP_DATA_DIR=/data
ENV UMBREL_APP_ID=balancebridge

# Run the server
CMD ["/app/balancebridge-server"]

