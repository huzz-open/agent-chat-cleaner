# Agent Chat Cleaner 产品设计与风险审查

> 状态：工程规格 v1.2（Rust 可恢复清理原型已实现，公开发布前仍需签名与真机验证）
> 工作名：Agent Chat Cleaner
> 核心定位：一款本地优先、可预览、可恢复的 AI 编程 Agent 会话整理工具。

> [!IMPORTANT]
> 截至 2026-08-03，仓库内已有 `desktop/` Cargo workspace、egui/eframe 桌面端、Codex/Claude Code 首行元数据扫描器、独占会话文件的隔离/恢复实现与 Windows x64 portable EXE。当前原型**不支持共享 JSONL/SQLite 的写入，也不提供永久删除**；这些格式保持只读，避免把共享大文件整体删除。官网中的示例数字仍是产品展示，不代表扫描结果。

### 当前仓库状态

| 交付物 | 当前状态 | 说明 |
| --- | --- | --- |
| 产品与风险方案 | 已完成并自审 | 本文；包含按会话粒度、安全边界与发布门禁 |
| 中英双语官网 | 已完成 | 产品定位与 Windows 下载入口，不展示收费信息 |
| Rust workspace | **已创建** | `desktop/` 四 crate workspace，锁定依赖 |
| Windows `.exe` / `.msi` | **portable EXE 已生成** | MSI、代码签名留到公开稳定版 |
| macOS `.app` / `.dmg` | **不存在** | P2 以后交付 |
| Linux AppImage / `.deb` | **不存在** | 测试版交付 |
| Codex / Claude Code 适配器 | **安全子集已实现** | 只识别可验证的单会话独占 JSONL；共享容器只读 |
| 支付、订单、授权 | **不在当前范围** | 当前版本直接下载运行，无收费和激活 |
| 隐私、许可、退款正式文本 | **未完成** | 商业发布阻断项 |

当前可作为 Windows x64 portable 技术预览下载运行。公开稳定版仍需补充代码签名、更多真实版本兼容样本、故障注入和共享容器的选择性重写；当前阶段不接支付。

## 0. 本次审查结论

原文件在本次审查开始时是 0 字节，因此没有可逐条批注的旧方案。以下方案依据当前产品方向重新补全。

这个产品有需求，但不能只做“批量删除 Codex 会话”。Codex 已经开始提供原生 `delete/archive` 能力，Claude Code 也在持续完善 session 管理。只卖一个删除按钮会很快被官方功能覆盖。

更可持续的定位是：

**跨 Agent 的本地会话卫生中心：统一盘点空间占用，用安全规则找出无价值会话，先隔离、后清除，并能恢复误操作。**

产品的五条不可妥协底线：

1. 默认只读扫描，任何删除前都必须有清单和预计释放空间。
2. 正在运行、格式未知、来源不明的会话一律不动。
3. 第一次“清理”只进入隔离区，真正永久删除是第二个独立动作。
4. 聊天正文不上传；应用不需要账号、付款或激活即可运行。
5. 删除单位是用户选中的 session，而不是承载它的文件；共享大文件只能记录级改写或事务级删除，绝不整文件删除。

## 1. 用户与问题

### 目标用户

- 高频使用 Codex、Claude Code 等 Agent 的个人开发者。
- 同时维护多个代码仓库、长期积累大量历史 session 的用户。
- 对隐私敏感，不愿安装需要读取所有网页内容的浏览器扩展的用户。
- 希望释放磁盘、减少历史噪声，但又担心误删有价值上下文的用户。

### 核心任务

- 一眼看清每个 Agent、项目和时间段占用了多少空间。
- 找到旧的、空的、失败的、重复的或极短的会话。
- 在清理前查看会话摘要、路径、时间、大小和风险状态。
- 精确选择一个或多个 session，只处理被选择的数据，不连带其他 session。
- 批量归档或隔离，并能可靠撤销。
- 到期后永久清除隔离区，真正释放空间。

### MVP 不做

- 不操作 ChatGPT、Claude.ai 等云端网页聊天。
- 不调用或仿造第三方未公开接口。
- 不编辑会话正文，不承诺“优化模型效果”。
- v1 不提供单条 message 级编辑或删除；最小用户可选单位是一个 session。
- 不做账号体系、云同步、团队共享。
- 不把“整理”“归档”和“永久删除”混成一个按钮。

## 2. MVP 范围

### 首发支持

- Windows 10/11 优先。
- macOS 第二优先；Linux 作为测试版。
- 数据源：Codex、Claude Code。
- 后续适配：Cursor、Cline、Roo Code、Continue、Aider 等。

### 核心功能

1. **自动发现**：识别已安装/使用过的 Agent 及其默认数据目录，也允许手动添加目录。
2. **只读扫描**：流式读取 JSON/JSONL，SQLite 使用只读连接；不一次性加载全部正文。
3. **统一列表**：Agent、项目、标题/首条提示、最后活动时间、大小、消息数、状态。
4. **筛选规则**：按 Agent、项目、日期、大小、消息数、失败/空会话、收藏状态筛选。
5. **空间视图**：展示可安全隔离的大小，不把数据库稀疏页等同于可立即释放空间。
6. **安全预览**：展示将被操作的 session、关联文件、共享容器、删除粒度、风险提示和预计结果。
7. **按需隔离清理**：独占文件可整体移动；共享 JSONL/SQLite 只移除选中 session 对应的记录，并把被移除记录写入 quarantine；绝不因选择一个 session 删除整个共享文件。
8. **一键恢复**：恢复原路径；冲突时提供保留两份、覆盖或取消。
9. **到期清除**：默认保留 7 天，用户主动确认后才永久清除。
10. **中英双语**：首次跟随系统语言，设置中可切换；所有危险提示必须完整翻译。

### 推荐的三档规则

- 安全：仅空会话、解析失败但确认无正文的残留、30 天以上且未收藏的极短会话。
- 平衡：90 天以上、未收藏、未在运行、最近未访问的普通会话。
- 自定义：用户明确组合条件，实时显示命中项和风险。

