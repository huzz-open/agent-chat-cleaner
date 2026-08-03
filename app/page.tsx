"use client";

import { useEffect, useState } from "react";

type Language = "zh" | "en";

const copy = {
  zh: {
    nav: [
      ["功能", "#features"],
      ["工作方式", "#workflow"],
      ["安全", "#safety"],
      ["下载", "#download"],
    ],
    badge: "为本地 AI 编程 Agent 而生",
    heroTitleA: "会话太多？",
    heroTitleB: "看清楚，再清干净。",
    heroBody:
      "统一整理 Codex、Claude Code 等本地会话。先预览、再隔离、可恢复——你的聊天内容始终留在电脑里。",
    primaryCta: "下载 Windows 版",
    secondaryCta: "看看如何工作",
    proof: ["聊天内容不上云", "误清理可恢复", "原生 Rust 桌面端"],
    mock: {
      title: "会话空间",
      subtitle: "本机扫描完成 · 刚刚",
      reclaim: "可安全整理",
      selected: "已选择 28 个会话",
      action: "移入隔离区",
      filter: "90 天未使用",
      col: ["会话", "最后活动", "空间"],
      rows: [
        ["日志清理策略", "Codex · 4 个月前", "184 MB"],
        ["Landing page experiments", "Claude Code · 5 个月前", "92 MB"],
        ["旧版构建排查", "Codex · 7 个月前", "61 MB"],
      ],
      quarantine: "隔离区保留 7 天",
      undo: "可随时恢复",
    },
    strip: "一个轻量桌面端，统一管理散落在不同 Agent 里的本地会话。",
    featuresEyebrow: "不只是一个删除按钮",
    featuresTitle: "每一步都让你心里有数",
    featuresBody:
      "官方工具擅长管理自己的会话。Agent Chat Cleaner 负责跨工具盘点、筛选和安全清理。",
    features: [
      ["01", "空间一眼看清", "按 Agent、项目、日期和大小聚合，区分逻辑大小与预计可回收空间。"],
      ["02", "聪明但不自作主张", "用安全、平衡或自定义规则找出旧会话；格式未知和运行中的会话自动跳过。"],
      ["03", "先隔离，后清除", "清理默认进入隔离区并保留完整清单。恢复原路径，冲突时绝不静默覆盖。"],
      ["04", "隐私不是一句口号", "扫描、预览、筛选与恢复都在本机完成。应用不需要账号，聊天正文和项目路径不会上传。"],
    ],
    workflowEyebrow: "三步完成",
    workflowTitle: "放心整理，不靠运气",
    workflow: [
      ["扫描", "只读发现", "自动找到支持的 Agent 数据目录，流式读取，不修改任何内容。"],
      ["审核", "确认清单", "查看命中规则、最后活动、占用空间和风险状态，再决定整理哪些。"],
      ["整理", "隔离与恢复", "先移动到隔离区。默认保留 7 天，期间可以一键恢复。"],
    ],
    safetyEyebrow: "安全是产品本身",
    safetyTitle: "默认拒绝不确定的操作",
    safetyBody:
      "清理工具最重要的功能，是知道什么时候不该清理。任何变化、占用或未知格式都会让该条目停在原处。",
    safetyItems: [
      ["运行保护", "组合进程、文件占用与最近写入状态，活动会话无法批量操作。"],
      ["格式保护", "适配器无法确认格式版本时 fail closed，不把“解析失败”当垃圾。"],
      ["路径保护", "不跟随符号链接和目录联接，每个目标执行前重新校验边界。"],
      ["完整恢复", "隔离清单记录来源、路径、大小与哈希，执行中断也能安全核对。"],
    ],
    compareEyebrow: "为什么不是脚本",
    compareTitle: "快很重要，可逆更重要",
    compareHeaders: ["能力", "手动 / 脚本", "Agent Chat Cleaner"],
    compareRows: [
      ["跨 Agent 统一视图", "需要逐个查找", "一个界面"],
      ["运行中会话保护", "取决于脚本", "默认强制"],
      ["格式变更保护", "容易失效", "未知即跳过"],
      ["误操作恢复", "依赖备份", "内置隔离区"],
      ["空间与规则预览", "需要自己统计", "执行前可见"],
    ],
    downloadEyebrow: "Windows 技术预览",
    downloadTitle: "现在开始整理",
    downloadBody: "一个原生 Rust 可执行文件。启动后只读扫描，只有你明确选择并输入确认词时才会把单个会话移入隔离区。",
    edition: "Windows x64",
    version: "v0.1.0 · Portable EXE",
    included: [
      "Codex 与 Claude Code 支持",
      "只读扫描、筛选、隔离与恢复",
      "活动会话和未知格式自动保护",
      "共享大文件保持只读，绝不整文件删除",
      "中文桌面界面",
    ],
    download: "下载 Agent Chat Cleaner",
    downloadNote: "Windows 10/11 x64 · 便携版 · 当前未签名",
    checksum: "查看 SHA-256 校验文件",
    faqEyebrow: "常见问题",
    faqTitle: "清理之前，你可能会问",
    faqs: [
      ["会上传我的聊天记录吗？", "不会。扫描、筛选、预览、隔离和恢复都在本机完成；应用不需要账号。"],
      ["误删了怎么办？", "默认操作不是永久删除，而是移动到隔离区并保留 7 天。你可以在隔离区恢复到原位置；遇到路径冲突时应用会停下来让你选择。"],
      ["和手写脚本有什么不同？", "脚本很适合熟悉目录和格式的用户。这个工具增加了跨 Agent 适配、格式变化保护、运行状态判断、操作预览和可靠恢复。"],
      ["会直接删除整个会话文件夹吗？", "不会。当前版本只处理经验证且一个文件对应一个会话的 JSONL；共享大文件和数据库保持只读，不会退化成整文件删除。"],
      ["什么时候支持更多 Agent？", "首发先把 Codex 与 Claude Code 的安全边界做扎实，再根据用户需求加入 Cursor、Cline、Roo Code 等适配器。"],
    ],
    finalTitle: "把空间还给电脑，\n把上下文留给自己。",
    finalBody: "轻量、本地、可恢复。为每天和 AI Agent 一起工作的你准备。",
    footerNote: "本产品与 OpenAI、Anthropic 无隶属或背书关系。",
    footerLinks: ["隐私", "条款", "安全"],
  },
  en: {
    nav: [
      ["Features", "#features"],
      ["How it works", "#workflow"],
      ["Safety", "#safety"],
      ["Download", "#download"],
    ],
    badge: "Built for local AI coding agents",
    heroTitleA: "Too many sessions?",
    heroTitleB: "See clearly. Clean safely.",
    heroBody:
      "Tidy local Codex, Claude Code, and other agent sessions in one place. Preview first, quarantine safely, and restore anytime—your chats never leave your computer.",
    primaryCta: "Download for Windows",
    secondaryCta: "See how it works",
    proof: ["No chat uploads", "Recoverable cleanup", "Native Rust desktop app"],
    mock: {
      title: "Session storage",
      subtitle: "Local scan complete · just now",
      reclaim: "Safe to organize",
      selected: "28 sessions selected",
      action: "Move to quarantine",
      filter: "Unused for 90 days",
      col: ["Session", "Last active", "Space"],
      rows: [
        ["Payment callback refactor", "Codex · 4 months ago", "184 MB"],
        ["Landing page experiments", "Claude Code · 5 months ago", "92 MB"],
        ["Legacy build debugging", "Codex · 7 months ago", "61 MB"],
      ],
      quarantine: "Quarantine keeps files for 7 days",
      undo: "Restore anytime",
    },
    strip: "One lightweight desktop app for local sessions scattered across your coding agents.",
    featuresEyebrow: "More than a delete button",
    featuresTitle: "Know exactly what happens",
    featuresBody:
      "Official tools manage their own sessions. Agent Chat Cleaner gives you one safe workflow across tools.",
    features: [
      ["01", "See storage clearly", "Group by agent, project, age, and size, with honest estimates for reclaimable disk space."],
      ["02", "Smart, never reckless", "Use safe, balanced, or custom rules. Active and unknown-format sessions are skipped automatically."],
      ["03", "Quarantine before purge", "Cleanup writes a full manifest and moves files to quarantine. Restore without silent overwrites."],
      ["04", "Privacy by architecture", "Scan, preview, filter, and restore locally. No account is required, and chat content or project paths are never uploaded."],
    ],
    workflowEyebrow: "Three simple steps",
    workflowTitle: "Clean with confidence",
    workflow: [
      ["Scan", "Read-only discovery", "Find supported agent data folders and stream metadata without changing a thing."],
      ["Review", "Confirm the plan", "Check matched rules, last activity, storage, and risk status before taking action."],
      ["Organize", "Quarantine & restore", "Move to quarantine first. Files stay recoverable for 7 days by default."],
    ],
    safetyEyebrow: "Safety is the product",
    safetyTitle: "Uncertainty means stop",
    safetyBody:
      "A cleanup tool must know when not to clean. Changed files, active sessions, and unknown formats stay exactly where they are.",
    safetyItems: [
      ["Active-session guard", "Process, file-lock, and recent-write signals prevent active sessions from entering bulk actions."],
      ["Format guard", "Unknown data versions fail closed. A parse error is never treated as disposable data."],
      ["Path guard", "Symlinks and junctions are not followed, and every target is revalidated before execution."],
      ["Complete recovery", "Manifests record source, path, size, and hash so interrupted operations remain auditable."],
    ],
    compareEyebrow: "Why not a script?",
    compareTitle: "Fast matters. Reversible matters more.",
    compareHeaders: ["Capability", "Manual / script", "Agent Chat Cleaner"],
    compareRows: [
      ["Cross-agent overview", "Find each folder", "One workspace"],
      ["Active session protection", "Depends on script", "Always on"],
      ["Format change protection", "Brittle", "Unknown means skip"],
      ["Mistake recovery", "Bring your own backup", "Built-in quarantine"],
      ["Space & rule preview", "Calculate it yourself", "Visible before action"],
    ],
    downloadEyebrow: "Windows technical preview",
    downloadTitle: "Start organizing now",
    downloadBody: "One native Rust executable. It scans read-only and moves a session to quarantine only after you select it and type the confirmation phrase.",
    edition: "Windows x64",
    version: "v0.1.0 · Portable EXE",
    included: [
      "Codex and Claude Code support",
      "Read-only scan, filters, quarantine, and restore",
      "Active and unknown-format sessions are protected",
      "Shared containers stay read-only; never whole-file deletion",
      "Chinese desktop interface",
    ],
    download: "Download Agent Chat Cleaner",
    downloadNote: "Windows 10/11 x64 · portable · currently unsigned",
    checksum: "View SHA-256 checksum",
    faqEyebrow: "FAQ",
    faqTitle: "Before you clean",
    faqs: [
      ["Do you upload my chats?", "No. Scanning, filtering, previewing, quarantining, and restoring happen locally, and no account is required."],
      ["What if I clean the wrong session?", "Cleanup moves sessions to quarantine for 7 days by default. Restore them to the original location at any time; path conflicts always require your choice."],
      ["How is this different from a script?", "Scripts are great when you know every format and folder. This tool adds cross-agent adapters, format guards, active-session detection, previews, and reliable recovery."],
      ["Can it delete an entire shared container?", "No. This build mutates only verified one-session-per-file JSONL. Shared files and databases remain read-only instead of falling back to whole-file deletion."],
      ["When will more agents be supported?", "The first release focuses on getting Codex and Claude Code safety right. Cursor, Cline, Roo Code, and others follow based on demand."],
    ],
    finalTitle: "Give space back to your machine.\nKeep context in your hands.",
    finalBody: "Lightweight, local, and reversible. Made for people who work with AI agents every day.",
    footerNote: "Not affiliated with or endorsed by OpenAI or Anthropic.",
    footerLinks: ["Privacy", "Terms", "Security"],
  },
} as const;

