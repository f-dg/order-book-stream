### Overview
A simple order book stream service that combines Binance and Bitstamp streams and proxies to clients.
Design with performance in mind while trade off extensibility of the code:
- `tokio::sync::broadcast` is used to deliver order book to clients. It doesn't care about slow clients which implies data losses if a client falls behind
- `BTreeMap` is used for storing order book: a simple and efficient enough data structure
- no easy way to add multiticker supporting

### RUN
```
# run a server
RUST_LOG=info cargo run --bin order_book_stream -- -t ETHBTC

# run a client to receive order book updates
cargo run --bin grpc_client
```
### Non Cargo Dependencies:
  - brew install protobuf
