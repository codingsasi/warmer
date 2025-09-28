FROM rust:1.90-alpine

WORKDIR /usr/src/warmer

RUN apk update --no-cache; \
    apk upgrade --no-cache; \
    apk add --no-cache \
    openssl-dev \
    musl-dev \
    libgcc \
    openssl-libs-static \
    zlib-dev \
    zlib-static \
    git \
    # Chrome dependencies
    chromium \
    nss \
    freetype \
    freetype-dev \
    harfbuzz \
    ca-certificates \
    ttf-freefont \
    # Set environment variables for Chrome
    && export CHROME_BIN=/usr/bin/chromium-browser \
    && export CHROME_PATH=/usr/lib/chromium/

ENV CHROME_BIN=/usr/bin/chromium-browser
ENV CHROME_PATH=/usr/lib/chromium/

COPY . /usr/src/warmer
RUN cd /usr/src/warmer && cargo build --release \
    && cp target/release/warmer /usr/bin/warmer \
    && rm -rf /usr/src/warmer/* \
    && apk add --no-cache \
       openssl-dev \
       musl-dev \
       libgcc \
       openssl-libs-static \
       zlib-dev \
       zlib-static \
       git

CMD ["/usr/bin/warmer"]