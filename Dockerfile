# Deloist в контейнере: серверная сборка (`deloist --server`), без окна, GTK и WebKit.
#
# Сервер слушает только loopback (HOST другим адресом не принимается, это защита). Поэтому порт `-p` наружу
# не пробросить: запускай с `--network host` (тогда loopback контейнера и есть loopback хоста) или ходи
# внутрь через `docker exec`.
#   docker build -t deloist-server .
#   docker run --rm --network host -v deloist-data:/data deloist-server
#
# Сборка: `RUSTUP_TOOLCHAIN=stable` перебивает rust-toolchain.toml (там тулчейн для Windows).

FROM rust:1-slim-bookworm AS build
ENV RUSTUP_TOOLCHAIN=stable
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN cargo build --release --no-default-features --bin deloist \
 && cp target/release/deloist /deloist

# Moli (lexmount/moli, Apache-2.0): light cloud browser. Version and checksum are pinned, as in src/moli.rs.
FROM debian:bookworm-slim AS moli
ARG MOLI_VERSION=1.1.24
ARG MOLI_SHA256=09e54e48be914df9596e8b9046cd2e1c67d5911761a09b0445488dd5d39c56d6
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL -o /moli.tar.gz "https://github.com/lexmount/moli/releases/download/v${MOLI_VERSION}/moli-x86_64-unknown-linux-gnu.tar.gz" \
 && echo "${MOLI_SHA256}  /moli.tar.gz" | sha256sum -c - \
 && mkdir /opt/moli \
 && tar -xzf /moli.tar.gz -C /opt/moli --strip-components=1 \
 && test -x /opt/moli/moli

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates libssl3 chromium fonts-liberation \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --create-home --uid 10001 deloist \
 && mkdir /data && chown deloist:deloist /data
COPY --from=build /deloist /usr/local/bin/deloist
# LICENSE-APACHE and licenses/ from the release archive stay next to the binary.
COPY --from=moli /opt/moli /opt/moli
RUN /opt/moli/moli --version
USER deloist
ENV CREW_DATA_DIR=/data \
    HOME=/home/deloist \
    # Клетку Chromium даёт gVisor вокруг контейнера. CREW_SANDBOX=off не ставим: консоли у ботов в облаке нет (CREW_CLOUD=1), а без политики команду остановила бы песочница (fail-closed).
    CREW_BROWSER_NO_SANDBOX=1 \
    # Moli is baked into the image and never downloaded in the cloud. CREW_CLOUD=1 (Moli by default) is set by the gateway, not here.
    CREW_MOLI_PATH=/opt/moli/moli
VOLUME /data
CMD ["deloist", "--server"]