不要提供“自动永久删除”。即使以后增加定时整理，也只能自动移动到隔离区。

## 3. 最容易出事故的技术漏洞

### 3.1 格式变化导致误删

每个 Agent 的本地格式都可能随版本变化。适配器必须输出 `Supported / UnknownVersion / Corrupted / Busy`，只有 `Supported` 才能进入批量操作。未知字段要保留，不能因解析失败就把文件判为垃圾。

每个适配器需要：

- 可识别的格式版本或特征探针。
- 黄金样本测试和损坏样本测试。
- 未识别版本的 fail-closed 行为。
- 应用更新后重新扫描，不能复用旧判断直接删除。

### 3.2 正在运行的会话被移动

不能只看文件修改时间。至少组合以下信号：

- Agent 进程是否存在。
- 文件是否被占用或锁定。
- 最近是否持续增长。
- session 元数据是否标记 active。

任何信号不确定，都标记“正在使用或状态未知”，禁止操作。UI 不能允许用户批量绕过；如需强制操作，只能在单条详情中二次输入确认。

### 3.3 把共享大文件误当成单个 session 删除

一个 session 可能对应独占 transcript，也可能只是共享 JSONL、SQLite、索引或附件容器中的部分记录。文件大小和 session 大小不是同一个概念。

硬规则：

- 用户选择和删除的最小单位是 `SessionKey`，不是路径。
- 只有已证明“这个文件中的全部数据都专属于所选 session”时，才允许移动整个文件。
- 多个 session 共用一个 JSONL/数据库时，删除一个 session 必须保留其他 session。
- 不能安全识别记录边界、引用关系或恢复方式时，返回 `GranularDeleteUnsupported`，不提供“改为删除整个文件”的降级选项。
- UI 必须明确显示本次操作是“移动独占文件”“改写共享 JSONL”“数据库事务”还是“调用官方 session API”。

操作优先级：

1. 官方按 session archive/delete API 或命令。
2. 适配器已验证的数据库行级事务。
3. 适配器已验证的共享 JSONL 流式重写。
4. 仅在资源完全独占时移动整个资源集合。
5. 以上都不满足则禁止删除。

一个 session 还可能同时拥有 transcript 行、索引行、附件和缓存。适配器必须把它们描述为同一个 `SessionResourceSet`，在一个 plan 中完成或全部回滚，不能只删正文留下悬挂索引，也不能为清一个索引删除整个数据库。

### 3.4 符号链接与目录逃逸

- 所有根目录先规范化为绝对路径。
- 默认不跟随 symlink、junction、reparse point。
- 每个待操作目标再次校验仍位于允许的数据根目录内。
- 拒绝 `..`、盘符切换、UNC 意外跳转和循环链接。
- 隔离目录不得放在 Agent 的自动扫描目录内。

### 3.5 跨盘移动并非原子操作

同一文件系统内使用原子 rename。跨卷默认拒绝“快速清理”，可选择：复制到隔离区 → 流式校验哈希与长度 → fsync → 写入 manifest → 再删除源文件。任一步失败都保留源文件并报告未完成。

### 3.6 恢复不完整

隔离清单至少记录：来源适配器、适配器版本、session ID、原始路径、隔离路径、文件长度、哈希、mtime、操作时间和关联资源。恢复前验证哈希；原路径有新文件时绝不静默覆盖。

### 3.7 扫描与清理竞态

扫描结果只是一张快照。执行前必须重新读取关键元数据，并验证大小、mtime、session ID、active 状态未变化；变化的条目自动跳过。

### 3.8 “释放空间”数字失真

SQLite 删除记录通常不会马上让数据库文件变小，硬链接、压缩和稀疏文件也会影响结果。界面应区分：

- 选中资源的逻辑大小。
- 预计可回收空间。
- 已实际回收空间。

不要在执行前承诺精确释放值。

### 3.9 共享大文件细删需要额外临时空间

从 1 GB 共享 JSONL 中删除一个小 session，仍可能需要顺序读取并重写接近 1 GB 数据。执行前必须展示预计读取量、写入量、临时空间和耗时，并确认目标卷有足够空间。

- 禁止原地截断、原地覆盖或边读边改源文件。
- 临时新容器必须与源容器位于可原子 replace 的同一文件系统。
- 临时空间不足时操作不开始，源文件保持不变。
- 用户取消只能发生在提交点之前；进入原子 replace 临界区后必须完成提交或回滚。
- 不能为了节省临时空间改成删除整个大文件。

## 4. Rust 桌面端建议

### UI 技术选型（已决策）

“纯 Rust”本身不等于小体积。最终包大小通常由 GPU 渲染后端、字体整形、多语言字体、图像解码和自动更新组件主导。

方案评估如下。由于 v1 已把“UI 源码也必须是 Rust”确定为硬约束，最终选择不再保持开放：

| 方案 | 优点 | 风险 | 建议 |
| --- | --- | --- | --- |
| Slint + Rust core | 原生编译、声明式 UI、成品观感更容易做好、资源占用低 | UI 使用 `.slint` DSL；商业产品需遵守 royalty-free license 的署名要求 | 备选，不进入 v1 P0 |
| egui/eframe | UI 逻辑也是 Rust，开发快，表格/诊断工具合适 | 默认观感偏工具化；CJK 字体需自行配置；GPU 后端和字体可能增大包 | **v1 已选定** |
| iced | Elm 架构清晰、类型安全 | 官方仍标注 experimental，复杂桌面控件的维护成本需验证 | 暂不首发 |
| Floem/GPUI | 现代响应式体验、性能潜力大 | 生态和 API 仍在快速变化 | 暂不首发 |

结论：v1 选择 egui/eframe。仍然必须用包含中文、虚拟列表、扫描进度和 1 万行数据的真实原型测量，确认这一选择满足体积和性能目标；如果失败，必须用正式 ADR 记录数据与迁移理由。

Slint 的 royalty-free 桌面许可可用于闭源商业产品，但需要按其许可要求署名。发版前仍应保存所用版本的许可文本并做一次法务核对。

