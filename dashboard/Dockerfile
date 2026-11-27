# ARK dashboard (Rust server in server/, frontend served from disk). Static musl binary on Alpine.
# Build from this directory:   docker build --platform linux/amd64 -t ark-dashboard-rs .
# (a host without internet gets the image as a tar: see deploy/deploy.sh)
FROM docker.io/library/rust:1.99-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY server ./server
RUN cd server && cargo build --release --locked && strip target/release/ark-dashboard

FROM docker.io/library/alpine:3.22
COPY --from=build /src/server/target/release/ark-dashboard /usr/local/bin/ark-dashboard
COPY frontend /app/frontend
ENV ARK_FRONTEND_DIR=/app/frontend
EXPOSE 8787
USER 1000:1000
# /api/health sits behind the login; any HTTP answer (also 401) means the process is alive.
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
  CMD wget -q -O /dev/null --header="Authorization: Basic $(printf '%s:%s' "$DASH_USER" "$DASH_PASS" | base64 | tr -d '\n')" http://127.0.0.1:8787/api/health || exit 1
ENTRYPOINT ["/usr/local/bin/ark-dashboard"]
