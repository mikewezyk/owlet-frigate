# syntax=docker/dockerfile:1
# Base images are pinned by digest (looked up 2026-11-24). Update them on purpose.
FROM rust:1-slim-trixie@sha256:51fdc80ccc3fb83bf14481f2cd8d0d11703eeb9f416ab47476c32a793a822307 AS build
RUN apt-get update \
 && apt-get install -y --no-install-recommends libssl-dev pkg-config \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY owlet-tutk/ ./
RUN cargo test --locked --release --lib --bins && cargo build --locked --release

FROM debian:trixie-slim@sha256:8aa5a7bf5a1955bddbe7d74fcdd0b91ab30e74362a8b73af672b0481b35b44a0 AS go2rtc
ARG TARGETARCH
ADD --chmod=755 https://github.com/AlexxIT/go2rtc/releases/download/v1.9.15/go2rtc_linux_${TARGETARCH} /go2rtc
RUN case "${TARGETARCH}" in \
      amd64) sum=20b46c6910734cb05ecefb234066f36868ff19b42ba1418a7a32e13a3e035712 ;; \
      arm64) sum=af416f22b21a24ab83b8d78642190b4932ae695b4e386853f11d1b0319781763 ;; \
      *) echo "unsupported architecture ${TARGETARCH}" >&2; exit 1 ;; \
    esac; echo "${sum}  /go2rtc" | sha256sum -c -

FROM debian:trixie-slim@sha256:8aa5a7bf5a1955bddbe7d74fcdd0b91ab30e74362a8b73af672b0481b35b44a0
RUN apt-get update \
 && apt-get install -y --no-install-recommends libssl3t64 \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 owlet \
 && install -d -o owlet -g owlet /state
COPY --from=build /build/target/release/owlet-tutk /usr/local/bin/
COPY --from=go2rtc /go2rtc /usr/local/bin/
COPY deploy/go2rtc.yaml /etc/go2rtc.yaml
USER owlet
VOLUME /state
ENTRYPOINT ["go2rtc", "-config", "/etc/go2rtc.yaml"]