### 推荐工程分层

```text
crates/
  app-core/          领域模型、规则、用例
  adapter-api/       Agent 适配器 trait
  adapter-codex/     Codex 探测、解析、资源映射
  adapter-claude/    Claude Code 探测、解析、资源映射
  scanner/           流式扫描、取消、进度
  quarantine/        manifest、移动、校验、恢复、清除
  telemetry/         默认关闭、只允许匿名产品事件
  desktop-ui/        egui/eframe
```

核心层不得依赖 UI；适配器不得直接发出永久删除，必须提交 `OperationPlan` 给 quarantine 层执行。

### 性能与体积原则

- 扫描线程与 UI 线程分离，支持取消和增量上报。
- JSONL 流式读取；只提取列表所需字段，详情按需加载。
- 目录遍历设置边界、深度和超时，避免扫描整个 home。
- 10,000+ 会话使用虚拟列表，不创建 10,000 个完整控件。
- `default-features = false`，只启用实际需要的渲染和图片格式。
- Release 打开 LTO、strip symbols、`panic = "abort"`；`opt-level = "s"` 与 `3` 都用真实数据 benchmark 后选择。
- 中文字体优先使用系统字体 fallback；如要内置字体，先确认许可证和增量体积。
- 冷启动目标 < 300 ms、空闲内存目标 < 50 MB、空闲 CPU 约 0、扫描期间 UI 不掉帧。目标必须在三平台 CI 的真机或固定 runner 上测量。

### 4.1 v1 技术决策：egui/eframe

为了满足“UI 也使用 Rust、无 WebView、无 Node/Electron 运行时”的要求，v1 桌面端确定采用：

- Rust stable，最低支持版本 MSRV 在首个可运行版本冻结。
- `egui + eframe` 构建全部桌面界面。
- UI、业务逻辑、扫描器、适配器和隔离区全部编译进原生 Rust 二进制。
- v1 不使用 Tauri、Electron、CEF、WebView2、HTML、CSS 或 JavaScript 作为桌面 UI。
- 官网继续是独立 Web 项目，不进入桌面端二进制。

选择 egui 的原因不是它一定最小，而是它满足全 `.rs`、跨平台和快速实现复杂列表的硬约束。P0 必须用真实功能测量产物；如果包含中文与 10,000 行虚拟列表后无法达到性能/体积目标，才能通过 Architecture Decision Record 改为 Slint，不能在开发中途随意混用两个 UI 框架。

首版渲染后端采用 `glow` 原型并关闭不需要的默认 feature；同时保留一次 `wgpu` 对照构建。最终后端以 Windows 10/11 兼容性、包体、冷启动、GPU 驱动问题和远程桌面表现决定，不以 Hello World 结果决定。

### 4.2 仓库与 Cargo workspace

官网已在仓库根目录，桌面端不应打乱现有部署结构。Rust 工程放入独立 workspace：

```text
agent-chat-cleaner/
  app/                              官网（现有）
  public/                           官网资源（现有）
  desktop/
    Cargo.toml                      Rust workspace
    Cargo.lock                      必须提交，保证应用构建可复现
    rust-toolchain.toml             固定 stable toolchain 与组件
    deny.toml                       许可证、重复依赖与安全规则
    apps/
      agent-chat-cleaner/
        Cargo.toml
        src/main.rs                 唯一 GUI 入口
        src/app.rs                  eframe App 与页面路由
        src/commands.rs             UI 发出的领域命令
        assets/                     仅允许已核对许可证的应用资源
    crates/
      app-core/                     领域模型、规则、状态机
      adapter-api/                  AgentAdapter trait 与公共类型
      adapter-codex/                Codex 发现、解析、资源映射
      adapter-claude/               Claude Code 发现、解析、资源映射
      scanner/                      有界并发、取消、增量结果
      quarantine/                   plan、manifest、移动、恢复、清除
      platform/                     路径、进程、锁、系统字体、密钥存储
      diagnostics/                  脱敏日志与诊断包
    fixtures/
      codex/                        合成/脱敏黄金样本
      claude/                       合成/脱敏黄金样本
      corrupted/                    截断、未知字段、损坏索引样本
    benches/
      scan.rs                       1k/10k/100k session 扫描基准
    tests/
      operation_recovery.rs         中断与恢复集成测试
      path_boundary.rs              路径逃逸、链接、junction 测试
    packaging/
      windows/                      WiX 配置、图标、升级代码
      macos/                        entitlements、签名/公证脚本
      linux/                        desktop file、AppImage/deb 配置
```

根目录 Web 构建与 `desktop/` Rust 构建相互独立：

```powershell
# 官网
npm run build

# Rust 桌面端
cargo build --manifest-path desktop/Cargo.toml --workspace --release
```

### 4.3 首个必须存在的二进制

P0 不以截图或 UI mockup 验收，必须生成真实可执行文件：

二进制属于构建产物，不直接提交进 Git：本地构建输出到 `desktop/target/`，正式产物由 CI 附加到对应 Release。仓库必须提交 Rust 源码、`Cargo.lock`、打包配置、哈希生成逻辑和构建说明，使任何干净环境能够重新生成相同版本。当前仓库既没有这些源码，也没有 CI Release 产物，因此不是“二进制被忽略”，而是桌面端确实尚未开始实现。

| 平台 | Cargo target | 原始产物 | 分发产物 | P0 要求 |
| --- | --- | --- | --- | --- |
| Windows x64 | `x86_64-pc-windows-msvc` | `agent-chat-cleaner.exe` | portable `.zip`，随后 `.msi` | **必须** |
| Windows ARM64 | `aarch64-pc-windows-msvc` | `agent-chat-cleaner.exe` | portable `.zip` / `.msi` | P2 |
| macOS Apple Silicon | `aarch64-apple-darwin` | Unix executable | `.app` + `.dmg` | P2 |
| macOS Intel | `x86_64-apple-darwin` | Unix executable | universal `.app` 可选 | 验证需求后 |
| Linux x64 | `x86_64-unknown-linux-gnu` | `agent-chat-cleaner` | AppImage + `.deb` | 测试版 |

