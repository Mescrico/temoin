FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
# Dependency layer: rebuilt only when Cargo.toml / Cargo.lock change.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src/bin web && echo 'fn main(){}' > src/main.rs && echo 'fn main(){}' > src/bin/sr.rs \
    && echo '' > src/lib.rs && touch web/landing.html web/admin.html \
    && cargo build --release --locked && rm -rf src web
COPY src src
COPY web web
RUN touch src/main.rs src/lib.rs src/bin/sr.rs && cargo build --release --locked \
    && cp target/release/temoin-server target/release/sr /usr/local/bin/

FROM alpine:3.22
RUN addgroup -S -g 10001 srec && adduser -S -u 10001 -G srec -H -h /data srec \
    && mkdir /data && chown srec:srec /data
COPY --from=build /usr/local/bin/temoin-server /usr/local/bin/sr /usr/local/bin/
USER srec
ENV SR_DATA_DIR=/data SR_BIND=0.0.0.0:8080
VOLUME ["/data"]
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s CMD wget -qO- http://127.0.0.1:8080/healthz || exit 1
ENTRYPOINT ["/usr/local/bin/temoin-server"]
