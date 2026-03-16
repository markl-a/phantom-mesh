# ============================================================================
# Clawtex-Core Dockerfile — Route G (Managed Hosting)
# Multi-stage build: Rust compile -> slim Debian runtime
#
# Build:  docker build -t clawtex-core .
# Run:    docker-compose up -d
# ============================================================================

# ── Stage 1: Builder ──────────────────────────────────────────────────────────
FROM rust:1.83-bookworm AS builder

WORKDIR /build

# Cache dependencies: copy manifests first, build a dummy lib to warm the cache
COPY Cargo.toml Cargo.lock* ./
RUN mkdir -p src && \
    echo "fn main() {}" > src/main.rs && \
    echo "" > src/lib.rs && \
    cargo build --release 2>/dev/null || true && \
    rm -rf src

# Copy full source and build the real binary
COPY src/ src/
RUN cargo build --release && \
    strip target/release/clawtex-core

# ── Stage 2: Runtime ─────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

# Prevent interactive prompts during install
ENV DEBIAN_FRONTEND=noninteractive

# Install runtime dependencies:
#   - ca-certificates, curl: HTTPS connections to APIs
#   - python3, python3-pip, python3-venv: browser tool (Playwright), email tool
#   - pandoc: PDF export
#   - libsqlite3-0: SQLite shared library (binary bundles it, but just in case)
#   - fonts-noto-cjk: CJK font support for PDF/browser rendering
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    python3 \
    python3-pip \
    python3-venv \
    pandoc \
    libsqlite3-0 \
    fonts-noto-cjk \
    texlive-xetex \
    texlive-fonts-recommended \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# Install Playwright in a virtual environment
RUN python3 -m venv /opt/playwright-env && \
    /opt/playwright-env/bin/pip install --no-cache-dir playwright && \
    /opt/playwright-env/bin/playwright install --with-deps chromium && \
    ln -s /opt/playwright-env/bin/python3 /usr/local/bin/playwright-python

# Add playwright-env to PATH so clawtex-core can find it
ENV PATH="/opt/playwright-env/bin:${PATH}"

# Create directory structure for clawtex data
RUN mkdir -p /root/.clawtex/workspace \
             /root/.clawtex/hands \
             /root/.clawtex/skills \
             /root/.clawtex/memory \
             /data

# Copy the compiled binary from builder stage
COPY --from=builder /build/target/release/clawtex-core /usr/local/bin/clawtex-core

WORKDIR /root

# HTTP API port (default 7878 in clawtex-core)
EXPOSE 7878

# Health check: hit the status endpoint
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:7878/status || exit 1

# Volume for persistent config, databases, and workspace
VOLUME ["/root/.clawtex"]

# Run the daemon, binding to 0.0.0.0 so it's accessible outside the container
CMD ["clawtex-core", "--host", "0.0.0.0", "--port", "7878"]