首个二进制的最小行为：

1. 能启动真实原生窗口并显示版本、构建 commit 和当前语言。
2. 能只读发现 Codex 与 Claude Code 默认目录；找不到时显示“未发现”，不能 panic。
3. 能扫描脱敏 fixture 并展示会话列表、来源、时间和逻辑大小。
4. 能识别独占文件、共享 JSONL、共享 SQLite，并展示每个 session 的 granular delete capability。
5. 只能生成精确到选择集的 `OperationPlan` 预览；P0 默认禁止实际移动和永久删除。
6. 提供“导出脱敏诊断”，不得包含正文、项目名、用户名或完整路径。
7. 关闭窗口后没有后台常驻进程。

Windows P0 验收命令：

```powershell
cargo build --manifest-path desktop/Cargo.toml `
  -p agent-chat-cleaner `
  --release `
  --target x86_64-pc-windows-msvc

desktop\target\x86_64-pc-windows-msvc\release\agent-chat-cleaner.exe --version
```

构建成功但没有实际运行 `--version`、启动窗口和扫描 fixture，不算 P0 完成。

### 4.4 Cargo 依赖边界

具体版本在创建 workspace 时写入 `Cargo.lock`，本文不写容易过期的精确版本。允许的首发依赖类别：

| 能力 | 首选 | 约束 |
| --- | --- | --- |
| UI | `eframe`, `egui` | 关闭不需要的默认 features |
| 异步/任务 | 标准线程 + `crossbeam-channel`，必要时再引入 `tokio` | 不因一个 HTTP 请求引入完整 runtime |
| 遍历 | `walkdir` 或受控自研遍历 | 默认不跟随链接，限制根目录与深度 |
| JSON/JSONL | `serde`, `serde_json` | 流式反序列化，不加载整个 transcript |
| SQLite | `rusqlite` | 只读 URI、busy timeout、禁止迁移第三方 DB |
| 哈希 | `blake3` 用于本地文件校验 | 下载产物另提供 SHA-256 |
| HTTP | `ureq` 或裁剪后的 `reqwest` | 超时、TLS、无自动上传日志 |
| 本地配置 | `serde` + 原子写文件 | 配置不保存正文或完整扫描结果 |
| 秘密存储 | Windows Credential Manager / macOS Keychain / Linux Secret Service | 通过 `platform` crate 封装 |
| 日志 | `tracing` | 字段白名单、路径与正文默认禁止记录 |
| 错误 | `thiserror` | library 不使用字符串拼接判断错误 |

所有新增 crate 必须通过：

- `cargo deny check licenses bans advisories sources`
- 许可证允许当前分发方式。
- 不存在无维护且处理不可信输入的关键依赖。
- 没有为了便利引入完整浏览器、脚本运行时或服务端组件。

### 4.5 领域模型与适配器契约

UI 不得直接操作磁盘。核心数据模型至少包含：

```rust
pub struct SessionSummary {
    pub key: SessionKey,
    pub agent: AgentKind,
    pub display_title: RedactedText,
    pub project_label: RedactedText,
    pub last_activity: SystemTime,
    pub logical_bytes: u64,
    pub reclaimable_bytes: Option<u64>,
    pub message_count: Option<u64>,
    pub safety: SafetyState,
    pub storage: StorageLayout,
    pub delete_capability: DeleteCapability,
}

pub enum SafetyState {
    SupportedIdle,
    Active,
    Busy,
    UnknownFormat,
    Corrupted,
    OutsideAllowedRoot,
}

pub enum StorageLayout {
    ExclusiveResources,
    SharedJsonl { container: ContainerId },
    SharedSqlite { container: ContainerId },
    ManagedByOfficialApi,
}

pub enum DeleteCapability {
    OfficialSessionOperation,
    MoveExclusiveResources,
    RewriteSelectedJsonlRecords,
    DeleteTransactionalRows,
    GranularDeleteUnsupported { reason: String },
}

pub struct OperationPlan {
    pub id: PlanId,
    pub created_at: SystemTime,
    pub snapshot: Vec<ResourceFingerprint>,
    pub operations: Vec<PlannedOperation>,
    pub estimated_reclaimable_bytes: u64,
    pub blocked: Vec<BlockedReason>,
}

pub enum PlannedOperation {
    OfficialSessionOperation { session: SessionKey },
    MoveExclusiveResources { session: SessionKey, resources: Vec<ResourceId> },
    RewriteSharedJsonl {
        container: ContainerId,
        selected_sessions: Vec<SessionKey>,
        expected_container: ResourceFingerprint,
    },
    MutateSharedSqlite {
        container: ContainerId,
        selected_sessions: Vec<SessionKey>,
        expected_schema: SchemaFingerprint,
    },
}
```

适配器契约：

```rust
pub trait AgentAdapter: Send + Sync {
    fn kind(&self) -> AgentKind;
    fn discover_roots(&self, platform: &dyn Platform) -> Result<Vec<DataRoot>>;
    fn probe(&self, root: &DataRoot) -> Result<FormatSupport>;
    fn scan(
        &self,
        root: &DataRoot,
        sink: &mut dyn SessionSink,
        cancel: &CancellationToken,
    ) -> Result<ScanReport>;
    fn resources_for(&self, key: &SessionKey) -> Result<SessionResourceSet>;
}
```

适配器只有“发现、探测、读取、描述资源和生成受约束 mutation plan”的权限，**不能提供接收任意路径的 `delete()` 方法**。所有破坏性操作统一交给 quarantine crate，在重新验证 plan snapshot 后执行。

`MoveExclusiveResources` 必须带有适配器给出的独占性证据；同一 `ResourceId` 如果被两个未选中的 session 引用，plan 构建直接失败。`RewriteSharedJsonl` 和 `MutateSharedSqlite` 只能携带 session identity 与版本化容器信息，不能让 UI 传入任意行号、SQL 或文件路径。

### 4.6 扫描流水线

```text
选择/发现根目录
  → 规范化绝对路径
  → 检查根目录白名单与链接
  → adapter probe
  → Supported 才进入只读 scan
  → 增量发送 SessionSummary
  → 统计逻辑/可回收空间
  → UI 虚拟列表渲染
