"use client";

import { useEffect, useState } from "react";

type Language = "zh" | "en";

const DOWNLOAD_URL = "/downloads/AgentChatCleaner-Windows-x64.exe";
const CHECKSUM_URL = "/downloads/AgentChatCleaner-Windows-x64.exe.sha256.txt";
const GITHUB_URL = "https://github.com/huzz-open/agent-chat-cleaner";

const copy = {
  zh: {
    nav: [["功能", "#features"], ["工作方式", "#workflow"], ["安全", "#safety"], ["下载", "#download"]],
    github: "GitHub",
    badge: "开源 · Windows x64 · v0.7.9",
    heroTitleA: "把散落的 Agent 会话，",
    heroTitleB: "整理成一张清楚的清单。",
    heroBody: "统一扫描 Cursor、ChatGPT / Codex 与 Claude Code。按项目、时间和大小找到不再需要的会话，批量处理，也能随时恢复。",
    primaryCta: "下载 Windows 版",
    secondaryCta: "查看工作方式",
    proof: ["纯 Rust 桌面端", "精确到单个会话", "删除进度实时可见"],
    mock: {
      menu: ["文件", "选项", "帮助"],
      sourceLabel: "会话来源：",
      source: "Cursor",
      scan: "扫描",
      existing: "现有会话 622",
      deleted: "已删除 4",
      all: "全选",
      invert: "反选",
      delete: "删除所选 (28)",
      search: "搜索任务标题、项目或会话 ID",
      columns: ["任务标题", "项目", "最后活动", "预估大小", "状态", "操作"],
      rows: [
        ["Project startup instructions", "todo_name", "45 分钟前", "88.2 KB", "locked", "删除"],
        ["修复 API 配置无法提交", "codex-proxy", "3 小时前", "24.6 MB", "ready", "删除"],
        ["聊天记录清理与恢复设计", "agent-chat-cleaner", "2 天前", "12.4 MB", "active", "删除"],
      ],
      progress: "正在处理 18 / 28 个会话",
      percent: "64%",
      locked: "已锁定，将在批量操作中保留",
      ready: "可以删除",
      active: "正在使用，暂不可删除",
    },
    strip: "一个便携 EXE，覆盖你常用的三个 AI 编程工具。",
    featuresEyebrow: "真正面向会话的清理工具",
    featuresTitle: "不仅看到占用，更能准确处理",
    featuresBody: "不同 Agent 的存储方式并不相同。应用分别解析文件、项目记录和共享数据库，在同一个界面里给出一致的操作体验。",
    features: [
      ["01", "三个来源，一个列表", "切换 Cursor、ChatGPT / Codex 和 Claude Code；任务标题、真实项目路径、最后活动与预估大小一目了然。"],
      ["02", "共享大文件按会话处理", "Cursor 的共享 SQLite 数据库不会被整文件删除。应用只在事务中改写已验证、可明确归属的会话记录。"],
      ["03", "批量操作不卡住界面", "有限并发处理独立文件，Cursor 事务复用扫描快照；进度、当前阶段和已完成数量持续显示。"],
      ["04", "删除策略由你决定", "默认保留可恢复副本，也可在设置中关闭。删除后列表立即更新，无需等待一次完整重扫。"],
    ],
    workflowEyebrow: "三步完成",
    workflowTitle: "从占用到清理，路径很短",
    workflow: [
      ["扫描", "识别真实会话", "读取各 Agent 的会话索引、项目路径与存储占用，自动合并重复记录。"],
      ["筛选", "留下重要内容", "搜索任务或项目，查看置顶、活动、可删除等状态，再勾选需要处理的会话。"],
      ["处理", "恢复或永久删除", "批量删除时显示完整进度；保留副本的会话可恢复，也可以从已删除列表彻底移除。"],
    ],
    safetyEyebrow: "谨慎处理共享数据",
    safetyTitle: "只改变能够证明归属的内容",
    safetyBody: "删除工具不该靠猜。应用在执行前重新检查来源状态、文件指纹与进程占用；无法确认边界时，操作会被阻止。",
    safetyItems: [
      ["置顶保护", "Cursor pinned、Codex 置顶与 Claude Code 明确命名会显示锁定图标，并从批量删除中排除。"],
      ["活动保护", "Agent 正在运行、文件仍被占用或内容刚刚变化时，对应操作会自动禁用。"],
      ["事务写入", "Cursor 数据在 SQLite 事务中校验并更新；失败会回滚，不留下处理一半的数据库。"],
      ["恢复清单", "可恢复删除记录来源、会话 ID 与恢复数据；路径冲突时停止，不会静默覆盖现有内容。"],
    ],
    compareEyebrow: "为真实数据结构设计",
    compareTitle: "比手动找文件更可控",
    compareHeaders: ["能力", "手动 / 脚本", "Agent Chat Cleaner"],
    compareRows: [
      ["跨 Agent 会话清单", "逐个目录查找", "统一界面"],
      ["共享数据库单会话删除", "容易误删整库", "事务级处理"],
      ["置顶与活动状态", "需要自行判断", "图标提示并保护"],
      ["批量进度与即时列表更新", "通常不可见", "全程可见"],
      ["误操作恢复", "依赖额外备份", "按需保留副本"],
    ],
    downloadEyebrow: "Windows 便携版",
    downloadTitle: "下载后直接运行",
    downloadBody: "无需安装，也不需要额外运行时。首次启动选择数据存储根目录，之后即可扫描与整理会话。",
    edition: "Windows x64",
    version: "v0.7.9 · Portable EXE",
    included: ["Cursor、ChatGPT / Codex、Claude Code", "现有 / 已删除筛选与全文搜索", "置顶、活动、可删除等组合状态", "批量进度、恢复与永久删除", "中英文产品官网"],
    download: "下载 Agent Chat Cleaner",
    downloadNote: "Windows 10/11 x64 · 便携版 · 当前未签名",
    checksum: "查看 SHA-256 校验文件",
    faqEyebrow: "常见问题",
    faqTitle: "使用之前，你可能会问",
    faqs: [
      ["Cursor 的 20 多 GB 大文件会怎样处理？", "应用会统计 globalStorage、workspaceStorage 与项目记录的物理占用，但删除单个会话时只修改经过验证且属于该会话的数据，不会删除整个 state.vscdb。"],
      ["为什么删除后释放空间可能小于预估？", "Cursor 使用共享 SQLite 与内容块，逻辑删除不等于数据库文件立即缩小。界面会区分会话预估与来源总占用，避免把整库大小算给某一条会话。"],
      ["误删后可以恢复吗？", "可以。默认删除会保留恢复副本；你也可以在设置中关闭该策略，让之后的删除不进入已删除列表。"],
      ["为什么有些按钮不可用？", "置顶、正在使用、来源变化或关系无法确认的会话会被保护。将鼠标停在状态图标或操作上可以查看具体原因。"],
      ["Windows 为什么提示未知发布者？", "当前便携版尚未购买代码签名证书，因此 SmartScreen 可能提示未知发布者。你可以用页面提供的 SHA-256 校验下载文件。"],
    ],
    finalTitle: "看清每个会话，\n再决定留下什么。",
    finalBody: "为 Cursor、ChatGPT / Codex 和 Claude Code 准备的原生会话管理工具。",
    footerNote: "开源项目，与 OpenAI、Anthropic 或 Cursor 无隶属或背书关系。",
  },
  en: {
    nav: [["Features", "#features"], ["Workflow", "#workflow"], ["Safety", "#safety"], ["Download", "#download"]],
    github: "GitHub",
    badge: "Open source · Windows x64 · v0.7.9",
    heroTitleA: "Turn scattered agent sessions",
    heroTitleB: "into one clear workspace.",
    heroBody: "Scan Cursor, ChatGPT / Codex, and Claude Code in one place. Find stale sessions by project, activity, and size—then process them in batches or restore them later.",
    primaryCta: "Download for Windows",
    secondaryCta: "See how it works",
    proof: ["Native Rust desktop app", "Per-session precision", "Live batch progress"],
    mock: {
      menu: ["File", "Options", "Help"], sourceLabel: "Source:", source: "Cursor", scan: "Scan",
      existing: "Existing 622", deleted: "Deleted 4", all: "Select all", invert: "Invert", delete: "Delete selected (28)", search: "Search title, project, or session ID",
      columns: ["Task", "Project", "Last active", "Est. size", "Status", "Actions"],
      rows: [["Project startup instructions", "todo_name", "45 min ago", "88.2 KB", "locked", "Delete"], ["Fix API configuration submit", "codex-proxy", "3 hours ago", "24.6 MB", "ready", "Delete"], ["Chat cleanup and recovery design", "agent-chat-cleaner", "2 days ago", "12.4 MB", "active", "Delete"]],
      progress: "Processing 18 / 28 sessions", percent: "64%", locked: "Pinned and protected from bulk actions", ready: "Ready to delete", active: "In use and temporarily protected",
    },
    strip: "One portable executable for the three AI coding tools you use most.",
    featuresEyebrow: "Session cleanup, properly modeled",
    featuresTitle: "See storage. Act with precision.",
    featuresBody: "Every agent stores chats differently. Dedicated adapters parse files, project records, and shared databases while keeping the workflow consistent.",
    features: [["01", "Three sources, one list", "Switch between Cursor, ChatGPT / Codex, and Claude Code with real titles, project paths, activity, and size."], ["02", "Per-session shared database edits", "Cursor's shared SQLite database is never deleted wholesale. Only verified records are changed inside a transaction."], ["03", "Responsive batch operations", "Bounded concurrency handles independent files while Cursor reuses scan snapshots. Stage, progress, and counts stay visible."], ["04", "Choose your deletion policy", "Keep recoverable copies by default, or disable them in settings. Successful rows disappear immediately without a full rescan."]],
    workflowEyebrow: "Three steps",
    workflowTitle: "A short path from storage to done",
    workflow: [["Scan", "Resolve real sessions", "Read each agent's index, project paths, and storage footprint while merging duplicate records."], ["Filter", "Keep what matters", "Search by task or project, review pinned and active states, then select only what you want to process."], ["Process", "Restore or purge", "Watch batch progress. Recover saved sessions later, or permanently remove entries from the deleted list."]],
    safetyEyebrow: "Careful with shared data",
    safetyTitle: "Change only what can be attributed",
    safetyBody: "A cleanup tool should never guess. Source state, fingerprints, and running processes are rechecked before mutation; uncertain boundaries stop the operation.",
    safetyItems: [["Pinned protection", "Cursor pinned, Codex pinned, and explicitly named Claude sessions show a lock and stay out of bulk deletion."], ["Activity protection", "Actions are disabled while an agent is running, a file is busy, or source data has just changed."], ["Transactional writes", "Cursor updates are verified in a SQLite transaction. Failures roll back instead of leaving a half-edited database."], ["Recovery manifest", "Recoverable deletes record source and session data. Path conflicts stop safely without silent overwrites."]],
    compareEyebrow: "Built for real storage formats", compareTitle: "More control than hunting through folders", compareHeaders: ["Capability", "Manual / script", "Agent Chat Cleaner"],
    compareRows: [["Cross-agent session list", "Search every folder", "One interface"], ["Per-session shared DB deletion", "Risk deleting the database", "Transactional"], ["Pinned and active state", "Judge it yourself", "Visible and protected"], ["Batch progress and instant updates", "Usually hidden", "Always visible"], ["Mistake recovery", "Bring your own backup", "Optional recovery copies"]],
    downloadEyebrow: "Portable for Windows", downloadTitle: "Download and run", downloadBody: "No installer and no extra runtime. Choose a data root on first launch, then scan and organize your sessions.", edition: "Windows x64", version: "v0.7.9 · Portable EXE",
    included: ["Cursor, ChatGPT / Codex, and Claude Code", "Existing / deleted filters and search", "Combined pinned, active, and deletable states", "Batch progress, recovery, and permanent deletion", "Bilingual product website"],
    download: "Download Agent Chat Cleaner", downloadNote: "Windows 10/11 x64 · portable · currently unsigned", checksum: "View SHA-256 checksum",
    faqEyebrow: "FAQ", faqTitle: "Before you start",
    faqs: [["What happens to Cursor's multi-gigabyte database?", "The app measures physical storage across globalStorage, workspaceStorage, and projects, but a single-session delete changes only verified records for that session. It never removes state.vscdb wholesale."], ["Why can reclaimed space be lower than the estimate?", "Cursor shares SQLite pages and content blocks. A logical delete does not immediately shrink the database, so session estimates and total source storage are shown separately."], ["Can I recover a deleted session?", "Yes. Deletes keep a recovery copy by default. You can disable that setting for future deletions so they do not enter the deleted list."], ["Why are some actions disabled?", "Pinned, active, changed, or uncertain sessions are protected. Hover a status icon or action to see the exact reason."], ["Why does Windows show Unknown publisher?", "The portable build is not code-signed yet, so SmartScreen may warn. Use the published SHA-256 checksum to verify the download."]],
    finalTitle: "See every session.\nChoose what stays.", finalBody: "A native session manager for Cursor, ChatGPT / Codex, and Claude Code.", footerNote: "Open source. Not affiliated with or endorsed by OpenAI, Anthropic, or Cursor.",
  },
} as const;

