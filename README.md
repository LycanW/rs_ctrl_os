# rs_ctrl_os

`rs_ctrl_os` 是一个用于构建分布式节点控制系统的小型运行时库，提供：

- **节点发现**：基于 UDP 多播的心跳机制（`Heartbeat` + `ServiceRegistry`）
- **消息通信**：基于 ZeroMQ 的 pub/sub 抽象（`PubSubManager`）
- **配置管理**：TOML 配置加载 + 动态热更新（`ConfigManager` / `load_config_typed`）
- **时间同步**：简单的主从时钟同步（`TimeSynchronizer`）
- **统一错误/日志**：`RsCtrlError` + `tracing` 日志初始化

适合需要在局域网内跑多进程/多节点，进行“互相发现 + 消息分发 + 动态配置”的系统。

---

## C / C++ API（预编译静态库）

自 **v0.5.0** 起，本库提供 **稳定 C ABI**（`include/rs_ctrl_os.h`）与 `**staticlib` 产物 `librs_ctrl_os.a`**，便于在 **C/C++ 工程** 中链接。**官方 Release 不发布 `librs_ctrl_os.so`**；若需要动态库，请自行克隆源码并在 `Cargo.toml` 的 `[lib] crate-type` 中加入 `"cdylib"` 后编译。

### 预编译包（GitHub Releases）

在仓库上 **推送形如 `v0.5.0` 的 tag** 后，GitHub Actions 会构建并上传 **两个 glibc 压缩包**（文件名随版本变化）：


| 资产                                     | 说明                          |
| -------------------------------------- | --------------------------- |
| `rs_ctrl_os-<版本>-glibc-x86_64.tar.gz`  | `x86_64-unknown-linux-gnu`  |
| `rs_ctrl_os-<版本>-glibc-aarch64.tar.gz` | `aarch64-unknown-linux-gnu` |


每个包内含：

- `librs_ctrl_os.a` — 静态库（链接进你的可执行文件）
- `include/rs_ctrl_os.h` — C 头文件
- `SHA256SUMS` — 校验文件

**不包含**：`libzmq`（请使用系统或 SDK 提供的 `-lzmq`）、源码中的 `c_examples/`（示例留在仓库内）。

**glibc 说明**：预编译库在 **Ubuntu 22.04 / 24.04** 类 CI 镜像上构建；若你的运行环境 **glibc 更旧**，可能无法链接或运行，请在本机 **`cargo build --release`** 自行编译。

### 链接示例（C）

解压后假设当前目录含有 `librs_ctrl_os.a` 与 `include/rs_ctrl_os.h`。`zmq` crate 默认会静态编译 bundled libzmq，其中含 **C++** 目标文件，因此用 **gcc 链 C 主程序时通常需要 `-lstdc++`**：

```bash
gcc -O2 -o myapp myapp.c \
  ./librs_ctrl_os.a \
  -lzmq -lstdc++ -lpthread -ldl -lm
```

**CMake + C++11 示例**（推荐）：目录 **`c_examples/`** 含 `CMakeLists.txt` 与 **`minimal.cpp`**。先在仓库根执行 `cargo build --release`，再：

```bash
cd c_examples
cmake -S . -B build -DRCOS_ROOT=.. -DRCOS_LIB=../target/release/librs_ctrl_os.a
cmake --build build
./build/rcos_minimal ../example_config.toml
```

- **`RCOS_ROOT`**：含 `include/rs_ctrl_os.h` 的路径（预编译包解压目录或本仓库根）。
- **`RCOS_LIB`**：`librs_ctrl_os.a` 的绝对或相对路径；省略时若存在 `RCOS_ROOT/target/release`（或 `debug`）下的静态库会自动选用。
- 若系统默认 `c++` 为不完整 clang，CMake 会尽量选用 **`g++`**。

### 从源码构建（Rust 开发者）

```bash
cargo build --release
# 静态库路径：target/release/librs_ctrl_os.a
```

