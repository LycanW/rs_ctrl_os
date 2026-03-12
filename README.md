# rs_ctrl_os

`rs_ctrl_os` 是一个用于构建分布式节点控制系统的小型运行时库，提供：

- **节点发现**：基于 UDP 多播的心跳机制（`Heartbeat` + `ServiceRegistry`）
- **消息通信**：基于 ZeroMQ 的 pub/sub 抽象（`PubSubManager`）
- **配置管理**：TOML 配置加载 + 动态热更新（`ConfigManager` / `load_config_typed`）
- **时间同步**：简单的主从时钟同步（`TimeSynchronizer`）
- **统一错误/日志**：`RsCtrlError` + `tracing` 日志初始化

适合需要在局域网内跑多进程/多节点，进行“互相发现 + 消息分发 + 动态配置”的系统。

---

## 框架能力与边界

### 框架负责什么

| 能力 | 说明 |
|------|------|
| **StaticBase 与 [static_config]** | 节点 ID、host、port、是否 master、publishers/subscribers 拓扑、static_nodes（IP fallback）、publish_hz/subscribe_hz、dynamic_load_enable 等**运行所需的基础配置** |
| **配置加载机制** | 从 TOML 解析 `[static_config]`，提供 `load_config_rcos`、`load_config_typed`、`ConfigManager` 等 API |
| **dynamic 热更新** | 当 `dynamic_load_enable=true` 时，监听配置文件变化并热重载 `[dynamic]` 内容 |
| **消息通道** | ZMQ pub/sub、发现、时间同步、频率限速、原始字节透传（`publish_raw` / `try_recv_raw`） |

### 框架不负责什么

| 边界 | 说明 |
|------|------|
| **[dynamic] 的结构与语义** | 框架只负责「加载并热更新」`[dynamic]`，**不定义**其字段。每个应用自行定义 `D: Deserialize`，例如 CAN 接口列表、电机参数、相机参数等 |
| **业务数据内容** | 图像、点云等大体量数据应通过 topic 传输，**不应**塞进 TOML。配置中只放「如何连接、参数、schema 版本」等元信息 |
| **业务协议与编码** | 消息 payload 的序列化方式（bincode / raw / JPEG 等）由应用选择；框架提供 `publish_topic`（bincode）、`publish_raw`（透传）两种能力 |

### 如何区分框架配置与业务配置

- **`[static_config]`**：框架强依赖，**必须存在**。包含 `my_id`、`host`、`port`、`publish_hz`、`subscribe_hz`、`dynamic_load_enable`、`publishers`、`subscribers`、`static_nodes`（可选）等。
- **`[dynamic]`**：业务自由定义。框架不解析其具体字段，只负责按你提供的 `D` 反序列化并（可选）热更新。  
  例如：can_bridge 定义 `interfaces`、`devices`；相机节点定义 `camera_id`、`resolution`；点云节点定义 `voxel_size` 等。

### 框架在背后完成的工作

以下能力由框架自动完成，应用通常无需关心实现细节：

| 模块 | 后台行为 |
|------|----------|
| **节点发现** | 启动两个线程：发送端每 1 秒向 `224.0.0.100:9999` 广播本节点 `Heartbeat`；接收端持续收取其它节点心跳，更新 `ServiceRegistry`，超过 10 秒未收到则从注册表剔除。 |
| **时间同步** | 接收端收到 `is_master=true` 的心跳时，提取其 `clock_time_ms`，计算本地与 master 的时钟偏移并低通滤波。`now_corrected_ms()` 内部使用该偏移修正当前时间。 |
| **ConfigManager 热更新** | 当 `dynamic_load_enable=true` 时，通过 `notify` 监听配置文件。文件变化时自动重读并解析 `[dynamic]`，更新内部 `RwLock`，`get_dynamic_clone()` 返回最新值。 |
| **发布频率控制** | 当 `publish_hz > 0` 时，`publish_topic` / `publish_raw` 内部按 `topic_key` 记录上次发送时间，超过最小间隔的请求会被静默丢弃（限频）。 |
| **订阅频率控制** | 当 `subscribe_hz > 0` 时，`try_recv_raw` / `try_recv_specific` 内部按 `local_name` 记录上次轮询时间，未到间隔则直接返回 `None`，避免过度轮询。 |
| **订阅连接建立** | 初始化时，若 discovery 和 `static_nodes` 均未提供目标地址，订阅进入 `pending_subs`。`try_recv_raw` 内部自动 tick()，优先从 `ServiceRegistry` 解析，其次从 `static_nodes`（`node_id -> "host:port"`）fallback，**无需手动调用 tick()**。 |
| **子话题过滤** | 若通过 `set_sub_topics` 设置了白名单，框架在 `try_recv_raw` 中只返回白名单内的 `sub_topic`，其它消息静默丢弃。 |

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
publish_hz = 1000
subscribe_hz = 1000
dynamic_load_enable = true

