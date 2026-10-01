# systems-server — the deployable /systems plane (Gmail-via-Nango RAG for the CLA).
#
# Build context = this Nango connector workspace (app/Connectors/nango). Self-contained: the
# connector crates carry NO pacific-core dependency, so — unlike arc-node — this builds and
# deploys on its own. On Railway: set the service root directory to app/Connectors/nango.
#
# Runtime env: PORT (Railway sets it), NANGO_SECRET_KEY (required for live search),
# NANGO_BASE_URL (optional; defaults to Nango Cloud).

FROM rust:1-slim AS build
WORKDIR /build
COPY . .
# Only systems-server + its deps (nango-rag, gmail, __nango) build; linear/outlook/slack are
# workspace-excluded and never touched.
RUN cargo build --release -p systems-server

FROM debian:stable-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /build/target/release/systems-server /usr/local/bin/systems-server
# Not root: it writes nothing at runtime and binds an unprivileged $PORT.
RUN useradd --system --no-create-home --shell /usr/sbin/nologin systems
USER systems
# Documentation only — the process binds $PORT (Railway injects it).
EXPOSE 8790
CMD ["systems-server"]