本仓库带有 **`.cargo/config.toml`**，将 **`CXX=g++` / `CC=gcc`** 传给 `zmq-sys` 的 bundled 构建，避免默认 `c++` 指向 **无 libstdc++ 头文件** 的 clang 而导致编译失败。若你环境不同可自行覆盖。

### C API 行为摘要

- 配置：`rs_ctrl_os_config_open` 打开完整 TOML；`rs_ctrl_os_config_get_dynamic_json` 返回 **`[dynamic]` 的 JSON**（需 `rs_ctrl_os_str_free`）。
- 发现：`rs_ctrl_os_discovery_start` 返回 registry；`**rs_ctrl_os_pubsub_new` 会消费（接管）registry**，成功后 **不要**再 `rs_ctrl_os_registry_destroy`；若 `pubsub_new` 失败需自行 `registry_destroy`。
- 收发：`rs_ctrl_os_pubsub_publish_raw`、`rs_ctrl_os_pubsub_try_recv_raw`；收到消息时对 `sub_topic_out` / `payload_out` 分别 `str_free` / `payload_free`。
- 错误：多数函数返回 `rcos_err_t`；失败详情见 `rs_ctrl_os_last_error`。

---

## 框架能力与边界

### 框架负责什么


| 能力                               | 说明                                                                                                                                       |
| -------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| **StaticBase 与 [static_config]** | 节点 ID、host、port、是否 master、publishers/subscribers 拓扑、static_nodes（IP fallback）、publish_hz/subscribe_hz、dynamic_load_enable 等**运行所需的基础配置** |
| **配置加载机制**                       | 从 TOML 解析 `[static_config]`，提供 `load_config_rcos`、`load_config_typed`、`ConfigManager` 等 API                                              |
| **dynamic 热更新**                  | 当 `dynamic_load_enable=true` 时，监听配置文件变化并热重载 `[dynamic]` 内容                                                                               |
| **消息通道**                         | ZMQ pub/sub、发现、时间同步、频率限速、原始字节透传（`publish_raw` / `try_recv_raw`）                                                                          |


### 框架不负责什么


| 边界                   | 说明                                                                                                 |
| -------------------- | -------------------------------------------------------------------------------------------------- |
| **[dynamic] 的结构与语义** | 框架只负责「加载并热更新」`[dynamic]`，**不定义**其字段。每个应用自行定义 `D: Deserialize`，例如 CAN 接口列表、电机参数、相机参数等               |
| **业务数据内容**           | 图像、点云等大体量数据应通过 topic 传输，**不应**塞进 TOML。配置中只放「如何连接、参数、schema 版本」等元信息                                 |
| **业务协议与编码**          | 消息 payload 的序列化方式（bincode / raw / JPEG 等）由应用选择；框架提供 `publish_topic`（bincode）、`publish_raw`（透传）两种能力 |


### 如何区分框架配置与业务配置

- `**[static_config]`**：框架强依赖，**必须存在**。包含 `my_id`、`host`、`port`、`is_master`、`publish_hz`、`subscribe_hz`、`dynamic_load_enable`、`publishers`、`subscribers`、`static_nodes`（可选）等。
- `**[dynamic]`**：业务自由定义。框架不解析其具体字段，只负责按你提供的 `D` 反序列化并（可选）热更新。  
例如：can_bridge 定义 `interfaces`、`devices`；相机节点定义 `camera_id`、`resolution`；点云节点定义 `voxel_size` 等。

### 框架在背后完成的工作

以下能力由框架自动完成，应用通常无需关心实现细节：


