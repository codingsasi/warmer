FROM rust:1.72

WORKDIR /usr/src/warmer

COPY . .

RUN cargo -V && cargo build --release \
    && cp target/release/warmer /usr/bin/warmer \
    && rm -rf /usr/src/warmer/*

CMD ["/usr/bin/warmer"]