```

约束：

- 每个数据源独立失败；Claude 解析失败不能阻断 Codex 结果。
- 有界 worker 数，默认不超过 `min(4, logical_cpu_count)`。
- 大文件设置读取上限；详情读取必须由用户触发。
- 取消后停止派发新任务，并等待当前只读操作安全退出。
- 扫描结果带 adapter version、格式指纹和扫描时间。
- UI 显示“扫描到的逻辑空间”，不能在未生成 plan 前显示“可安全释放”。

### 4.7 隔离区事务与状态机

会话生命周期：

```text
Discovered
  → Planned
  → Revalidated
  → Quarantining
  → Quarantined
  → Restoring → Restored
  → Purging   → Purged

任何中断 → Interrupted（下次启动恢复审计）
任何不确定 → Blocked（源文件保持不变）
```

manifest 使用版本化格式，例如：

```json
{
  "schema_version": 1,
  "operation_id": "uuid",
  "adapter": "codex",
  "adapter_version": "semver",
  "created_at": "RFC3339",
  "state": "quarantined",
  "selected_sessions": ["session-key"],
  "mutation_kind": "rewrite_shared_jsonl",
  "resources": [
    {
      "source": "platform-native-path",
      "container_before": "blake3:...",
      "container_after": "blake3:...",
      "removed_payload": "relative-path-only",
      "removed_length": 123,
      "removed_hash": "blake3:...",
      "unselected_record_set_hash": "blake3:..."
    }
  ]
}
```

写入顺序必须是：生成 plan → 重新验证源 → 写临时 manifest → 移动/复制资源 → 校验 → 原子提交 manifest。只有 committed manifest 中的条目可以出现在“隔离区”页面。永久清除默认不由定时后台任务自动触发。

#### 4.7.1 独占文件与共享容器的不同处理

**独占资源**

仅当资源引用图证明文件、目录或附件只属于所选 session，才允许把整个资源移动到 quarantine。用户选择 3 个 session，就只能出现这 3 个 session 的独占资源，不能把它们的父目录整体加入 plan。

**共享 JSONL/记录文件**

按需删除需要重写容器，但不是删除容器：

1. 记录原文件的长度、mtime、哈希和格式指纹并确认空闲。
2. 逐条流式解析；未选中 session 的原始记录原样写入同目录临时文件。
3. 仅把选中 session 的原始记录写入 quarantine payload，供恢复使用。
4. `fsync` 临时文件与 quarantine payload。
5. 验证选中记录全部移除、未选中记录集合/顺序未变、记录总数守恒。
6. 再次确认源文件 fingerprint 未变化。
7. 使用平台原子 replace 提交新容器；失败时保留原文件。
8. 提交 manifest 后才清理事务临时备份。

如果格式包含跨记录偏移、校验和、尾部索引或无法重建的引用，适配器必须实现并验证相应重建逻辑；否则标记不支持。不能简单按字符串匹配删除行。

恢复共享 JSONL 时只合并 quarantine 中被移除的记录，不用旧容器覆盖当前容器。适配器必须验证 session ID 冲突、格式版本与插入位置；任何不确定都停止恢复。

**共享 SQLite**

- 优先使用官方 session delete/archive 接口。
- 直接操作数据库时必须确认 schema fingerprint、外键/关联表、触发器、WAL 状态和无活动写入者。
- 在单个事务中只删除所选 session 的主记录、明确归属的从记录和索引行。
- 事务提交前检查未选中 session 的行数与关键摘要没有变化。
- SQLite 数据库文件、`-wal`、`-shm` 都不能因为删除一个 session 被整文件删除。
- `VACUUM` 只作为独立的“压缩数据库”操作：默认关闭、明确告知会重写整个数据库、要求空闲与临时空间，且不能与 session 删除混成一个按钮。
- 如果无法可靠导出并恢复被选中行，则 P1 不开放该格式的直接数据库删除。

**官方 API/命令**

调用前后都要读取 session 列表，验证只有选择集发生变化。官方操作如果只支持“清空全部”，不符合本产品的粒度要求，不能调用。

#### 4.7.2 粒度守恒不变量

一次操作选择集合为 `S`，执行前全部 session 为 `A`。成功隔离后必须满足：

```text
active_after = A - S
quarantine_added = S
unselected_changed = empty
unknown_side_effects = empty
```

除了适配器明确声明且在预览中展示的派生缓存，不允许修改 `A - S` 的任何资源。验证不变量失败时操作状态为 `Interrupted/Blocked`，不得向用户显示“清理成功”。

### 4.8 桌面 UI 页面与状态

v1 桌面端必须具备这些真实页面，而不是官网 mockup：

1. **首次启动**：语言、隐私承诺、扫描范围说明。
2. **Overview**：Agent 数、session 数、逻辑空间、上次扫描时间。
3. **Sessions**：虚拟列表、排序、筛选、多选、详情抽屉。
4. **Review plan**：选中的 session、每项删除粒度（独占文件/共享 JSONL/SQLite/API）、共享容器中不会被触碰的 session 数、blocked 条目、预计空间、隔离期限。
5. **Quarantine**：恢复、查看 manifest、单独永久清除。
6. **Settings**：语言、隔离目录、保留天数、更新、诊断。
7. **About**：版本、commit、许可证、数据目录、安全联系入口。

应用状态只能通过领域命令变化：

```text
UiIntent → Command → app-core use case → Event → AppState → egui render
```

扫描和文件操作不得在 `eframe::App::update` 中同步执行。UI 每帧只消费有限数量事件，防止扫描结果洪峰卡住渲染。

### 4.9 配置与本地数据路径

应用不得把自己的数据写进 Codex 或 Claude Code 目录。默认位置：

| 平台 | 配置/状态 | 隔离区 |
| --- | --- | --- |
| Windows | `%LOCALAPPDATA%\AgentChatCleaner\` | `%LOCALAPPDATA%\AgentChatCleaner\quarantine\` |
| macOS | `~/Library/Application Support/AgentChatCleaner/` | 同目录下 `quarantine/` |
| Linux | `$XDG_STATE_HOME/agent-chat-cleaner/`，否则 `~/.local/state/...` | 同目录下 `quarantine/` |

配置文件包含 schema version。配置损坏时备份为 `.corrupt` 并回退安全默认值，不能覆盖第三方数据。便携版如支持自定义数据目录，必须使用明确的 `--data-dir`，不能根据当前工作目录隐式决定。

### 4.10 二进制体积与构建 profile

`desktop/Cargo.toml` 的首版 release profile：

```toml
[profile.release]
opt-level = "s"
lto = "fat"
codegen-units = 1
panic = "abort"
strip = "symbols"

