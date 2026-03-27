FROM rust:1.93.1

RUN rustup component add rustfmt clippy \
    && cargo install cargo-nextest --version 0.9.131 --locked