function GithubIcon() {
  return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 2.2a10 10 0 0 0-3.16 19.49c.5.09.68-.22.68-.48v-1.88c-2.78.6-3.37-1.18-3.37-1.18-.45-1.16-1.11-1.47-1.11-1.47-.91-.62.07-.61.07-.61 1 .07 1.53 1.03 1.53 1.03.9 1.53 2.35 1.09 2.92.83.09-.65.35-1.09.64-1.34-2.22-.25-4.56-1.11-4.56-4.94 0-1.09.39-1.98 1.03-2.68-.1-.25-.45-1.27.1-2.64 0 0 .84-.27 2.75 1.02A9.55 9.55 0 0 1 12 7.01c.85 0 1.69.11 2.49.34 1.91-1.29 2.75-1.02 2.75-1.02.55 1.37.2 2.39.1 2.64.64.7 1.03 1.59 1.03 2.68 0 3.84-2.34 4.68-4.57 4.93.36.31.68.92.68 1.86v2.77c0 .27.18.58.69.48A10 10 0 0 0 12 2.2Z" /></svg>;
}

function StatusIcon({ type, label }: { type: string; label: string }) {
  return <span className={`status-icon ${type}`} title={label} aria-label={label}>{type === "locked" ? "▣" : type === "ready" ? "✓" : "◷"}</span>;
}

