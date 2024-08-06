## How to run it


As **Master**

```sh
RUST_LOG="info" cargo r --release -- master --host ws://127.0.0.1:9001 -r <rpc_url> -p <priority_fees>
```

As a **Node**
```sh
RUST_LOG="info" cargo r --release -- node --master ws://127.0.0.1:9001 -r <rpc_url>
```