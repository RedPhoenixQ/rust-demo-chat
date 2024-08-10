FROM rust:1-slim-buster AS build
RUN cargo new --bin app
WORKDIR /app
COPY . /app/.
RUN cargo build --release

FROM debian:buster-slim  
RUN apt-get update
RUN apt-get install curl -y
COPY --from=build /app/target/release/rust-demo-chat /app/main
COPY assets /app/assets
EXPOSE 3000
CMD "/app/main"