| 模块                    | 后台行为                                                                                                                                                                                       |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **节点发现**              | 启动两个线程：发送端每 1 秒向 `224.0.0.100:9999` 广播本节点 `Heartbeat`；接收端持续收取其它节点心跳，更新 `ServiceRegistry`，超过 10 秒未收到则从注册表剔除。                                                                                |
| **时间同步**              | 接收端收到 `is_master=true` 的心跳时，提取其 `clock_time_ms`，计算本地与 master 的时钟偏移并低通滤波。`now_corrected_ms()` 内部使用该偏移修正当前时间。                                                                                |
| **ConfigManager 热更新** | 当 `dynamic_load_enable=true` 时，通过 `notify` 监听配置文件。文件变化时自动重读并解析 `[dynamic]`，更新内部 `RwLock`，`get_dynamic_clone()` 返回最新值。                                                                      |
| **发布频率控制**            | 当 `publish_hz > 0` 时，`publish_topic` / `publish_raw` 内部按 `topic_key` 记录上次发送时间，超过最小间隔的请求会被静默丢弃（限频）。                                                                                         |
| **订阅频率控制**            | 当 `subscribe_hz > 0` 时，`try_recv_raw` / `try_recv_specific` 内部按 `local_name` 记录上次轮询时间，未到间隔则直接返回 `None`，避免过度轮询。                                                                             |
| **订阅连接建立**            | 初始化时，若 discovery 和 `static_nodes` 均未提供目标地址，订阅进入 `pending_subs`。`try_recv_raw` 内部自动 tick()，优先从 `ServiceRegistry` 解析，其次从 `static_nodes`（`node_id -> "host:port"`）fallback，**无需手动调用 tick()**。 |
| **子话题过滤**             | 若通过 `set_sub_topics` 设置了白名单，框架在 `try_recv_raw` 中只返回白名单内的 `sub_topic`，其它消息静默丢弃。                                                                                                             |


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
git clone https://github.com/LycanW/rs_ctrl_os.git
cd rs_ctrl_os
```

或者使用 cargo 添加依赖：

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

`pub_node` 会持续往 `control` topic 发布消息。**注意**：当前 `pub_node` 示例仅发布、不订阅，因此不会打印收到的消息。若需同时收发，可参考 `examples/` 自行扩展，或运行两个进程（见下文）。

如果你改动配置文件里的 `[dynamic]`（比如改前缀、改间隔）并保存，进程会自动加载新的动态配置：

- `message_prefix` 会改变打印出来的文本前缀；
- `interval_ms` 会改变发送/接收的频率。

### 2. 跑两个进程：一个 pub，一个 sub

有时候你想在两个不同的进程里测试 pub/sub。可以用仓库里的：

- `examples/pub_node.rs`
- `examples/sub_node.rs`

以及对应的 `pub_config.toml`、`sub_config.toml`。

**配置说明**：`sub_config.toml` 中 `local_sub` 指向的 `target_node_id` 需与 `pub_node` 的 `my_id` 一致，才能收到 pub 的消息。仓库中的 `sub_config.toml` 可能指向其他节点（如 `gateway_node_01`），用于不同场景。若要 pub/sub 互通，可将 `[static_config.subscribers]` 改为 `local_sub = "pub_node"`，并配置 `static_nodes` 或依赖 discovery 解析地址。

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
- `sub_node` 通过 discovery 或 `static_nodes` 连接 `pub_node`，从 `local_sub` 收消息并打印。

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

## API 参考

### 1. 初始化

#### `init_logging()`

初始化 `tracing` 日志，默认 INFO 级别。应在 `main` 入口处调用一次。

```rust
use rs_ctrl_os::init_logging;
init_logging();
```

---

### 2. 配置管理

#### `load_config_rcos(path) -> Result<(StaticBase, toml::Value)>`

从 TOML 加载配置，返回框架静态配置 + 原始 `[dynamic]`（`toml::Value`）。  
适用于需要手动反序列化 `[dynamic]` 或只需 `static_config` 的场景。  

- **path**：配置文件路径（`impl AsRef<Path>`）  
- **返回**：`(StaticBase, toml::Value)`，`[dynamic]` 缺失时返回空表

#### `load_config_typed::<D>(path) -> Result<(StaticBase, D)>`

一次性加载配置，返回强类型 `(StaticBase, D)`。无文件监听，无热更新开销。  

- **D**：需实现 `Deserialize`，对应 `[dynamic]` 结构  
- **适用**：不需要热重载的节点

#### `ConfigManager<D>`

带热重载的配置管理器。当 `dynamic_load_enable=true` 时，通过 `notify` 监听文件变化并自动重载 `[dynamic]`。


| 方法                  | 签名                                        | 说明                             |
| ------------------- | ----------------------------------------- | ------------------------------ |
| `new`               | `new(config_path: &Path) -> Result<Self>` | 加载配置并（可选）启动文件监听                |
| `static_cfg`        | `static_cfg(&self) -> &StaticBase`        | 获取静态配置引用                       |
| `get_dynamic_clone` | `get_dynamic_clone(&self) -> D`           | 获取当前 `[dynamic]` 的克隆（热更新后为最新值） |
| `config_path`       | `config_path(&self) -> &Path`             | 配置文件路径                         |


**D 约束**：`Clone + Deserialize + Send + Sync + 'static`

