# rs_ctrl_os

`rs_ctrl_os` 是一个用于构建分布式节点控制系统的小型运行时库，提供：

- **节点发现**：基于 UDP 多播的心跳机制（`Heartbeat` + `ServiceRegistry`）
- **消息通信**：基于 ZeroMQ 的 pub/sub 抽象（`PubSubManager`）
- **配置管理**：TOML 配置加载 + 动态热更新（`ConfigManager`）
- **时间同步**：简单的主从时钟同步（`TimeSynchronizer`）
- **统一错误/日志**：`RsCtrlError` + `tracing` 日志初始化

适合需要在局域网内跑多进程/多节点，进行“互相发现 + 消息分发 + 动态配置”的系统。

---

## 使用教程（一步一步）

### 0. 准备环境

- 安装 ZeroMQ 库（不同平台命令略有差异，以下是常见示例）：
  - Debian/Ubuntu: `sudo apt-get install libzmq3-dev`
  - Fedora: `sudo dnf install zeromq zeromq-devel`
  - macOS（Homebrew）: `brew install zeromq`
- 安装 Rust 稳定版（建议用 `rustup`）。

克隆本项目：

```bash
git clone https://github.com/yourname/rs_ctrl_os.git
cd rs_ctrl_os
```

或者使用cargo

```bash
cargo add rs_ctrl_os
```

### 1. 跑一个最简单的 pub/sub 示例

准备一个最小的配置（你也可以用仓库里的 `example_config.toml`）：

```toml
[static_config]
my_id = "node1"
host = "127.0.0.1"
port = 5555
is_master = true

[static_config.subscribers]
local_sub = "node1"

[static_config.publishers]
control = "self"

[dynamic]
message_prefix = "hello"
interval_ms = 200
```

在项目根目录运行：

```bash
cargo run --example pub_node -- example_config.toml
```

你会看到终端持续打印收到的消息。此时 pub 和 sub 都在同一个进程里：

- `control` topic 上不断发布字符串消息；
- `local_sub` 订阅自己发出的数据并打印。

如果你改动 `example_config.toml` 里的 `[dynamic]`（比如改前缀、改间隔）并保存，进程会自动加载新的动态配置：

- `message_prefix` 会改变打印出来的文本前缀；
- `interval_ms` 会改变发送/接收的频率。

### 2. 跑两个进程：一个 pub，一个 sub

有时候你想在两个不同的进程里测试 pub/sub。可以用仓库里的：

- `examples/pub_node.rs`
- `examples/sub_node.rs`

以及对应的：

- `pub_config.toml`
- `sub_config.toml`

先打开一个终端作为发布端：

```bash
cargo run --example pub_node -- pub_config.toml
```

再打开另一个终端作为订阅端：

```bash
cargo run --example sub_node -- sub_config.toml
```

此时：

- `pub_node` 会按照 `pub_config.toml` 的 `[dynamic]` 配置，持续往 ZeroMQ PUB socket 上发消息。
- `sub_node` 通过 UDP 多播发现 `pub_node`，连接上它的 PUB socket，然后不断从 `local_sub` 这个订阅名收消息并打印。

你可以动态修改 `pub_config.toml` 的 `[dynamic]`，比如：

```toml
[dynamic]
message_prefix = "pub1"
interval_ms = 200
```

改成：

```toml
[dynamic]
message_prefix = "PUB-UPDATED"
interval_ms = 1000
```

保存之后，几百毫秒到一两秒内你会看到：

- `sub_node` 打印出的消息前缀从 `pub1` 变成 `PUB-UPDATED`；
- 输出频率从 200ms 一条变成大约 1 秒一条。

---

## 功能概览

- **配置管理（ConfigManager）**
  - 从一个包含 `[static_config]` 和 `[dynamic]` 的 TOML 文件加载配置。
  - 静态配置：`StaticBase`（节点 ID、host、port、是否 master、订阅/发布拓扑）。
  - 动态配置：任意 `D: Deserialize + Clone`，通过文件监听自动热更新。
- **节点发现（start_discovery + ServiceRegistry）**
  - 使用 UDP 多播地址 `224.0.0.100:9999` 定期发送/接收 `Heartbeat`。
  - 自动维护一个节点注册表 `ServiceRegistry`，可以通过 `get_address(node_id)` 获取对方地址。
- **ZeroMQ Pub/Sub（PubSubManager）**
  - 按 `StaticBase.publishers` 在本地绑定 PUB socket。
  - 按 `StaticBase.subscribers` 动态连接到其他节点的 PUB。
  - 消息格式为 **三帧 multipart**：
    1. 节点 ID（`my_id`，UTF‑8 字节）
    2. 子话题（`sub_topic`，UTF‑8 字节）
    3. 业务 payload（`bincode` 序列化的任意 `T: Serialize`）
- **时间同步（TimeSynchronizer）**
  - 通过 master 心跳中的 `clock_time_ms` 与本地时间对比，估算偏移。
  - 提供 `now_corrected_ms()` 作为“粗略对齐后的集群时间”。

