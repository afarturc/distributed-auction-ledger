FROM rust:bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl-dev ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /data
COPY --from=builder /app/target/release/ledger /usr/local/bin/ledger

EXPOSE 8080 44000

CMD ["ledger"]