# The CI runner: the toolchain ci.yml and deploy.yml need, and the Actions runner, for one job.
# Built and run by ~/wallflowers/dev/ci.sh (make ci); the two lock files arrive as named build contexts.
FROM ubuntu:24.04@sha256:68ee22004a9109b44973313b02ff7b1de250f1de0f5ecb628d73f579aaff33ac

ARG NODE=v24.22.1
ARG NODE_SHA=dbabf7a1cbf947e6059ffad2c27b80041db97dea1e553b62afdee7f13fca336e
ARG RUNNER=2.338.2
ARG RUNNER_SHA=670adb2f268fe21dcbb58b3db0e0aa9d5b3bddfe2c651481336e561ff86de945

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update -q && apt-get install -yq --no-install-recommends \
      build-essential pkg-config git curl jq unzip xz-utils ca-certificates \
      libopenblas-dev postgresql-16 sqlite3 age python3.12 python3.12-venv python3-pip \
    && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL -o /tmp/node.tar.xz https://nodejs.org/dist/$NODE/node-$NODE-linux-arm64.tar.xz \
    && echo "$NODE_SHA  /tmp/node.tar.xz" | sha256sum -c - \
    && tar -xJ -C /opt -f /tmp/node.tar.xz && ln -s /opt/node-$NODE-linux-arm64 /opt/node && rm /tmp/node.tar.xz

# initdb refuses root: the door tests start their own clusters from PG_BIN.
RUN useradd -m -s /bin/bash runner
WORKDIR /home/runner/actions-runner
RUN curl -fsSL -o /tmp/r.tgz https://github.com/actions/runner/releases/download/v$RUNNER/actions-runner-linux-arm64-$RUNNER.tar.gz \
    && echo "$RUNNER_SHA  /tmp/r.tgz" | sha256sum -c - \
    && tar xzf /tmp/r.tgz && rm /tmp/r.tgz && ./bin/installdependencies.sh >/dev/null \
    && mkdir _work && chown -R runner:runner /home/runner

USER runner
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable \
      -t wasm32-unknown-unknown -t x86_64-unknown-linux-gnu \
    && mkdir -p /home/runner/.cargo/registry
COPY --from=auth requirements-dev.lock /tmp/requirements-dev.lock
COPY --from=github deploy-tools.lock /tmp/deploy-tools.lock
RUN python3.12 -m venv /home/runner/venv \
    && /home/runner/venv/bin/pip install -q --require-hashes -r /tmp/requirements-dev.lock \
    && /home/runner/venv/bin/pip install -q --no-deps --require-hashes -r /tmp/deploy-tools.lock

ENV PATH=/home/runner/venv/bin:/home/runner/.cargo/bin:/opt/node/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
# run.sh is PID 1: it passes a stop on to the runner, which fails its job on GitHub before exiting.
ENV RUNNER_MANUALLY_TRAP_SIG=1
ENTRYPOINT ["./run.sh"]
