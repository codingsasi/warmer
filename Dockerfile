FROM rust:1.71-alpine

WORKDIR /usr/src/warmer

COPY target/release/warmer /usr/bin/warmer

CMD ["/usr/bin/warmer"]