---

## 安装

在你的 `Cargo.toml` 中添加依赖：

```toml
[dependencies]
rs_ctrl_os = "0.1"
```

你也可以通过路径依赖 / git 依赖方式在本地使用：

```toml
[dependencies]
rs_ctrl_os = { path = "./rs_ctrl_os" }
```

---

## 快速上手

下面是一个简单的单进程 pub/sub 示例，展示主要 API 的使用方式。

### 1. 初始化日志

```rust
use rs_ctrl_os::init_logging;

fn main() {
    init_logging();
    // ...
}
```

### 2. 从 TOML 加载配置（静态 + 动态）

```toml
# example_config.toml
[static_config]
my_id = "node1"
host = "127.0.0.1"
port = 5555
is_master = true

[static_config.subscribers]
local_sub = "node1"

[static_config.publishers]
control = "self"

[dynamic]
message_prefix = "hello"
```

```rust
use std::path::Path;
use serde::Deserialize;
use rs_ctrl_os::ConfigManager;

#[derive(Clone, Deserialize)]
struct DynamicCfg {
    message_prefix: String,
    publish_hz: u64,
    subscribe_hz: u64,
}

fn main() -> rs_ctrl_os::Result<()> {
    init_logging();

    let manager: ConfigManager<DynamicCfg> =
        ConfigManager::new(Path::new("example_config.toml"))?;
    let static_cfg = manager.static_cfg().clone();

    // 后面可以随时通过 manager.get_dynamic_clone() 获取最新动态配置
    Ok(())
}
```

### 3. 启动节点发现 + 时间同步

```rust
use std::sync::Arc;
use rs_ctrl_os::{start_discovery, TimeSynchronizer};

fn main() -> rs_ctrl_os::Result<()> {
    init_logging();
    // ... 加载配置

    let time_sync = Arc::new(TimeSynchronizer::new());

    let registry = start_discovery(
        &static_cfg.my_id,
        &static_cfg.host,
        static_cfg.port,
        static_cfg.is_master,
        Some(time_sync.clone()),
    )?;

    // registry 会在后台线程持续更新
    Ok(())
}
```

### 4. 创建 Pub/Sub 管理器并发送消息

```rust
use rs_ctrl_os::PubSubManager;
use std::thread;
use std::time::Duration;

fn main() -> rs_ctrl_os::Result<()> {
    init_logging();
    // ... 加载配置 + start_discovery

    let bus = PubSubManager::new(&static_cfg, registry)?;

        loop {
        let dyn_cfg = manager.get_dynamic_clone();
        let ts_ms = time_sync.now_corrected_ms();

        let payload = format!(
            "{} from {} at {} ms",
            dyn_cfg.message_prefix, static_cfg.my_id, ts_ms
        );

        // topic_key = "control"，sub_topic = "demo"
        bus.publish_topic("control", "demo", &payload)?;

        if let Some(received) = bus.try_recv_specific::<String>("local_sub", "demo")? {
            println!("Received: {received}");
        }

        // 简单示例：按 subscribe_hz 驱动主循环节奏
        let interval = if dyn_cfg.subscribe_hz > 0 {
            Duration::from_secs_f64(1.0 / dyn_cfg.subscribe_hz as f64)
        } else {
            Duration::from_millis(100)
        };
        thread::sleep(interval);
    }
}
```

---

## 示例（examples）

仓库中包含几个可运行的示例，建议阅读并直接运行：

- `examples/pub_node.rs` / `examples/sub_node.rs`  
  单 pub + 单 sub，带动态 TOML 和时间戳。
- `examples/multi_pub_node.rs` / `examples/multi_sub_node.rs`  
  单进程 multi_pub（多个子话题）+ 单进程 multi_sub（多流订阅），演示如何在一个 socket 上区分多种业务流。
- `examples/raw_sub_node.rs`  
  展示如何用 `try_recv_raw` 收到 **原始二进制 payload**，并打印十六进制 + 反序列化后的字符串。

运行示例（在项目根目录）：

```bash
# 简单 pub/sub
cargo run --example pub_node -- example_config.toml

# 多 pub / 多 sub
cargo run --example multi_pub_node -- multi_pub_config.toml
cargo run --example multi_sub_node -- multi_sub_config.toml
```

---

## 错误处理

库统一使用：

```rust
use rs_ctrl_os::{Result, RsCtrlError};
```

`RsCtrlError` 覆盖了：

- 配置错误：`Config(String)`
- 通信错误：`Comms(String)`
- 发现错误：`Discovery(String)`
- IO 错误：`Io(std::io::Error)`
- ZeroMQ 错误：`Zmq(zmq::Error)`
- Bincode 序列化错误：`Bincode(Box<bincode::ErrorKind>)`

绝大多数 API 都返回 `rs_ctrl_os::Result<T>`，便于在上层直接用 `?` 传播。

---

## 许可证

本项目采用 **MIT** 许可证发布。  
详见 `LICENSE-MIT` 文件。

