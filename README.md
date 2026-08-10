# Agent Chat Cleaner

Agent Chat Cleaner 是一个面向 Windows 的 AI 编程 Agent 会话整理工具，支持预览、批量清理和恢复。桌面端使用纯 Rust 与 egui/eframe 构建，不使用 WebView、Electron 或 Node 运行时。

当前测试版为 v0.7.9，支持 Windows x64，并识别 ChatGPT/Codex、Claude Code 与 Cursor 的会话。桌面界面采用高密度浅色布局：标准菜单栏、会话来源工具栏、列表头筛选、顶部批量操作、垂直居中的搜索框、可调列宽、虚拟滚动行和轻量操作。删除、彻底删除及执行进度弹窗采用统一的居中内容、居中按钮组和一致间距；表单类弹窗保持便于阅读的左对齐。顶部不重复展示会话数和占用；关于窗口只保留标准软件信息。状态列使用可叠加图标分别表示保留锁定、可删除、正在使用、关系未知、路径阻止和可恢复，每个图标提供独立提示。Cursor 会合并所有 `workspaceStorage/*/state.vscdb` 中 `cursor/pinnedComposers` 的会话 ID；ChatGPT/Codex 桌面端从 `.codex-global-state.json` 的 `pinned-thread-ids` 识别侧边栏置顶，并兼容 `threads.is_pinned`。两者在真正删除前还会重新检查来源状态。Claude Code 没有普通会话 pin 字段，因此把用户通过 rename 设置的 `customTitle` 作为明确的命名保留信号并锁定。删除确认不要求输入确认文字；普通删除默认先进入可恢复存储，彻底删除只作用于选中的隔离副本。用户也可以在“删除与存储设置”中关闭恢复副本；此后新删除的会话永久删除，不进入“已删除”。应用只在首次进入某个 Agent 时扫描对应目录并缓存结果，不会启动时扫描全部来源。

删除成功后，应用会立即从当前列表移除已提交的会话，不再自动触发完整扫描；独占文件来源会精确增量调整空间统计，Cursor 只重新读取受影响的 `state.vscdb`/WAL 指纹，使后续删除仍使用最新快照。恢复会话会立即回插列表，并保留独占源文件原来的修改时间，因此无需等待扫描即可再次删除。只有本地重建失败时才回退完整扫描；Cursor 的物理占用统计因 SQLite 空闲页和共享容器无法由单条会话精确推导，按需点击“扫描”校准。

批量删除会持续上报阶段、已完成数量、百分比和当前会话。ChatGPT/Codex 与 Claude Code 的独占文件使用有界并发：保留恢复副本时最多 2 路，永久删除时最多 4 路，避免无限并发拖垮磁盘。Cursor 的会话共用同一个 SQLite 数据库，因此写事务保持串行；批次会复用置顶 ID 快照、进程状态和最新数据库指纹，不再为每个会话重复遍历全部 `workspaceStorage`，并每 32 条复核一次 Cursor 是否被重新启动。

ChatGPT/Codex 的任务标题优先读取 `.codex/session_index.jsonl` 中最新的 `thread_name`，与 Codex 侧边栏保持一致；搜索使用同一标题，因此不会再以冗长的首条提示词代替已经生成或重命名的任务名。Claude Code 按 `customTitle`、`aiTitle`、首条提示词的优先级显示，即使标题记录出现在首条提示词之后也会正确显示和搜索。

Cursor 会话会按 `composerId` 合并 `%APPDATA%\Cursor\User\globalStorage\state.vscdb` 与 `~/.cursor/projects` 中的 transcript，不会按存储位置生成重复行；顶层子代理不会独立混入主列表。顶部“占用”统计 globalStorage、workspaceStorage 和 project 文件的物理总量。删除前必须完全退出 Cursor；应用会把经验证的 `composerHeaders`、`composerData`、bubble、checkpoint、代码差异上下文和独占 transcript 写入单会话恢复 payload，再在 SQLite `IMMEDIATE` 事务中逐值复核并删除。未验证归属的 `agentKv` 内容寻址块、`composer.content` 共享块、Cursor 自动备份和搜索数据库暂不改写。

