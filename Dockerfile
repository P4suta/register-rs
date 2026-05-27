# Dev image for despeckle. Ships every tool the justfile recipes invoke,
# so host machines need nothing beyond Docker.
#
# Speed-tuning policy:
#   - cargo-binstall pulls prebuilt binaries (cuts ~20 min → ~30 s).
#   - mold linker via clang shaves 30–70% off link time.
#   - BuildKit cache mounts retain cargo registry/index between builds.

# syntax=docker/dockerfile:1.7

FROM rust:1.95-bookworm AS dev

ARG USER_UID=1000
ARG USER_GID=1000

ENV DEBIAN_FRONTEND=noninteractive \
    CARGO_HOME=/usr/local/cargo \
    RUSTUP_HOME=/usr/local/rustup \
    PATH=/usr/local/cargo/bin:/usr/local/rustup/bin:$PATH

# Base packages: build tooling, mold linker, graphviz for `cargo depgraph` → SVG.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        clang \
        cmake \
        curl \
        git \
        gnupg \
        graphviz \
        lld \
        mold \
        pkg-config \
        sudo \
        unzip \
    && rm -rf /var/lib/apt/lists/*

# Rust toolchain extras.
RUN rustup component add rustfmt clippy rust-src

# cargo-binstall: single-binary release, downloads prebuilt binaries for
# subsequent cargo subcommands.
RUN curl -fsSL https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh \
    | bash

# Pass GITHUB_TOKEN via BuildKit secret to avoid the unauthenticated
# api.github.com 60/hr rate-limit:
#   GITHUB_TOKEN=$(gh auth token) docker compose build
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=secret,id=github-token,required=false \
    GITHUB_TOKEN="$(cat /run/secrets/github-token 2>/dev/null || true)" \
    cargo binstall --no-confirm --no-symlinks \
        cargo-deny \
        cargo-audit \
        cargo-llvm-cov \
        cargo-nextest \
        cargo-machete \
        cargo-sort \
        cargo-rdme \
        cargo-modules \
        cargo-depgraph \
        just \
        taplo-cli \
        typos-cli

# actionlint (binary release).
RUN curl -fsSL https://raw.githubusercontent.com/rhysd/actionlint/main/scripts/download-actionlint.bash \
    | bash -s -- latest /usr/local/bin

# lefthook (.deb release). Resolve the latest tag via the GitHub API; auth via
# GITHUB_TOKEN secret to avoid the unauthenticated 60/hr api.github.com cliff.
RUN --mount=type=secret,id=github-token,required=false bash -eu -c '\
    arch="$(dpkg --print-architecture)"; \
    token="$(cat /run/secrets/github-token 2>/dev/null || true)"; \
    if [ -n "$token" ]; then \
        version_json="$(curl -fsSL -H "Authorization: Bearer $token" https://api.github.com/repos/evilmartians/lefthook/releases/latest)"; \
    else \
        version_json="$(curl -fsSL https://api.github.com/repos/evilmartians/lefthook/releases/latest)"; \
    fi; \
    version="$(echo "$version_json" | sed -n "s/.*\"tag_name\": *\"v\([^\"]*\)\".*/\1/p")"; \
    test -n "$version" || { echo "ERROR: could not resolve lefthook latest tag (api.github.com rate-limited?)" >&2; exit 1; }; \
    echo "lefthook: v$version"; \
    curl -fsSL -o /tmp/lefthook.deb "https://github.com/evilmartians/lefthook/releases/download/v${version}/lefthook_${version}_${arch}.deb"; \
    dpkg -i /tmp/lefthook.deb; \
    rm /tmp/lefthook.deb \
'

# biome (Rust-backed formatter, JSON). Single binary, no Node required.
RUN ARCH="$(dpkg --print-architecture)" \
    && case "$ARCH" in \
        amd64) BIOME_ARCH="linux-x64" ;; \
        arm64) BIOME_ARCH="linux-arm64" ;; \
        *) echo "unsupported arch: $ARCH" >&2 && exit 1 ;; \
    esac \
    && curl -fsSL "https://github.com/biomejs/biome/releases/latest/download/biome-${BIOME_ARCH}" \
        -o /usr/local/bin/biome \
    && chmod +x /usr/local/bin/biome

# yamlfmt (Go single binary).
RUN ARCH="$(dpkg --print-architecture)" \
    && case "$ARCH" in \
        amd64) YAMLFMT_ARCH="Linux_x86_64" ;; \
        arm64) YAMLFMT_ARCH="Linux_arm64" ;; \
        *) echo "unsupported arch: $ARCH" >&2 && exit 1 ;; \
    esac \
    && YAMLFMT_VERSION="0.13.0" \
    && curl -fsSL "https://github.com/google/yamlfmt/releases/download/v${YAMLFMT_VERSION}/yamlfmt_${YAMLFMT_VERSION}_${YAMLFMT_ARCH}.tar.gz" \
        | tar xz -C /usr/local/bin yamlfmt

# Match host UID so bind-mounted files don't end up root-owned.
RUN groupadd --gid ${USER_GID} dev \
    && useradd --uid ${USER_UID} --gid ${USER_GID} -m dev \
    && echo "dev ALL=(ALL) NOPASSWD:ALL" >> /etc/sudoers \
    && chown -R dev:dev /usr/local/cargo /usr/local/rustup

USER dev
ENV INSIDE_CONTAINER=1

WORKDIR /workspace
CMD ["bash"]
