# syntax=docker/dockerfile:1
# arc/Dockerfile — the clean-sheet Arc image. Public face: the arc-gateway /v1 router.
# Planes behind it: semaphore (relay) · systems (connectors) · console (spatial: operator-supplied).
#
# ONE image, ONE wire contract (docs/arc-api-icd-v1.html): the edge Arc. Nothing the image
# runs needs a GPU, so there is no CUDA target — and no spatial server ships either: the image
# carries no proxy and no python interpreter, so spatial runs ONLY from an operator-supplied
# ARC_SPATIAL_CMD (see arcup), typically pointed at a host MLX/MPS service.
#
# Public port = arc-gateway ($PORT). Planes bind internal ports; the gateway routes.
# Build context = arc/. arc-node (membership) needs pacific-core (cross-repo) so it is NOT in
# the image — the gateway reports membership live:false until it is provisioned separately.

# ── Rust planes: gateway + relay + systems-server + arc-node (membership) ──────────────
FROM rust:1-slim AS rust-build
# git for the private git deps (fetched over HTTPS with a build-time token — no SSH agent
# required); libopenblas-dev to LINK the lodedb math stack (faer/ndarray/cblas).
RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates libopenblas-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY gateway ./gateway
COPY planes ./planes
# The deploy stamp every shipped plane compiles in. An image build sets no ARC_BUILD_* vars, so
# the binaries it produces report `stamped:false` and SAY so — honest, and the reason the edge
# deploy (hosting/deploy-arc.sh, which does set them) is the provenanced path.
COPY lib/arc-build ./lib/arc-build
# A site's public face (face-render), a workspace member since 2fa5bd6: the gateway serves
# faces through it, and cargo cannot load the workspace without it (NC-43).
COPY lib/face-render ./lib/face-render
# Strip the local-dev [patch] so pacific-core resolves from its GIT source (there is no sibling
# Pacific repo in the image), and drop the traffic cockpit (not an image plane). arc-node
# (membership) IS built now — it is what makes the membership gate real.
RUN sed -i '/# PATCH-STRIP-START/,/# PATCH-STRIP-END/d; /"console\/traffic"/d' Cargo.toml
# Build-time GitHub token — GITHUB_TOKEN surfaced via ARG. It fetches the private pacific + LodeDB git deps over HTTPS: the
# git credential helper references the token BY NAME (not value), so no secret lands in a layer,
# and insteadOf rewrites the deps' ssh:// URLs to https://. Locally: --build-arg GITHUB_TOKEN=…
ARG GITHUB_TOKEN
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
RUN git config --global credential.helper '!f() { echo username=x-access-token; echo "password=$GITHUB_TOKEN"; }; f' \
    && git config --global url."https://github.com/".insteadOf "ssh://git@github.com/" \
    && cargo build --release -p arc-gateway -p relay -p systems-server -p node
# → /src/target/release/{arc-gateway,semaphore,systems-server,node}

# ── console — Bun + Vite (static SPA) ─────────────────────────────────────────
FROM oven/bun:1 AS console-build
WORKDIR /console
COPY console/package.json console/bun.lock ./
RUN bun install --frozen-lockfile
COPY console/ ./
RUN bun run build
# → /console/dist

# ── edge (the DEFAULT/last stage — Mac dev Arc + the deployed edge; spatial is operator-supplied) ──
FROM debian:stable-slim AS edge
# libopenblas0 = the runtime BLAS the arc-node binary (lodedb math stack) links against.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libopenblas0 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=rust-build /src/target/release/arc-gateway    /usr/local/bin/arc-gateway
COPY --from=rust-build /src/target/release/semaphore      /usr/local/bin/semaphore
COPY --from=rust-build /src/target/release/systems-server /usr/local/bin/systems-server
COPY --from=rust-build /src/target/release/node           /usr/local/bin/node
COPY --from=console-build /console/dist /opt/arc/console
# The member-gated Arc directory served at GET /v1/arcs (the central Arc's official list).
COPY arcs.json         /opt/arc/arcs.json
ENV ARC_ARCS_FILE=/opt/arc/arcs.json
COPY arcup             /usr/local/bin/arcup
RUN chmod +x /usr/local/bin/arcup
# 8080 gateway (/v1, public) · 8787 relay · 8795 systems · 8790 console · 8791 spatial
# (8791 only answers if the operator supplies ARC_SPATIAL_CMD — nothing in the image binds it)
EXPOSE 8080 8787 8795 8790 8791
CMD ["arcup"]
