# syntax=docker/dockerfile:1.7

ARG UBUNTU_VERSION=24.04
FROM mcr.microsoft.com/devcontainers/base:ubuntu-${UBUNTU_VERSION}

ARG DEBIAN_FRONTEND=noninteractive
ARG USERNAME=vscode
ARG NODE_MAJOR=22
ARG RUST_VERSION=1.93.1
ARG CODEX_NPM_PACKAGE=@openai/codex
ARG CODEX_VERSION=0.115.0

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        git \
        gpg \
        libssl-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir -p /etc/apt/keyrings \
    && curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key \
        | gpg --dearmor -o /etc/apt/keyrings/nodesource.gpg \
    && echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_${NODE_MAJOR}.x nodistro main" \
        > /etc/apt/sources.list.d/nodesource.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

RUN npm install --global "${CODEX_NPM_PACKAGE}@${CODEX_VERSION}" \
    && npm cache clean --force

USER ${USERNAME}

ENV CARGO_HOME=/home/${USERNAME}/.cargo
ENV RUSTUP_HOME=/home/${USERNAME}/.rustup
ENV PATH=/home/${USERNAME}/.cargo/bin:${PATH}

RUN curl https://sh.rustup.rs -sSf \
        | sh -s -- -y --profile minimal --default-toolchain ${RUST_VERSION} \
    && rustup component add clippy rustfmt

WORKDIR /workspaces/nao
