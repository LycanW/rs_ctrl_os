# AGENTS.md

## Cursor Cloud specific instructions

### Overview
`rs_ctrl_os` is a Rust library crate for building distributed node control systems. It provides UDP multicast node discovery, ZeroMQ pub/sub messaging, TOML config hot-reload, and time synchronization. See `README.md` for full API reference.

### System dependencies
- **`libzmq3-dev`** and **`pkg-config`** must be installed (`sudo apt-get install -y libzmq3-dev pkg-config`).
- **`libstdc++.so` symlink**: The default `cc` linker (clang) on this VM cannot find `-lstdc++` because only `libstdc++.so.6` exists. A symlink must exist: `sudo ln -sf /usr/lib/x86_64-linux-gnu/libstdc++.so.6 /usr/lib/x86_64-linux-gnu/libstdc++.so`.
- **Rust toolchain**: The `time` crate dependency requires Rust 1.85+. Run `rustup default stable` to ensure the latest stable is active (the VM may default to an older version).

### Build gotcha: CXX environment variable
The `zmq-sys` crate compiles ZeroMQ from C++ source. The default `c++` on this VM is clang, which cannot find C++ standard headers. Set `CXX=g++` for all cargo commands:
```bash
CXX=g++ cargo build
CXX=g++ cargo test
CXX=g++ cargo clippy
CXX=g++ cargo run --example pub_node -- pub_config.toml
```

### Running examples
See `README.md` for full instructions. Key examples:
- **Single pub/sub**: `cargo run --example pub_node -- example_config.toml`
- **Multi-process pair**: run `pub_node` with `pub_config.toml` and `multi_sub_node` with `multi_sub_config.toml` in separate terminals.
- **Multi-topic**: run `multi_pub_node` with `multi_pub_config.toml` and `multi_sub_node` with `multi_sub_config.toml`.

### Tests
- `cargo test --lib` runs unit tests (currently 0 tests defined in the crate).
- `cargo test --doc` runs doc-tests; 2 pre-existing failures exist in `src/config.rs` doc examples (undefined type `MyDynamicConfig`, missing `?` return types). These are repository issues, not environment issues.

### Lint
- `cargo clippy` produces warnings only (no errors). Warnings are pre-existing.
