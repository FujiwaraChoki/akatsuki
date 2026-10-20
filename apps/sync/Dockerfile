# syntax=docker/dockerfile:1.7
#
# akatsuki-sync: the Rust sync worker image.
#
#   docker build -f apps/sync/Dockerfile -t akatsuki-sync .        (from the repository root)
#
# Multi-stage: compile in the official Rust image, run in distroless (no shell, no package
# manager) as the non-root user. The build context is filtered by apps/sync/Dockerfile.dockerignore,
# an allow-list, so .env files and other secrets can never enter the image.
# Runtime hardening (read-only root, tmpfs, dropped capabilities) is set by infra/ Compose.

# Base images by digest next to the readable tag (D-409): a re-pushed tag cannot change what is built. Bump
# both together in a reviewed change (scripts/check-image-pins.py checks that every FROM and image is pinned).
ARG RUST_IMAGE=rust:1.98.1-trixie@sha256:a8a5f0a1e5fe7dfe1d352591e4a1c7dd2c08fd70475cae872cf3458ba0df0546
ARG RUNTIME_IMAGE=gcr.io/distroless/cc-debian13:nonroot@sha256:e792ab3d241a468a4fd7519ddbbebe66b49b5f365771716ea688ad40b6c6f1c2

FROM ${RUST_IMAGE} AS build
WORKDIR /src
ENV CARGO_TERM_COLOR=never CARGO_INCREMENTAL=0
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p akatsuki-sync \
    && install -D -m 0755 target/release/akatsuki-sync /out/akatsuki-sync \
    && /out/akatsuki-sync --version

FROM ${RUNTIME_IMAGE}
LABEL org.opencontainers.image.title="akatsuki-sync" \
      org.opencontainers.image.description="Akatsuki sync worker" \
      org.opencontainers.image.source="https://github.com/FujiwaraChoki/akatsuki" \
      org.opencontainers.image.licenses="LicenseRef-ASAL-1.0"
COPY --from=build /out/akatsuki-sync /usr/local/bin/akatsuki-sync
ENV BIND_ADDR=0.0.0.0:8787 \
    RUST_LOG=info
EXPOSE 8787
USER nonroot:nonroot
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/akatsuki-sync", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/akatsuki-sync"]
CMD ["serve"]
