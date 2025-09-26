FROM rust:1.90-alpine

WORKDIR /usr/src/warmer

RUN apk update --no-cache; \
    apk upgrade --no-cache; \
    apk add --no-cache \
    openssl-dev \
    musl-dev \
    libgcc \
    openssl-libs-static \
    git

RUN cd /usr/src/ && git clone https://github.com/codingsasi/warmer.git  \
    && cd warmer && cargo build --release \
    && cp target/release/warmer /usr/bin/warmer \
    && rm -rf /usr/src/warmer/* \
    && apk add --no-cache \
       openssl-dev \
       musl-dev \
       libgcc \
       openssl-libs-static \
       git

CMD ["/usr/bin/warmer"]