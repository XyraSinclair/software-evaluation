FROM rust:1.97.0-bookworm AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
RUN cargo build --locked --release --bin sevald

FROM debian:bookworm-slim
RUN install -d -o 10001 -g 10001 /var/lib/sevald
COPY --from=build /build/target/release/sevald /usr/local/bin/sevald
USER 10001:10001
EXPOSE 7077
VOLUME ["/var/lib/sevald"]
ENTRYPOINT ["sevald"]
CMD ["serve", "--listen", "0.0.0.0:7077", "--cache-dir", "/var/lib/sevald"]