#### `StaticBase`

框架静态配置结构体，从 TOML `[static_config]` 解析。


| 字段                    | 类型                        | 默认      | 说明                                               |
| --------------------- | ------------------------- | ------- | ------------------------------------------------ |
| `my_id`               | `String`                  | -       | 本节点唯一 ID                                         |
| `host`                | `String`                  | -       | 本节点监听地址（如 `127.0.0.1`）                           |
| `port`                | `u16`                     | -       | 本节点监听端口                                          |
| `is_master`           | `bool`                    | `false` | 是否作为时间同步 master                                  |
| `publishers`          | `HashMap<String, String>` | `{}`    | `topic_key -> "node_id"` 或 `"self"`（本地绑定）        |
| `subscribers`         | `HashMap<String, String>` | `{}`    | `local_name -> target_node_id`                   |
| `static_nodes`        | `HashMap<String, String>` | `{}`    | `node_id -> "host:port"`，discovery 失败时的 fallback |
| `publish_hz`          | `i64`                     | -       | 发布频率上限：`>0` 限频，`0` 不限，`<0` 禁止发布                  |
| `subscribe_hz`        | `i64`                     | -       | 订阅轮询频率：`>0` 限频，`0` 不限，`<0` 禁止订阅                  |
| `dynamic_load_enable` | `bool`                    | `true`  | 是否启用 `[dynamic]` 热更新                             |


---

### 3. 节点发现

#### `start_discovery(...) -> Result<ServiceRegistry>`

```rust
pub fn start_discovery(
    my_id: &str,
    my_host: &str,
    my_port: u16,
    is_master: bool,
    time_sync: Option<Arc<TimeSynchronizer>>,
) -> Result<ServiceRegistry>
```

启动 UDP 多播发现（`224.0.0.100:9999`）。后台线程每 1 秒广播心跳，接收其他节点心跳并更新注册表。  

- **time_sync**：传入 `TimeSynchronizer` 时，会从 master 心跳中提取 `clock_time_ms` 进行时间同步  
- **返回**：共享的 `ServiceRegistry`，供 `PubSubManager` 解析订阅目标地址

#### `ServiceRegistry`

节点注册表，内部维护 `node_id -> (host, port, timestamp)`。


| 方法            | 签名                                                           | 说明                           |
| ------------- | ------------------------------------------------------------ | ---------------------------- |
| `new`         | `new() -> Self`                                              | 创建空注册表                       |
| `register`    | `register(&self, hb: &Heartbeat)`                            | 注册/更新节点（框架内部使用）              |
| `get_address` | `get_address(&self, node_id: &str) -> Option<(String, u16)>` | 根据 node_id 获取 `(host, port)` |
| `cleanup`     | `cleanup(&self, timeout_secs: u64)`                          | 剔除超时未心跳的节点（框架内部每轮调用）         |


#### `Heartbeat`

心跳消息结构（JSON 序列化，用于发现协议）。通过 `rs_ctrl_os::discovery::Heartbeat` 访问（未在 crate 根重导出）。

