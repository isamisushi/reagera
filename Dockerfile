# Build the examples to WebAssembly, then serve www/ as static files.
FROM rust:1-slim-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates git \
    && rm -rf /var/lib/apt/lists/*
RUN rustup target add wasm32-unknown-unknown
RUN curl -sSL https://github.com/rustwasm/wasm-bindgen/releases/download/0.2.130/wasm-bindgen-0.2.130-x86_64-unknown-linux-musl.tar.gz \
    | tar xz -C /usr/local/bin --strip-components=1 wasm-bindgen-0.2.130-x86_64-unknown-linux-musl/wasm-bindgen
WORKDIR /app
COPY Cargo.toml Cargo.lock build.sh ./
COPY .cargo ./.cargo
COPY reagera ./reagera
COPY examples ./examples
COPY www ./www
RUN ./build.sh

FROM nginx:alpine
COPY --from=build /app/www /usr/share/nginx/html
