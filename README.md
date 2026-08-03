# Agent Chat Cleaner

Agent Chat Cleaner 是一个本地优先、可恢复的 AI 编程 Agent 会话整理工具。桌面端使用纯 Rust 与 egui/eframe 构建，不使用 WebView、Electron 或 Node 运行时。

当前技术预览支持 Windows x64，并识别 Codex 与 Claude Code 的 JSONL 会话。只有经验证为“一份文件只属于一个会话”、且最近 10 分钟没有写入的条目才能被选择。操作默认移入隔离区，不提供永久删除。

## 安全边界

- 默认只读扫描；只读取 JSONL 首行元数据、文件大小和修改时间。
- 不上传聊天正文、项目路径或扫描结果。
- 不跟随符号链接，不扫描 `subagents` 子目录。
- 活动会话、未知格式、共享 JSONL 和 SQLite 保持只读。
- 共享大文件不会因为删除一个会话而被整体删除。
- 隔离前复制并落盘，用 BLAKE3 校验副本，随后再次校验源文件指纹。
- 恢复遇到同名目标时停止，不覆盖现有文件。
- 自动化测试仅使用 `tempfile` 临时目录，不调用真实用户目录的变更接口。

## 直接运行

Windows 10/11 x64 用户可运行：

`artifacts/AgentChatCleaner-Windows-x64.exe`

命令行选项：

```powershell
AgentChatCleaner-Windows-x64.exe --version
AgentChatCleaner-Windows-x64.exe --safe-mode
AgentChatCleaner-Windows-x64.exe --data-dir D:\AgentChatCleanerData
```

`--safe-mode` 会禁用隔离与恢复按钮，只保留只读扫描。

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

```powershell
npm install
npm run lint
npm test
```

官网是独立的中英文 Web 项目，桌面端二进制不包含 Web 代码。

## 当前限制

- 桌面 UI 当前以中文为主；官网支持中文与英文。
- 技术预览 EXE 尚未做 Windows 代码签名。
- 共享 JSONL、SQLite 的会话级重写尚未开放，相关条目只读。
- 未实现 macOS、Linux 安装包和自动更新。

完整产品方案与风险审查见 `agent_chat_cleaner_0f9a8f56.plan.md`。