[profile.release-fast]
inherits = "release"
opt-level = 3
```

两个 profile 都要跑真实 benchmark。体积目标不是验收的唯一指标：

| 指标 | P0 目标 | 阻断阈值/处理 |
| --- | --- | --- |
| Windows portable zip | ≤ 15 MB 目标 | > 25 MB 必须出依赖体积报告 |
| 冷启动到首帧 | < 300 ms 目标 | > 800 ms 阻断首发 |
| 空闲 RSS | < 50 MB 目标 | > 100 MB 阻断首发 |
| 空闲 CPU | 接近 0 | 持续 > 1% 阻断首发 |
| 10k fixture 扫描 | < 2 s 目标（SSD） | 记录机器基线后判定 |
| 10k 列表滚动 | 60 FPS 目标 | 明显卡顿阻断首发 |

所有数字必须在固定硬件说明下报告，不能把 Debug 构建和 Release 构建混比，也不能拿单次结果做营销承诺。

### 4.11 测试金字塔

```text
单元测试
  路径规范化、规则、大小计算、状态机、manifest 编解码

适配器契约测试
  黄金样本、未知字段、格式升级、截断 JSONL、busy SQLite
  同一大文件含多个 session，只删除选择集并保持其他记录

集成测试
  同盘移动、跨盘复制校验、共享 JSONL 原子重写、SQLite 行级事务
  冲突恢复、中途断电模拟、幂等重试、选择集守恒

平台测试
  Windows junction/锁文件、macOS symlink/权限、Linux XDG/权限

UI 测试
  中英切换、危险确认、10k 虚拟列表、150% 缩放、键盘操作
```

fixture 只能使用合成或完成脱敏审计的数据。禁止把真实用户 session 提交进 Git。

永久删除相关代码要求 property test 或故障注入覆盖以下点：任一步骤失败不丢源文件；重复执行不扩大影响范围；manifest 与资源不一致时 fail closed。

必须存在以下最小回归样例：

- 一个 1 GB JSONL 包含 100 个 session，选择其中 1 个后，容器仍存在，另外 99 个 session 可读取且记录摘要不变。
- 一个 SQLite 包含 3 个 session 及关联索引，删除 1 个后另外 2 个仍可恢复/继续使用，数据库文件仍存在。
- 一个目录包含已选和未选 session 的独占文件，plan 不得把共同父目录作为删除目标。
- granular capability 为 unsupported 时，UI 没有“强制删除整个文件”入口。

### 4.12 CI 与发布流水线

每个 PR：

```powershell
cargo fmt --manifest-path desktop/Cargo.toml --all -- --check
cargo clippy --manifest-path desktop/Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path desktop/Cargo.toml --workspace --all-targets
cargo deny --manifest-path desktop/Cargo.toml check
npm run lint
npm test
```

每个 tag `desktop-vX.Y.Z`：

1. 在干净 runner 从 tag 构建，禁止从开发者工作区打包。
2. 生成原始二进制、安装包、SHA-256、SBOM 和第三方许可证清单。
3. Windows 使用受保护证书签名 `.exe`/`.msi` 并验证签名。
4. macOS 签名、hardened runtime、公证、staple，再验证 Gatekeeper。
5. Linux 产物使用项目 release key 签名校验文件。
6. 上传 staging，安装/卸载/升级冒烟测试通过后才提升到 production。
7. 发布清单自身做 Ed25519 签名；客户端验证清单与安装包哈希。

CI secret 只能用于受保护 tag 环境，PR 和 fork 无法读取代码签名证书或更新清单签名私钥。

### 4.13 版本与更新策略

- 应用版本使用 SemVer；适配器格式支持版本单独记录在扫描报告。
- `desktop-v1.0.0` 与官网版本分开打 tag。
- 已下载的版本无需联网或激活即可持续运行。
- 自动更新默认只检查签名元数据，下载前展示版本和大小。
- 清理事务进行中时禁止安装更新。
- 至少保留最近两个签名稳定版本供回滚。
- 安全问题可以停止分发旧安装包，但不得远程让已安装版本无法启动；高危格式问题通过 signed denylist 禁用对应适配器，而不是禁用整个应用。

### 4.14 Rust P0 Definition of Done

以下全部满足，才可以在文档状态中把“Rust 桌面端”从“未实现”改成“技术原型完成”：

- [x] `desktop/Cargo.toml` 和 `desktop/Cargo.lock` 已生成并纳入交付。
- [x] `cargo build --release` 在 Windows x64 成功。
- [x] `agent-chat-cleaner.exe --version` 输出版本与 commit。
- [x] GUI 以 `--safe-mode` 启动冒烟 5 秒后正常关闭，无后台残留。
- [x] 能只读扫描 Codex、Claude Code 的合成 fixture。
- [ ] 能显示每个 session 是独占资源还是共享容器，并生成对应粒度 plan。
- [ ] 未知格式、活动文件、目录链接全部在 UI 显示 blocked（当前均不会获得写权限，但部分未知文件会直接跳过）。
- [ ] 能生成但不能执行 P0 `OperationPlan`。
- [x] 临时 fixture 单元测试、适配器测试、源文件变化与恢复冲突测试通过；更完整的 junction 与故障注入仍属于公开版门禁。
- [ ] 记录真实 `.exe`/zip 体积、冷启动、RSS、CPU 和 10k 扫描结果。
- [ ] 生成 Windows portable zip、SHA-256、SBOM 与许可证清单（当前已有 EXE 与 SHA-256，尚缺 zip/SBOM/许可证清单）。
- [x] README 明确写明原型的选择性隔离、安全边界与测试不接触真实用户目录。

P0 完成后，才能进入 P1 实现隔离和恢复。永久清除必须是最后实现、最后开放的能力。

## 5. 免费分发策略

当前阶段产品免费使用。官网不使用“限免”“原价”“首发价”或付费对比等营销文案，也不需要专门强调免费；用户从官网直接下载并运行即可。

### 产品行为

- 无账号、无登录、无授权码、无设备绑定。
- 无支付宝、微信支付、订单、D1 或 license Worker。
- 无 DRM、联网激活、试用倒计时或功能锁。
- 离线状态下除检查更新外全部功能正常。
- 当前阶段不预留隐藏收费开关；未来若改变模式，需要重新评审产品、隐私和兼容策略。

### 下载与更新

```text
Cloudflare 静态官网
  → Windows 下载链接
  → 签名 EXE/MSI 或 portable zip
  → 同页展示版本、发布日期、大小、SHA-256