项目列不再尝试从 `d-code-inner-todo-name` 一类编码目录名反推路径。Cursor 从 `workspaceIdentifier.uri/configPath.fsPath`、Claude Code 从会话记录的 `cwd`、Codex 从 `session_meta.payload.cwd` 读取 Agent 自己保存的真实项目路径；列表显示真实项目名，hover 显示完整路径，并支持按完整路径搜索。Cursor 多根工作区显示 workspace 配置名称及其配置文件路径。

空间统计按来源区分：Cursor 顶部显示会话目录、globalStorage 与 workspaceStorage 的物理总量；Claude Code 只统计 `projects` 下的会话、子代理和附件，不把下载的程序或插件缓存算作聊天；Codex 只统计现有与归档会话 JSONL，不把 `logs_2.sqlite` 运行日志、程序文件或缓存冒充为可按会话释放的空间。Codex 的大 JSONL 是独占会话文件，可按单会话隔离；共享状态和日志数据库保持不改写。

## 安全边界

- 默认只读扫描；流式读取有限元数据和第一条用户提示以生成任务标题，不读取整份会话进内存。
- 不上传聊天正文、项目路径或扫描结果。
- 不跟随符号链接，不扫描 `subagents` 子目录。
- 活动会话和未知格式保持只读；Cursor 运行期间所有 Cursor 写操作均禁用。
- 共享大文件不会因为删除一个会话而被整体删除。
- 隔离前复制并落盘，用 BLAKE3 校验副本，随后再次校验源文件指纹。
- 恢复遇到同名目标时停止，不覆盖现有文件。
- 永久删除只允许作用于已提交隔离清单中的单个 `payload.jsonl`，保留审计 manifest；共享容器没有此入口。
- 自动化测试仅使用 `tempfile` 临时目录，不调用真实用户目录的变更接口。

## 直接运行

Windows 10/11 x64 用户可从官网下载安装包，仓库内对应发布文件为：

`public/downloads/AgentChatCleaner-Windows-x64.exe`

命令行选项：

```powershell
AgentChatCleaner-Windows-x64.exe --version
AgentChatCleaner-Windows-x64.exe --safe-mode
AgentChatCleaner-Windows-x64.exe --data-dir D:\AgentChatCleanerData
```

`--safe-mode` 会禁用删除、恢复与彻底删除按钮，只保留只读扫描。首次启动必须确认一个隔离根目录，应用会在预览中展示三个 Agent 子目录，确认前不会创建目录或开放写操作。之后可在顶部“设置”菜单中分别调整；默认优先使用非系统盘的 `AgentChatCleanerData` 目录，并在首次删除时才创建。

## 构建桌面端

需要 Rust stable 与 Windows MSVC toolchain：

```powershell
cargo test --manifest-path desktop/Cargo.toml --workspace
cargo clippy --manifest-path desktop/Cargo.toml --workspace --all-targets -- -D warnings
cargo build --manifest-path desktop/Cargo.toml --release --target x86_64-pc-windows-msvc
```

原始产物位于：

`desktop/target/x86_64-pc-windows-msvc/release/agent-chat-cleaner.exe`

## 官网

线上地址：<https://acc.huzz.top>

备用地址：<https://agent-chat-cleaner-site.huzzcj.workers.dev>

```powershell
npm install
npm run lint
npm test
npm run deploy:dry-run
npm run deploy
```

官网是独立的中英文 Web 项目，使用 Vinext 构建并部署到 Cloudflare Workers；桌面端二进制不包含 Web 代码。发布前需要先执行 `npx wrangler login` 登录 Cloudflare。

## 开源协议

项目采用 [MIT License](LICENSE)，公开仓库地址为 <https://github.com/huzz-open/agent-chat-cleaner>。

## 当前限制

- 桌面 UI 当前以中文为主；官网支持中文与英文。
- 技术预览 EXE 尚未做 Windows 代码签名。
- Cursor 当前只删除能够由 `composerId` 明确归属的直接记录；共享 CAS 内容块、自动备份、全文搜索缓存和数据库压缩仍保守保留，因此“逻辑删除大小”不等同于立即释放的物理空间。
- 未实现 macOS、Linux 安装包和自动更新。

完整产品方案与风险审查见 `agent_chat_cleaner_0f9a8f56.plan.md`。
