# ---------- builder ----------
FROM rust:1.88-bookworm AS builder

# musl toolchain, NIE instalujemy libssl-dev (przechodzimy na rustls)
RUN apt-get update && apt-get install -y --no-install-recommends \
    musl-tools pkg-config \
 && rm -rf /var/lib/apt/lists/*

# target musl
RUN rustup target add aarch64-unknown-linux-musl

WORKDIR /app

# --- cache deps ---
# jeżeli masz Cargo.lock, skopiuj też (lepszy cache)
COPY Cargo.toml Cargo.lock ./
# placeholder src, żeby cargo rozwiązał zależności
RUN mkdir -p src && echo "fn main(){}" > src/main.rs
# jeśli używasz SQLX OFFLINE, skopiuj metadane (opcjonalnie)
# COPY .sqlx .sqlx

# wstępny build zależności (pozwala zcache'ować większość paczek na musl)
RUN cargo build --release --target aarch64-unknown-linux-musl || true

# --- właściwe źródła ---
RUN rm -rf src
COPY . .

COPY .sqlx .sqlx
# (opcjonalnie) jeżeli kompilujesz makra sqlx bez połączenia do DB:
ENV SQLX_OFFLINE=true

# statyczny link na musl
ENV RUSTFLAGS="-C target-feature=+crt-static"
RUN cargo build --release --target aarch64-unknown-linux-musl

# ---------- runtime ----------
FROM alpine:3.18

# certy do TLS (rustls korzysta z systemowych)
RUN apk add --no-cache ca-certificates

WORKDIR /app

# kopiujemy zbudowaną binarkę; NAZWA WYJŚCIOWEGO PLIKU Z CARGO to "rust-bot"
# jeżeli u Ciebie bin ma inną nazwę, podmień poniższą ścieżkę źródłową
COPY --from=builder /app/target/aarch64-unknown-linux-musl/release/raid_bot /app/raid_bot

ENV RUST_LOG=info
RUN chmod +x /app/raid_bot

CMD ["./raid_bot"]
