FROM rust:1.64

WORKDIR /usr/src/warmer

COPY . .

RUN cargo -V && cargo build --release \
    && cp target/release/warmer /usr/bin/warmer && cp sitemap.xml /usr/bin/sitemap.xml

CMD ["/usr/bin/warmer"]