[static_config.subscribers]
local_sub = "node1"

[static_config.publishers]
control = "self"

# 可选：当 discovery 未找到目标时，用此地址直连（适合无多播环境）
[static_config.static_nodes]
# node1 = "127.0.0.1:5555"

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

- **配置管理**
  - **`load_config_rcos(path)`**：返回 `(StaticBase, toml::Value)`，框架解析 `[static_config]`，`[dynamic]` 以原始 `toml::Value` 返回，由应用自行反序列化。
  - **`load_config_typed::<D>(path)`**：返回 `(StaticBase, D)`，一次性加载，无热更新，适合无需动态重载的场景。
  - **`ConfigManager<D>`**：加载 `[static_config]` + `[dynamic]`，当 `StaticBase.dynamic_load_enable=true`（默认）时监听文件变化并热重载 `[dynamic]`。
  - **`StaticBase`**：节点 ID、host、port、is_master、publishers/subscribers、static_nodes（IP fallback）、publish_hz、subscribe_hz、dynamic_load_enable 等框架必需字段。
- **节点发现（start_discovery + ServiceRegistry）**
  - 使用 UDP 多播地址 `224.0.0.100:9999` 定期发送/接收 `Heartbeat`。
  - 自动维护一个节点注册表 `ServiceRegistry`，可以通过 `get_address(node_id)` 获取对方地址。
- **ZeroMQ Pub/Sub（PubSubManager）**
  - 按 `StaticBase.publishers` 在本地绑定 PUB socket，按 `StaticBase.subscribers` 动态连接其他节点。
  - **`publish_topic`**：bincode 序列化，适合结构化小消息（控制指令、状态等）。
  - **`publish_raw`**：透传原始字节，适合图像、点云等已编码二进制，不经过 serde。
  - **`try_recv_raw`**：返回 `(sub_topic, Vec<u8>)`，由应用自行解析。
  - **`try_recv_specific`**：将 payload 反序列化为指定类型（bincode）。
  - 消息格式为 **三帧 multipart**：`[节点 ID, sub_topic, payload]`。
- **时间同步（TimeSynchronizer）**
  - 通过 master 心跳中的 `clock_time_ms` 与本地时间对比，估算偏移，提供 `now_corrected_ms()`。

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
publish_hz = 1000
subscribe_hz = 1000
dynamic_load_enable = true

[static_config.subscribers]
local_sub = "node1"

[static_config.publishers]
control = "self"

[dynamic]
message_prefix = "hello"
```

**方式一：需要热重载时，用 ConfigManager**

```rust
use std::path::Path;
use serde::Deserialize;
use rs_ctrl_os::ConfigManager;

#[derive(Clone, Deserialize)]
struct DynamicCfg {
    message_prefix: String,
    interval_ms: u64,
}

fn main() -> rs_ctrl_os::Result<()> {
    rs_ctrl_os::init_logging();

    let manager: ConfigManager<DynamicCfg> =
        ConfigManager::new(Path::new("example_config.toml"))?;
    let static_cfg = manager.static_cfg().clone();

    // 通过 manager.get_dynamic_clone() 获取最新 dynamic（文件变化时自动更新）
    Ok(())
}
```

**方式二：不需要热重载时，用 load_config_typed**

```rust
use serde::Deserialize;
use rs_ctrl_os::load_config_typed;

#[derive(Clone, Deserialize)]
struct DynamicCfg {
    message_prefix: String,
}

fn main() -> rs_ctrl_os::Result<()> {
    rs_ctrl_os::init_logging();

    let (static_cfg, dynamic) = load_config_typed::<DynamicCfg>("example_config.toml")?;
    // 一次性加载，无 watcher 开销
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

        // topic_key = "control"，sub_topic = "demo"（bincode 序列化）
        bus.publish_topic("control", "demo", &payload)?;

        // 图像/点云等二进制可用 publish_raw 透传，无需 bincode
        // bus.publish_raw("camera", "frame", &jpeg_bytes)?;

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

- `examples/pub_node.rs` / `examples/sub_node.rs`：单 pub + 单 sub，pub 使用 `ConfigManager` 热重载，sub 使用 `load_config_typed` 一次性加载。
- `examples/multi_pub_node.rs` / `examples/multi_sub_node.rs`：多子话题 pub/sub，`multi_sub` 使用 `set_sub_topics` 过滤子话题；`sub_node` 使用 `try_recv_raw` 接收原始 payload 并反序列化。

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