export default function Home() {
  const [language, setLanguage] = useState<Language>("zh");
  useEffect(() => {
    const saved = window.localStorage.getItem("acc-language") as Language | null;
    const preferred = saved === "zh" || saved === "en" ? saved : navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en";
    const frame = window.requestAnimationFrame(() => setLanguage(preferred));
    return () => window.cancelAnimationFrame(frame);
  }, []);
  useEffect(() => {
    document.documentElement.lang = language === "zh" ? "zh-CN" : "en";
    window.localStorage.setItem("acc-language", language);
  }, [language]);
  const t = copy[language];

  return <main id="top">
    <header className="site-header">
      <a className="brand" href="#top" aria-label="Agent Chat Cleaner home"><span className="brand-mark" aria-hidden="true"><i /><i /><i /></span><span>Agent Chat Cleaner</span></a>
      <nav className="desktop-nav" aria-label="Primary navigation">{t.nav.map(([label, href]) => <a key={href} href={href}>{label}</a>)}</nav>
      <div className="header-actions">
        <a className="github-link" href={GITHUB_URL} target="_blank" rel="noreferrer"><GithubIcon /><span>{t.github}</span></a>
        <button className="language-toggle" type="button" onClick={() => setLanguage(language === "zh" ? "en" : "zh")} aria-label="Switch language">{language === "zh" ? "EN" : "中"}</button>
        <a className="button small" href={DOWNLOAD_URL}>{t.primaryCta}</a>
      </div>
    </header>

    <section className="hero section-shell">
      <div className="hero-copy">
        <span className="eyebrow-pill"><span />{t.badge}</span>
        <h1>{t.heroTitleA}<br /><em>{t.heroTitleB}</em></h1>
        <p>{t.heroBody}</p>
        <div className="hero-actions"><a className="button" href={DOWNLOAD_URL}>{t.primaryCta}<b>↓</b></a><a className="text-link" href="#workflow">{t.secondaryCta}<span>→</span></a></div>
        <div className="proof-row">{t.proof.map((item) => <span key={item}><i>✓</i>{item}</span>)}</div>
      </div>

      <div className="product-stage" aria-label="Agent Chat Cleaner application preview">
        <div className="app-window">
          <div className="window-title"><div className="window-app"><span className="mini-logo">A</span>Agent Chat Cleaner</div><div className="window-controls"><i>—</i><i>□</i><i>×</i></div></div>
          <div className="app-menu">{t.mock.menu.map((item) => <span key={item}>{item}</span>)}</div>
          <div className="app-toolbar"><label>{t.mock.sourceLabel}</label><button>{t.mock.source}<span>⌄</span></button><button className="scan-button">{t.mock.scan}</button></div>
          <div className="table-tools"><div className="filter-tabs"><button className="active">{t.mock.existing}</button><button>{t.mock.deleted}</button></div><span className="divider" /><button>{t.mock.all}</button><button>{t.mock.invert}</button><button className="delete-selected">{t.mock.delete}</button><div className="mock-search">⌕ <span>{t.mock.search}</span></div></div>
          <div className="session-table">
            <div className="table-row table-head"><span>□</span>{t.mock.columns.map((column) => <span key={column}>{column}</span>)}</div>
            {t.mock.rows.map((row) => <div className="table-row" key={row[0]}><span>□</span><strong title={row[0]}>{row[0]}</strong><span title={row[1]}>{row[1]}</span><span>{row[2]}</span><span className="size">{row[3]}</span><span><StatusIcon type={row[4]} label={row[4] === "locked" ? t.mock.locked : row[4] === "ready" ? t.mock.ready : t.mock.active} /></span><button className={row[4] === "locked" ? "disabled" : ""}>{row[5]}</button></div>)}
          </div>
          <div className="batch-progress"><div className="progress-copy"><span>{t.mock.progress}</span><strong>{t.mock.percent}</strong></div><div className="progress-track"><i /></div></div>
        </div>
        <div className="float-card card-a"><span className="float-icon">↻</span><div><strong>{language === "zh" ? "删除后立即更新" : "Instant list updates"}</strong><small>{language === "zh" ? "无需完整重扫" : "No full rescan required"}</small></div></div>
        <div className="float-card card-b"><span className="float-icon lock">▣</span><div><strong>{language === "zh" ? "重要会话已保护" : "Important sessions protected"}</strong><small>{language === "zh" ? "置顶与活动状态自动识别" : "Pinned and active states detected"}</small></div></div>
      </div>
    </section>

    <div className="support-strip"><div className="agent-logos"><span>CURSOR</span><span>ChatGPT / CODEX</span><span>CLAUDE CODE</span></div><p>{t.strip}</p></div>

    <section id="features" className="content-section section-shell"><div className="section-intro"><span className="section-kicker">{t.featuresEyebrow}</span><h2>{t.featuresTitle}</h2><p>{t.featuresBody}</p></div><div className="feature-grid">{t.features.map(([number, title, body]) => <article key={number}><span>{number}</span><h3>{title}</h3><p>{body}</p></article>)}</div></section>
    <section id="workflow" className="workflow-section"><div className="section-shell"><div className="section-intro centered"><span className="section-kicker">{t.workflowEyebrow}</span><h2>{t.workflowTitle}</h2></div><div className="workflow-grid">{t.workflow.map(([title, subtitle, body], index) => <article key={title}><div className="step-number">0{index + 1}</div><div className="step-line" /><span>{title}</span><h3>{subtitle}</h3><p>{body}</p></article>)}</div></div></section>
    <section id="safety" className="safety-section section-shell"><div className="safety-copy"><span className="section-kicker">{t.safetyEyebrow}</span><h2>{t.safetyTitle}</h2><p>{t.safetyBody}</p><div className="safety-seal"><span>✓</span><div><strong>{language === "zh" ? "边界不明确时停止" : "Stops on uncertainty"}</strong><small>{language === "zh" ? "每次写入前重新校验" : "Revalidated before every write"}</small></div></div></div><div className="safety-list">{t.safetyItems.map(([title, body], index) => <article key={title}><span>{["▣", "◷", "↻", "⌁"][index]}</span><div><h3>{title}</h3><p>{body}</p></div></article>)}</div></section>
    <section className="compare-section"><div className="section-shell"><div className="section-intro centered"><span className="section-kicker">{t.compareEyebrow}</span><h2>{t.compareTitle}</h2></div><div className="compare-table"><div className="compare-row compare-head">{t.compareHeaders.map((item) => <span key={item}>{item}</span>)}</div>{t.compareRows.map((row) => <div className="compare-row" key={row[0]}>{row.map((cell, index) => <span key={cell} className={index === 2 ? "accent-cell" : ""}>{index === 2 && <i>✓</i>}{cell}</span>)}</div>)}</div></div></section>
    <section id="download" className="download-section section-shell"><div className="download-card"><div className="download-copy"><span className="section-kicker">{t.downloadEyebrow}</span><h2>{t.downloadTitle}</h2><p>{t.downloadBody}</p><ul>{t.included.map((item) => <li key={item}><span>✓</span>{item}</li>)}</ul></div><div className="download-box"><div className="windows-mark">⊞</div><strong>{t.edition}</strong><span>{t.version}</span><a className="button download-button" href={DOWNLOAD_URL}>{t.download}<b>↓</b></a><small>{t.downloadNote}</small><a className="checksum-link" href={CHECKSUM_URL}>{t.checksum}</a></div></div></section>
    <section className="faq-section section-shell"><div className="section-intro centered"><span className="section-kicker">{t.faqEyebrow}</span><h2>{t.faqTitle}</h2></div><div className="faq-list">{t.faqs.map(([question, answer]) => <details key={question}><summary>{question}<span>＋</span></summary><p>{answer}</p></details>)}</div></section>
    <section className="final-cta"><div className="section-shell"><div><h2>{t.finalTitle.split("\n").map((line) => <span key={line}>{line}</span>)}</h2><p>{t.finalBody}</p></div><div className="final-actions"><a className="button light" href={DOWNLOAD_URL}>{t.primaryCta}<b>↓</b></a><a className="github-final" href={GITHUB_URL} target="_blank" rel="noreferrer"><GithubIcon />{t.github}</a></div></div></section>
    <footer><div className="section-shell"><a className="brand footer-brand" href="#top"><span className="brand-mark" aria-hidden="true"><i /><i /><i /></span><span>Agent Chat Cleaner</span></a><p>{t.footerNote}</p><a className="footer-github" href={GITHUB_URL} target="_blank" rel="noreferrer"><GithubIcon />huzz-open/agent-chat-cleaner</a></div></footer>
  </main>;
}