静态 update-manifest.json
  → 版本、下载 URL、SHA-256、发行说明
  → Ed25519 签名
  → 客户端本地验签后才提示更新
```

- 官网静态资源继续部署在 Cloudflare。
- 正式二进制可使用 GitHub Releases 或 R2 存储；官网只引用稳定下载 URL。
- P0 内部原型可直接交付未签名 portable EXE，但必须明确“内部测试”；公开发布前仍需要代码签名。
- 更新检查最多发送应用版本、平台和架构，不发送 session、路径、设备指纹或项目数据。
- 更新检查失败不影响应用启动和本地功能。

### 运营成本边界

免费不等于可以忽略维护成本。首发只承诺 Windows x64、Codex 与 Claude Code 两个适配器；新增平台和 Agent 以安全测试能力为准。官网不承诺响应时限、终身更新或尚未实现的 roadmap 日期。

## 6. 隐私、安全、合规与分发

### 隐私承诺要可验证

- 扫描、筛选、预览、隔离和恢复全部在本机完成。
- 默认不收集聊天正文、文件路径、项目名、prompt 或命令内容。
- 崩溃报告必须去掉路径、用户名和消息正文，默认征得用户同意。
- 应用日志只记录适配器名、错误码、耗时和计数；提供一键导出前预览。
- 官网分析优先使用无 Cookie 的最小化统计，并在隐私政策说明。

### 发版前必须有

- 隐私政策，明确扫描范围、日志字段与更新检查数据。
- 软件许可协议与服务条款。
- 第三方商标声明：与 OpenAI、Anthropic 等无隶属或背书关系。
- Windows 代码签名与 SmartScreen 策略。
- macOS Developer ID 签名、公证与 hardened runtime。
- 下载文件的 SHA-256、签名更新清单和回滚版本。
- 安全联系方式和漏洞响应流程。

自动更新必须验证签名，不能只信任 HTTPS。更新服务器被攻破不应导致任意二进制被安装。

## 7. 中英双语策略

- `zh-CN` 与 `en` 使用相同 key，CI 检查缺失和多余 key。
- 日期、数字、文件大小、复数和标点本地化，不能简单替换字符串。
- 危险确认用动作结果描述，如“将 28 个会话移入隔离区”，不要只写“确定吗”。
- 中文默认文案强调“本地、可恢复、按会话选择”；英文强调 “local-only, reversible, session-level selection”。
- 官网提供手动语言切换并记忆选择，不强制根据 IP 跳转。
- 桌面端首次跟随 OS locale，找不到翻译时回退英文并记录缺失 key。

## 8. 竞品与替代方案

| 类别/产品 | 主要能力 | 优势 | 空缺/机会 |
| --- | --- | --- | --- |
| Codex 官方 archive/delete | 单个 session 生命周期管理 | 官方、格式最可靠 | 跨项目盘点、空间统计和批量安全规则仍弱 |
| Claude Code session 管理 | resume、rename、fork、导出 | 与 Claude 工作流深度结合 | 不统一管理其他 Agent，清理体验不是核心 |
| ChatGPT Bulk Delete 等浏览器扩展 | 网页聊天多选删除 | 上手快、很多免费 | 面向云端 ChatGPT，不处理本地编程 Agent；扩展权限带来信任成本 |
| Recap（JetBrains 插件） | 多 Agent 历史查看器 | IDE 内统一查看、本地优先 | 重点是回顾/搜索，不是跨平台空间清理与恢复 |
| LLMnesia / Chat Locker / Æfusio | 搜索、导入、归档 AI 历史 | 支持来源多，信息再利用强 | 更偏“保留和检索”，与安全清理互补也竞争 |
| 手工删除/PowerShell/shell 脚本 | 免费、可定制 | 极轻量 | 容易删错、格式变更脆弱、没有预览和恢复保障 |

### 差异化信息架构

官网和产品都应围绕三个词：

1. **Local**：所有聊天留在本机。
2. **Reversible**：先隔离、可恢复。
3. **Universal**：一个界面整理多个 Agent。

不要把“Rust”放在主标题。用户需要的是放心清理；Rust 作为性能和轻量的证明放在第二层。

## 9. 官网结构

单页首发官网包含：

1. 顶部导航：原理、功能、安全、FAQ、语言切换与下载。
2. Hero：本地 Agent 会话太多？看清楚，再清干净。
3. 产品界面展示：空间、来源、规则、隔离区状态。
4. 三步流程：扫描 → 审核 → 隔离/恢复。
5. 安全承诺：不上传、运行中保护、格式未知保护、可恢复。
6. 竞品差异：跨 Agent + 可恢复，而不是单纯批删。
7. 下载：Windows x64 EXE、版本、大小、SHA-256 与安全提示。
8. FAQ：是否上传、支持平台、如何恢复、与脚本的差异、后续版本。
9. 页脚：隐私、条款、商标声明和联系入口。

下载按钮只能指向实际通过构建门禁的二进制；没有产物时不得伪装成可下载。

## 10. 验收标准

### 安全

- 任何格式未知或活动中的 session 都无法被批量操作。
- 删除一个共享容器中的 session 不会删除容器，也不会修改其他 session。
- 适配器不支持记录级/事务级删除时，该 session 只能查看，不能退化为整文件删除。
- 随机中断复制/移动/写 manifest，重启后可检测并安全恢复。
- 目录逃逸、symlink、junction 和路径冲突测试全部通过。
- 同一操作重复执行不会重复删除或破坏 manifest。

### 性能

- 10,000 个会话扫描期间 UI 保持响应，可随时取消。
- 列表滚动不随总行数线性降低帧率。
- 空闲 CPU 近似为 0，不驻留后台轮询文件系统。

### 产品

- 新用户不看文档即可完成扫描、预览、隔离、撤销。
- 用户能区分“归档”“隔离”“永久删除”。
- 中英文在 100%/125%/150% 缩放下无关键文案截断。
- 离线状态可使用全部本地整理、隔离与恢复功能。

## 11. 里程碑与实际交付物

时间是初始估算，安全测试未通过时不得为了日期压缩阶段。每个阶段都以仓库和构建产物验收，不以截图或文案验收。

### P-1：文档与官网（当前阶段）

已完成：

- 产品定位、风险模型、竞品和免费分发方案。
- Rust workspace、二进制、构建、测试和发布规格。
- 中英文官网与下载入口。

未完成：

- 代码签名安装包与跨平台产物。
- 共享 JSONL/SQLite 的安全写入支持。

### P0：Rust 安全技术原型（当前已交付一个可运行切片）

仓库交付：

- `desktop/` Cargo workspace 和已锁定依赖。
- egui/eframe 原生 GUI。
- Codex/Claude Code 合成 fixture 与只读适配器。
- 后台扫描、搜索、虚拟列表、安全状态与选择预览。
- path boundary、unknown format、busy/active 测试。

二进制交付：

- Windows x64 `agent-chat-cleaner.exe`。
- portable EXE 与 SHA-256；zip、SBOM、许可证清单待公开稳定版补齐。
- `--version`、`--data-dir`、`--safe-mode` 可用。

安全边界：

- 只允许经验证的单会话独占文件进入选择集。
- 支持明确确认后的可恢复隔离与无覆盖恢复；不提供永久删除。
- 共享 JSONL、SQLite、未知格式、活动文件和 `subagents` 保持只读。

### P1：可恢复清理 MVP（预计 2–3 周）

仓库交付：

- 版本化 quarantine manifest。
- 同盘原子移动、跨盘校验复制、恢复冲突处理。
- 共享 JSONL 的选择性流式重写与记录级恢复。
- SQLite 选择性行级事务；数据库压缩作为独立可选操作。
- 选择集守恒测试：未选 session 和共享容器不得被误删。
- 启动时审计 interrupted operation。
- 故障注入、幂等与恢复测试。
- 中英危险文案和键盘可访问性。

二进制交付：

- Windows portable zip。
- 未签名内部测试 `.msi`，仅供受控测试。
- 可安装、升级、卸载的冒烟测试报告。

安全边界：

- 默认只移动到隔离区。
- 永久清除仍用 feature flag 关闭。

### P1.5：永久清除安全审计（预计 1 周）

- 开放单条/批量永久清除前的独立确认。
- 仅允许 committed quarantine 资源被清除。
- 断电、磁盘满、权限变化、杀进程和重复执行测试。
- 由第二人或独立审查轮次检查所有 destructive path。

没有审查通过，不开放永久清除。

### P2：公开 Windows v1（预计 2–3 周）

仓库交付：

- 隐私、许可和商标正式页面。
- 签名更新清单、回滚与 adapter denylist。

发布交付：

- 已签名 Windows `.exe`/`.msi`。
- 安装包 SHA-256、SBOM、第三方许可证。
- 官网真实下载链接与校验信息。

### P3：macOS/Linux 与更多 Agent

- macOS `.app`/`.dmg`、Developer ID 签名、公证。
- Linux AppImage/`.deb` 与签名校验。
- 新 Agent 适配器 SDK。
- 自定义规则模板、空间趋势和更多恢复工具。

## 12. Go / No-Go 检查

以下任一项未完成，不应标记为公开稳定版：

- [ ] Windows Release 二进制、签名安装包和哈希可实际下载。
- [ ] 干净机器安装、升级、卸载和启动测试通过。
- [ ] 官网展示的功能全部存在于同版本真实二进制，而不是 mockup。
- [ ] 两个 Agent 的真实版本样本和升级兼容测试。
- [ ] 共享大文件/数据库的选择性删除测试通过；未选 session 摘要保持不变。
- [ ] 不支持细粒度删除的格式不会出现整文件删除降级入口。
- [ ] 隔离与恢复故障注入测试通过。
- [ ] 永久清除仅能作用于 committed quarantine，且完成独立安全审查。
- [ ] 代码签名和安全更新链路完成。
- [ ] 隐私政策、许可和商标声明上线。
- [ ] 官网不再展示占位下载或虚假用户评价。

## 参考（调研时间：2026-08-03）

- [Codex session archive/delete 改进讨论](https://github.com/openai/codex/issues/28675)
- [Codex CLI 当前源码中的 session delete 命令](https://github.com/openai/codex/blob/main/codex-rs/cli/src/main.rs)
- [Claude Code session 官方文档](https://code.claude.com/docs/en/sessions)
- [Slint 官方许可与桌面产品说明](https://slint.dev/blog/making-slint-desktop-ready)
- [egui 官方文档](https://docs.rs/egui/latest/egui/)
- [iced 官方文档与 experimental 声明](https://docs.iced.rs/iced/)
- [Cloudflare Pages Functions 计费](https://developers.cloudflare.com/pages/functions/pricing/)