export default function Home() {
  const [language, setLanguage] = useState<Language>("zh");

  useEffect(() => {
    const saved = window.localStorage.getItem("acc-language") as Language | null;
    const preferred = saved === "zh" || saved === "en"
      ? saved
      : navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en";
    const frame = window.requestAnimationFrame(() => setLanguage(preferred));
    return () => window.cancelAnimationFrame(frame);
  }, []);

  useEffect(() => {
    document.documentElement.lang = language === "zh" ? "zh-CN" : "en";
    window.localStorage.setItem("acc-language", language);
  }, [language]);

  const t = copy[language];

  return (
    <main>
      <header className="site-header">
        <a className="brand" href="#top" aria-label="Agent Chat Cleaner home">
          <span className="brand-mark" aria-hidden="true"><i /><i /><i /></span>
          <span>Agent Chat Cleaner</span>
        </a>
        <nav className="desktop-nav" aria-label="Primary navigation">
          {t.nav.map(([label, href]) => <a key={href} href={href}>{label}</a>)}
        </nav>
        <div className="header-actions">
          <button
            className="language-toggle"
            type="button"
            onClick={() => setLanguage(language === "zh" ? "en" : "zh")}
            aria-label={language === "zh" ? "Switch to English" : "切换到中文"}
          >
            <span className={language === "zh" ? "active" : ""}>中</span>
            <b>/</b>
            <span className={language === "en" ? "active" : ""}>EN</span>
          </button>
          <a className="header-buy" href="/downloads/AgentChatCleaner-Windows-x64.exe" download>{language === "zh" ? "下载 Windows 版" : "Download"}</a>
        </div>
      </header>

      <section className="hero" id="top">
        <div className="hero-copy">
          <div className="eyebrow-pill"><span />{t.badge}</div>
          <h1>{t.heroTitleA}<br /><em>{t.heroTitleB}</em></h1>
          <p className="hero-body">{t.heroBody}</p>
          <div className="hero-actions">
            <a className="button button-primary" href="/downloads/AgentChatCleaner-Windows-x64.exe" download>{t.primaryCta}<span>↓</span></a>
            <a className="button button-ghost" href="#workflow">{t.secondaryCta}<span>↓</span></a>
          </div>
          <div className="proof-list">
            {t.proof.map((item) => <span key={item}><b>✓</b>{item}</span>)}
          </div>
        </div>

        <div className="product-visual" aria-label={language === "zh" ? "产品界面预览" : "Product interface preview"}>
          <div className="visual-glow" />
          <div className="app-window">
            <div className="app-titlebar">
              <div className="traffic"><i /><i /><i /></div>
              <div className="app-brand"><span className="mini-mark" /> Agent Chat Cleaner</div>
              <span className="window-tag">LOCAL</span>
            </div>
            <div className="app-layout">
              <aside className="app-sidebar">
                <span className="sidebar-label">OVERVIEW</span>
                <div className="sidebar-item active"><i className="side-icon grid-icon" />{t.mock.title}<b>42</b></div>
                <div className="sidebar-item"><i className="side-icon archive-icon" />{language === "zh" ? "隔离区" : "Quarantine"}<b>7</b></div>
                <span className="sidebar-label sources">AGENTS</span>
                <div className="agent-source"><span className="agent-dot codex">C</span>Codex<b>26</b></div>
                <div className="agent-source"><span className="agent-dot claude">A</span>Claude Code<b>16</b></div>
                <div className="sidebar-safe"><i>✓</i><div><strong>{language === "zh" ? "本地模式" : "Local mode"}</strong><small>{language === "zh" ? "内容从不上传" : "Content never uploads"}</small></div></div>
              </aside>
              <div className="app-content">
                <div className="content-head">
                  <div><h3>{t.mock.title}</h3><p>{t.mock.subtitle}</p></div>
                  <button type="button">↻</button>
                </div>
                <div className="space-card">
                  <div><span>{t.mock.reclaim}</span><strong>1.24 <small>GB</small></strong></div>
                  <div className="donut"><span>68%</span></div>
                  <div className="space-legend"><i /><span>Codex <b>824 MB</b></span><i /><span>Claude Code <b>416 MB</b></span></div>
                </div>
                <div className="table-toolbar">
                  <div className="filter-chip"><span>◷</span>{t.mock.filter}<b>×</b></div>
                  <div className="search-box">⌕ <span>{language === "zh" ? "搜索会话" : "Search sessions"}</span></div>
                </div>
                <div className="session-table">
                  <div className="table-row table-header"><span><i className="checkbox checked" />{t.mock.col[0]}</span><span>{t.mock.col[1]}</span><span>{t.mock.col[2]}</span></div>
                  {t.mock.rows.map((row, index) => (
                    <div className="table-row" key={row[0]}>
                      <span><i className={`checkbox ${index < 2 ? "checked" : ""}`}>{index < 2 ? "✓" : ""}</i><span className={`file-icon tone-${index}`}>▤</span><strong>{row[0]}</strong></span>
                      <span>{row[1]}</span><span>{row[2]}</span>
                    </div>
                  ))}
                </div>
                <div className="selection-bar">
                  <div><span className="shield-icon">✓</span><p><strong>{t.mock.selected}</strong><small>{t.mock.quarantine} · {t.mock.undo}</small></p></div>
                  <button type="button">{t.mock.action}<span>→</span></button>
                </div>
              </div>
            </div>
          </div>
          <div className="floating-note note-top"><span>✓</span><div><strong>{language === "zh" ? "3 个活动会话已保护" : "3 active sessions protected"}</strong><small>{language === "zh" ? "不会进入批量操作" : "Excluded from bulk actions"}</small></div></div>
          <div className="floating-note note-bottom"><span>↶</span><div><strong>{language === "zh" ? "随时撤销" : "Undo anytime"}</strong><small>{language === "zh" ? "隔离区完整保留原路径" : "Original paths are preserved"}</small></div></div>
        </div>
      </section>

      <div className="trust-strip"><span className="strip-mark" />{t.strip}<span className="strip-mark" /></div>

      <section className="section features-section" id="features">
        <div className="section-heading split-heading">
          <div><p className="section-eyebrow">{t.featuresEyebrow}</p><h2>{t.featuresTitle}</h2></div>
          <p>{t.featuresBody}</p>
        </div>
        <div className="feature-grid">
          {t.features.map(([number, title, body], index) => (
            <article className={`feature-card feature-${index + 1}`} key={number}>
              <div className="feature-top"><span>{number}</span><i className={`feature-symbol symbol-${index + 1}`} /></div>
              <h3>{title}</h3><p>{body}</p>
            </article>
          ))}
        </div>
      </section>

      <section className="section workflow-section" id="workflow">
        <div className="section-heading centered">
          <p className="section-eyebrow">{t.workflowEyebrow}</p><h2>{t.workflowTitle}</h2>
        </div>
        <div className="workflow-grid">
          {t.workflow.map(([kicker, title, body], index) => (
            <article className="workflow-step" key={kicker}>
              <div className="step-number">0{index + 1}</div>
              <div className={`step-illustration step-${index + 1}`}>
                <div className="step-orb"><span>{index === 0 ? "⌕" : index === 1 ? "✓" : "↶"}</span></div>
                {index === 0 && <><i className="scan-line" /><b className="scan-dot one" /><b className="scan-dot two" /><b className="scan-dot three" /></>}
                {index === 1 && <><i className="review-line line-one" /><i className="review-line line-two" /><i className="review-line line-three" /></>}
                {index === 2 && <><i className="restore-arrow" /><b className="restore-box" /></>}
              </div>
              <p className="step-kicker">{kicker}</p><h3>{title}</h3><p className="step-copy">{body}</p>
            </article>
          ))}
        </div>
      </section>

      <section className="safety-section" id="safety">
        <div className="safety-inner">
          <div className="safety-copy">
            <p className="section-eyebrow light">{t.safetyEyebrow}</p><h2>{t.safetyTitle}</h2><p>{t.safetyBody}</p>
            <div className="local-chip"><span className="pulse-dot" />{language === "zh" ? "100% 本机处理" : "100% on-device"}</div>
          </div>
          <div className="safety-list">
            {t.safetyItems.map(([title, body], index) => (
              <article key={title}><span className="safety-number">0{index + 1}</span><div><h3>{title}</h3><p>{body}</p></div><i>✓</i></article>
            ))}
          </div>
        </div>
      </section>

      <section className="section compare-section">
        <div className="section-heading centered narrow">
          <p className="section-eyebrow">{t.compareEyebrow}</p><h2>{t.compareTitle}</h2>
        </div>
        <div className="compare-table" role="table">
          <div className="compare-row compare-head" role="row">
            {t.compareHeaders.map((item, index) => <span key={item} className={index === 2 ? "highlight" : ""}>{index === 2 && <i className="mini-mark" />}{item}</span>)}
          </div>
          {t.compareRows.map((row) => (
            <div className="compare-row" role="row" key={row[0]}>
              <strong>{row[0]}</strong><span className="muted-cell"><i>—</i>{row[1]}</span><span className="good-cell"><i>✓</i>{row[2]}</span>
            </div>
          ))}
        </div>
      </section>

      <section className="download-section" id="download">
        <div className="download-intro">
          <p className="section-eyebrow light">{t.downloadEyebrow}</p><h2>{t.downloadTitle}</h2><p>{t.downloadBody}</p>
        </div>
        <div className="download-card">
          <div className="download-card-head"><span>{t.edition}</span><b>RUST</b></div>
          <div className="version-line"><strong>{t.version}</strong></div>
          <div className="download-divider" />
          <ul>{t.included.map((item) => <li key={item}><span>✓</span>{item}</li>)}</ul>
          <a className="button button-download" href="/downloads/AgentChatCleaner-Windows-x64.exe" download>{t.download}<span>↓</span></a>
          <p className="download-note"><i />{t.downloadNote}</p>
          <a className="download-checksum" href="/downloads/AgentChatCleaner-Windows-x64.exe.sha256.txt">{t.checksum}</a>
        </div>
      </section>

      <section className="section faq-section" id="faq">
        <div className="section-heading faq-heading">
          <div><p className="section-eyebrow">{t.faqEyebrow}</p><h2>{t.faqTitle}</h2></div>
          <div className="faq-mark" aria-hidden="true">?</div>
        </div>
        <div className="faq-list">
          {t.faqs.map(([question, answer], index) => (
            <details key={question} open={index === 0}>
              <summary><span>0{index + 1}</span><strong>{question}</strong><i>+</i></summary>
              <p>{answer}</p>
            </details>
          ))}
        </div>
      </section>

      <section className="final-cta">
        <div className="final-pattern" />
        <div><h2>{t.finalTitle.split("\n").map((line) => <span key={line}>{line}</span>)}</h2><p>{t.finalBody}</p></div>
        <a className="button final-button" href="/downloads/AgentChatCleaner-Windows-x64.exe" download>{t.primaryCta}<span>↓</span></a>
      </section>

      <footer>
        <div className="footer-brand"><a className="brand" href="#top"><span className="brand-mark" aria-hidden="true"><i /><i /><i /></span><span>Agent Chat Cleaner</span></a><p>{t.footerNote}</p></div>
        <div className="footer-links">{t.footerLinks.map((item) => <a href="#faq" key={item}>{item}</a>)}</div>
        <p className="copyright">© 2026 Agent Chat Cleaner</p>
      </footer>

    </main>
  );
}