```rust
pub struct Heartbeat {
    pub node_id: String,
    pub host: String,
    pub port: u16,
    pub timestamp: u64,
    pub clock_time_ms: u64,
    pub is_master: bool,
}
```

---

### 4. ZeroMQ Pub/Sub（PubSubManager）

#### 创建

```rust
pub fn new(static_cfg: &StaticBase, registry: ServiceRegistry) -> Result<Self>
```

根据 `static_config` 的 `publishers`/`subscribers` 绑定 PUB、连接 SUB。`target = "self"` 的 topic 共用本机 PUB socket。

#### 频率与过滤


| 方法                 | 签名                                                                        | 说明                                                          |
| ------------------ | ------------------------------------------------------------------------- | ----------------------------------------------------------- |
| `set_publish_hz`   | `set_publish_hz(&mut self, hz: i64)`                                      | **覆盖**发布频率。`new()` 已从 `static_config` 注入，通常无需调用；仅在运行时需修改时使用 |
| `set_subscribe_hz` | `set_subscribe_hz(&mut self, hz: i64)`                                    | **覆盖**订阅轮询频率。同上，一般依赖配置即可                                    |
| `set_sub_topics`   | `set_sub_topics(&mut self, local_name: &str, topics: &[S]) -> Result<()>` | 为 `local_name` 设置 sub_topic 白名单，仅返回列表内的消息；空列表表示不过滤          |
| `tick`             | `tick(&mut self) -> Result<()>`                                           | 尝试为 `pending_subs` 建立连接；`try_recv_raw` 内部会自动调用，一般无需手动调用     |


#### 发布


| 方法              | 签名                                                                                                 | 说明                      |
| --------------- | -------------------------------------------------------------------------------------------------- | ----------------------- |
| `publish_topic` | `publish_topic<T: Serialize>(&mut self, topic_key: &str, sub_topic: &str, data: &T) -> Result<()>` | Bincode 序列化后发送，适合结构化小消息 |
| `publish_raw`   | `publish_raw(&mut self, topic_key: &str, sub_topic: &str, payload: &[u8]) -> Result<()>`           | 透传原始字节，适合图像、点云等已编码数据    |


**消息格式**：ZMQ 三帧 multipart `[节点ID, sub_topic, payload]`

**频率控制**：当 `publish_hz > 0` 时，按 `topic_key` 限频，超频请求静默丢弃。

#### 接收


| 方法                  | 签名                                                                                                      | 说明                                                            |
| ------------------- | ------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| `try_recv_raw`      | `try_recv_raw(&mut self, local_name: &str) -> Result<Option<(String, Vec<u8>)>>`                        | 非阻塞接收，返回 `(sub_topic, payload)`；内部自动 `tick()` 并做频率限制          |
| `try_recv_specific` | `try_recv_specific<T: Deserialize>(&mut self, local_name: &str, target_sub: &str) -> Result<Option<T>>` | 仅当 `sub_topic == target_sub` 时用 bincode 反序列化为 `T`，否则返回 `None` |


**频率控制**：当 `subscribe_hz > 0` 时，按 `local_name` 限频，未到间隔返回 `None`。

---

### 5. 时间同步（TimeSynchronizer）

主从时钟同步，从 `is_master=true` 的心跳中提取 `clock_time_ms` 计算偏移。


| 方法                   | 签名                                                              | 说明                  |
| -------------------- | --------------------------------------------------------------- | ------------------- |
| `new`                | `new() -> Self`                                                 | 创建同步器，初始未同步         |
| `update_from_master` | `update_from_master(&self, master_id: &str, master_ts_ms: u64)` | 由发现模块内部调用，应用一般不直接使用 |
| `now_corrected_ms`   | `now_corrected_ms(&self) -> u64`                                | 返回经偏移修正的当前时间（毫秒）    |
| `is_synced`          | `is_synced(&self) -> bool`                                      | 是否已与 master 同步      |


