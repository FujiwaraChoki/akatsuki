# syntax=docker/dockerfile:1
# akatsuki-api: Hono on Node, built from the repository root:
#   docker build -f apps/api/Dockerfile -t akatsuki-api .
# The runtime image holds one bundled file (no node_modules, no package manager), runs as a non-root user
# and writes nothing, so it can run with a read-only root filesystem. Secrets come from the env file at
# runtime (infra/api.env.example lists the names); none are baked into the image.

FROM node:26.7.0-alpine3.24@sha256:aadf416b2cdce311a8811ba3f0608a61b77dbf997500e2eafe781b51f6a0b019 AS build
WORKDIR /src
RUN npm install -g pnpm@12.11.2 --no-fund --no-audit --loglevel=error
# apps/api/Dockerfile.dockerignore keeps .env files, node_modules, Rust targets and VCS data out of the context.
COPY . .
RUN pnpm install --frozen-lockfile --filter "@akatsuki/api..." \
 && pnpm --filter @akatsuki/api run build

FROM node:26.7.0-alpine3.24@sha256:aadf416b2cdce311a8811ba3f0608a61b77dbf997500e2eafe781b51f6a0b019 AS runtime
# Drop the package managers: the service needs only the node binary.
RUN rm -rf /usr/local/lib/node_modules /usr/local/bin/npm /usr/local/bin/npx /usr/local/bin/corepack \
      /opt/yarn-* /usr/local/bin/yarn /usr/local/bin/yarnpkg
ENV NODE_ENV=production \
    BIND_ADDR=0.0.0.0:8787
WORKDIR /app
COPY --from=build --chown=root:root /src/apps/api/dist/ /app/
USER 1000:1000
EXPOSE 8787
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
  CMD wget -q -O /dev/null http://127.0.0.1:8787/healthz || exit 1
ENTRYPOINT ["node", "/app/server.mjs"]
