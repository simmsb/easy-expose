FROM ekidd/rust-musl-builder:1.57.0 AS chef
USER root
RUN cargo install cargo-chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Notice that we are specifying the --target flag!
RUN cargo chef cook --release --target x86_64-unknown-linux-musl --recipe-path recipe.json
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl --bin easy-expose

FROM alpine as runtime
LABEL org.opencontainers.image.source https://github.com/simmsb/easy-expose

RUN apk add --no-cache \
  openssh-client \
  ca-certificates \
  bash
WORKDIR app
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/easy-expose /usr/local/bin/
ENTRYPOINT ["/usr/local/bin/easy-expose"]