**用法**：将 `Arc::new(TimeSynchronizer::new())` 传入 `start_discovery`，之后用 `now_corrected_ms()` 获取协调后的时间戳。

---

### 6. 错误类型

```rust
use rs_ctrl_os::{Result, RsCtrlError};
```


| 变体                                 | 说明                 |
| ---------------------------------- | ------------------ |
| `Config(String)`                   | 配置加载/解析错误          |
| `Comms(String)`                    | 通信错误（如 topic 未找到）  |
| `Serialization(String)`            | 序列化错误              |
| `Discovery(String)`                | 发现模块错误             |
| `NodeNotFound(String)`             | 注册表中无此 node_id     |
| `Io(std::io::Error)`               | IO 错误              |
| `Zmq(zmq::Error)`                  | ZeroMQ 错误          |
| `Bincode(Box<bincode::ErrorKind>)` | Bincode 序列化/反序列化错误 |


所有 API 返回 `rs_ctrl_os::Result<T>`，可用 `?` 传播。

---

## 安装

在你的 `Cargo.toml` 中添加依赖：

```toml
[dependencies]
rs_ctrl_os = "0.5.0"
```

或者也可以

```bash
cargo add rs_ctrl_os
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

以下片段需与步骤 1（**方式一** ConfigManager）、步骤 3 组合使用，才能获得 `manager`、`static_cfg`、`registry`、`time_sync`。

```rust
use rs_ctrl_os::PubSubManager;
use std::thread;
use std::time::Duration;

fn main() -> rs_ctrl_os::Result<()> {
    init_logging();
    // ... 步骤 1（ConfigManager）+ 步骤 3（start_discovery），得到 manager, static_cfg, registry, time_sync

    let mut bus = PubSubManager::new(&static_cfg, registry)?;

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
        let interval = if static_cfg.subscribe_hz > 0 {
            Duration::from_secs_f64(1.0 / static_cfg.subscribe_hz as f64)
        } else {
            Duration::from_millis(100)
        };
        thread::sleep(interval);
    }
}
```

---

## 示例（examples）

### 内置示例

- `examples/pub_node.rs` / `examples/sub_node.rs`：单 pub + 单 sub，pub 使用 `ConfigManager` 热重载，sub 使用 `load_config_typed` 一次性加载。
- `examples/multi_pub_node.rs` / `examples/multi_sub_node.rs`：多子话题 pub/sub，`multi_sub_node` 使用 `set_sub_topics` 过滤子话题；两者均通过 `try_recv_raw` 接收原始 payload 并自行反序列化。

### 实际项目示例

**[can_bridge](https://github.com/LycanW/can_bridge)**：CAN 总线网关，将 Linux SocketCAN 与 ZeroMQ 打通，实现 CAN ↔ 分布式消息的双向桥接。基于 rs_ctrl_os 构建，典型用法包括：

- 使用 `ConfigManager` 加载配置并热重载 `[dynamic]`（接口、设备、控制开关等）
- `start_discovery` + `PubSubManager` 实现传感器数据发布（`sensor_mit` / `sensor_dji` / `sensor_imu`）与控制指令订阅（`ctrl_mit` / `ctrl_dji`）
- `publish_topic` 发布解析后的传感器 JSON，`try_recv_raw` 接收控制命令
- `[static_config]` 完全遵循 rs_ctrl_os 规范

可作为将 rs_ctrl_os 应用于机器人/嵌入式桥接场景的参考。

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
- 序列化错误：`Serialization(String)`
- 发现错误：`Discovery(String)`
- 节点未找到：`NodeNotFound(String)`
- IO 错误：`Io(std::io::Error)`
- ZeroMQ 错误：`Zmq(zmq::Error)`
- Bincode 序列化错误：`Bincode(Box<bincode::ErrorKind>)`

绝大多数 API 都返回 `rs_ctrl_os::Result<T>`，便于在上层直接用 `?` 传播。

---

## 许可证

本项目采用 **MIT** 许可证发布。  
详见 `LICENSE` 文件。