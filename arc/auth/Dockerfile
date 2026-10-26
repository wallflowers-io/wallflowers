# kenjin-auth — the one box in the sign-in flow that is not static.
#
# It holds what the site image must never contain: the Apple .p8 signing key,
# the PKCE verifiers in flight, every refresh token, and the OreCloud secret
# key. The site container proxies /auth/ here (see the ^~ /auth/ location in
# ../site/default.conf.template) and serves nothing under that path itself.
# By digest (SEC-24): the tag moves. linux/amd64 is 44ff437b…, built 19 Sep 2026.
FROM python:3.12-slim@sha256:f77ac9e44ae96ef2c90b8053ea08c31f8be030f824196b0ae4db6d462c84e51f

# lodedb needs a compiler for none of its wheels on this platform, but pip
# resolving without one fails obscurely if that ever changes. Kept lean instead:
# if a source build appears, add build-essential here rather than debugging it.
WORKDIR /srv

# Litestream: continuous, WAL-aware replication of the identity store to R2.
# Pinned and checksum-verified -- this binary is in the path of every credential
# we hold, so "latest" is not good enough.
ADD https://github.com/benbjohnson/litestream/releases/download/v0.5.17/litestream-0.5.17-linux-x86_64.tar.gz /tmp/litestream.tar.gz
RUN echo "cfb371176d164437ae869f8351cfde49bd1804ae71c61923f75c9cba9c9c006d  /tmp/litestream.tar.gz" | sha256sum -c - \
 && tar -C /usr/local/bin -xzf /tmp/litestream.tar.gz litestream \
 && rm /tmp/litestream.tar.gz \
 && litestream version

COPY requirements.lock ./
COPY app ./app
COPY litestream.yml /etc/litestream.yml
COPY --chmod=755 entrypoint.sh /usr/local/bin/entrypoint.sh
# A failed install MUST fail the build. This used to end in `|| true`, which let a
# transient resolver failure produce an image with no uvicorn in it — one that
# builds green, starts, and dies with "No module named uvicorn".
#
# The lock, not pyproject's ranges (SEC-24): every package by version and hash,
# wheels only, so nothing resolves or compiles at build. The project itself is
# not installed — uvicorn imports app.main from /srv — which also keeps its build
# backend (hatchling, unpinned under build isolation) out of the image.
# Regenerate with the command at the head of requirements.lock.
RUN pip install --no-cache-dir --require-hashes --only-binary=:all: -r requirements.lock

# /data holds auth.db: the (iss, sub) -> uid mapping. IT MUST BE A VOLUME.
# Losing it does not log people out -- it silently makes every returning user a
# NEW user with a new, empty OreCloud store, which looks like data loss and is
# not recoverable from the providers.
RUN useradd --system --uid 10001 kenjin \
 && mkdir -p /data && chown kenjin:kenjin /data
VOLUME ["/data"]
USER kenjin

ENV DB_PATH=/data/auth.db \
    PORT=8000
EXPOSE 8000

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD python -c "import urllib.request,sys; sys.exit(0 if urllib.request.urlopen('http://127.0.0.1:8000/auth/health',timeout=4).status==200 else 1)"

# One worker: the sqlite handle and the JWKS cache are per-process, and a
# second worker would double both for no gain at sign-in volume. Litestream
# supervises it -- see entrypoint.sh.
#
# The entrypoint is made executable at COPY time, not with a RUN below: by this
# point USER has dropped to kenjin, who cannot chmod a root-owned file in
# /usr/local/bin.
CMD ["/usr/local/bin/entrypoint.sh"]
