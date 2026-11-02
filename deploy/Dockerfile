FROM rust:1.99.0-slim-bookworm AS build
WORKDIR /build
ARG SOURCE_REVISION=unrecorded
ENV SENTINEL_BUILD_REVISION=$SOURCE_REVISION
COPY Cargo.toml Cargo.lock ./
COPY deploy/config/stress.toml ./deploy/config/stress.toml
COPY deploy/config/monitoring.toml ./deploy/config/monitoring.toml
COPY migrations ./migrations
COPY src ./src
RUN cargo build --release --locked

FROM busybox:stable-musl@sha256:3c6ae8008e2c2eedd141725c30b20d9c36b026eb796688f88205845ef17aa213 AS runtime-tools
RUN mkdir /runtime-bin && ln -s /bin/busybox /runtime-bin/sh && ln -s /bin/busybox /runtime-bin/df && ln -s /bin/busybox /runtime-bin/cat

FROM gcr.io/distroless/cc-debian13:nonroot@sha256:08476c244888bfb425bae1ae4dd7c17f093e972aab8426883d97f3c6f280f7c1
COPY --from=runtime-tools /bin/busybox /bin/busybox
COPY --from=runtime-tools /runtime-bin/ /usr/local/bin/
COPY --from=build /build/target/release/sui-sentinel /usr/local/bin/sui-sentinel
COPY deploy/ops/entrypoint.sh /usr/local/bin/sentinel-entrypoint
COPY deploy/config/monitoring.mainnet.toml deploy/config/suilend.toml /etc/sentinel/
USER 65532:65532
EXPOSE 9108
ENTRYPOINT ["/bin/busybox", "sh", "/usr/local/bin/sentinel-entrypoint"]
