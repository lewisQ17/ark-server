FROM node:22-alpine
WORKDIR /app
# backend deps (lockfile, geen dev-dependencies)
COPY backend/package*.json ./backend/
RUN cd backend && npm ci --omit=dev --no-audit --no-fund
# app
COPY backend ./backend
COPY frontend ./frontend
ENV NODE_ENV=production
EXPOSE 8787
USER node
WORKDIR /app/backend
# /api/health zit achter Basic auth; elke HTTP-reactie (ook 401) betekent: proces leeft.
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
  CMD wget -q -O /dev/null --header="Authorization: Basic $(printf '%s:%s' "$DASH_USER" "$DASH_PASS" | base64 | tr -d '\n')" http://127.0.0.1:8787/api/health || exit 1
CMD ["node", "server.js"]
