# syntax=docker/dockerfile:1.7
# Build context: repository root (`docker build -f Dockerfile .`).

FROM node:22-bookworm-slim AS web-build
WORKDIR /src/web
RUN corepack enable && corepack prepare pnpm@11.13.1 --activate
COPY web/package.json web/pnpm-lock.yaml web/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm build

FROM rust:1.98-bookworm AS rust-build
WORKDIR /src
COPY . ./
COPY --from=web-build /src/web/dist web/dist
RUN cargo build --release --locked -p BeanAgent --features ui

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 beanagent \
    && useradd --system --uid 10001 --gid beanagent --home-dir /var/lib/beanagent --create-home beanagent
COPY --from=rust-build /src/target/release/BeanAgent /usr/local/bin/BeanAgent
RUN install -d -o 10001 -g 10001 -m 0700 /var/lib/beanagent /srv/beanagent/workspace
WORKDIR /srv/beanagent
USER 10001:10001
ENV RUST_LOG=info,beanagent_security=warn
EXPOSE 7878
VOLUME ["/var/lib/beanagent", "/srv/beanagent/workspace"]
ENTRYPOINT ["/usr/local/bin/BeanAgent"]
CMD ["--config", "/etc/beanagent/BeanAgent.toml", "serve"]
