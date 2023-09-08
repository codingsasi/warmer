FROM rust:1.72-alpine

WORKDIR /usr/src/warmer

RUN apk update; \
    apk upgrade;

RUN apk add --no-cache \
    openssl-dev \
    musl-dev \
    libgcc \
    openssl-libs-static


COPY . .

RUN cargo -V && cargo build --release \
    && cp target/release/warmer /usr/bin/warmer \
    && rm -rf /usr/src/warmer/*

CMD ["/usr/bin/warmer"]