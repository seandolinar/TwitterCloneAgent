FROM rust:1.99.1-bookworm AS api
WORKDIR /build
COPY apps/api/Cargo.toml apps/api/Cargo.lock apps/api/build.rs ./
COPY apps/api/src ./src
COPY apps/api/migrations ./migrations
RUN cargo build --release --locked --bin twitter-clone-api --jobs 2

FROM node:24.18.1-bookworm-slim AS web
WORKDIR /build
COPY apps/web/package.json apps/web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY apps/web/tsconfig.json apps/web/vite.config.ts ./
COPY apps/web/src ./src
COPY apps/web/server ./server
COPY apps/web/public ./public
RUN npm run build && npm run typecheck

FROM node:24.18.1-bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
ENV NODE_ENV=production APP_ENV=production WEB_RUNTIME_MODE=compute \
    HOST=0.0.0.0 PORT=8080 BROWSER_BIND=127.0.0.1:8081 GATEWAY_BIND=127.0.0.1:8082 \
    API_BASE_URL=http://127.0.0.1:8081 GATEWAY_API_BASE_URL=http://127.0.0.1:8082
WORKDIR /app
COPY --from=api /build/target/release/twitter-clone-api /usr/local/bin/twitter-clone-api
COPY --from=web --chown=node:node /build/.output ./.output
COPY --chown=node:node infra/compute/entrypoint.mjs ./entrypoint.mjs
USER node
EXPOSE 8080
STOPSIGNAL SIGTERM
ENTRYPOINT ["node", "/app/entrypoint.mjs"]
