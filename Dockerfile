# Image de tonapi-server, avec la CLI `tonapi` (keygen, validate) et la configuration des
# universités. Construction : `docker build -t tonapi .` depuis la racine du dépôt.
# Exploitation (variables, sondes, arrêt) : Docs/exploitation.md.
#
# Les images de base sont désignées par leur version ; les épingler aussi par empreinte
# (`@sha256:…`) dans la chaîne de livraison.

# Même version que rust-toolchain.toml.
FROM rust:1.98.1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
# Sans `RUN --mount` : l'image se construit aussi avec l'ancien constructeur (Docker sans buildx).
RUN cargo build --release --locked -p tonapi-server -p tonapi-cli \
    && mkdir -p /out \
    && cp target/release/tonapi-server target/release/tonapi /out/
COPY universities universities
# Une configuration refusée ne doit pas devenir une image.
RUN /out/tonapi validate universities

FROM debian:bookworm-slim
# native-tls : OpenSSL du système (certains CAS n'acceptent que des suites que rustls refuse).
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin tonapi
COPY --from=build /out/tonapi-server /out/tonapi /usr/local/bin/
COPY --from=build /src/universities /app/universities
WORKDIR /app
ENV TONAPI_ADDR=0.0.0.0:8080 \
    TONAPI_UNIVERSITIES=/app/universities \
    TONAPI_LOG_FORMAT=json
USER 10001
# 8080 : API. Métriques sur un port interne seulement si TONAPI_METRICS_ADDR est défini
# (par exemple 0.0.0.0:9090, à ne pas publier hors du réseau de supervision) ; hors boucle locale,
# TONAPI_METRICS_TOKEN (32 caractères au moins) est obligatoire et le serveur refuse de démarrer sans.
EXPOSE 8080
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/tonapi-server"]
