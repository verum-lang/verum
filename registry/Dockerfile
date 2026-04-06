# Multi-stage build for Verum registry
# Stage 1: Build the registry binary
FROM verum-lang/verum:latest AS builder
WORKDIR /app
COPY . .
RUN verum build --release --profile production

# Stage 2: Minimal runtime image
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

RUN groupadd --gid 1000 verum && \
    useradd --uid 1000 --gid verum --shell /bin/false --create-home verum

COPY --from=builder /app/target/release/verum-registry /usr/local/bin/

RUN mkdir -p /data/packages && chown verum:verum /data/packages

USER verum

EXPOSE 8080

ENV REGISTRY_HOST=0.0.0.0:8080 \
    RUST_LOG=info

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/verum-registry", "--healthcheck"]

CMD ["verum-registry"]
