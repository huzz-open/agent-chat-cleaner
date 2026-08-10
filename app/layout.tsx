import type { Metadata } from "next";
import "./globals.css";

const publicSiteUrl = process.env.NEXT_PUBLIC_SITE_URL ?? "https://acc.huzz.top";

export const metadata: Metadata = {
  metadataBase: new URL(publicSiteUrl),
  title: "Agent Chat Cleaner — AI Agent 会话管理工具",
  description: "统一扫描、筛选、删除并恢复 Cursor、ChatGPT / Codex 与 Claude Code 会话。纯 Rust Windows 桌面应用。",
  applicationName: "Agent Chat Cleaner",
  keywords: ["Cursor", "ChatGPT", "Codex", "Claude Code", "AI agent", "session cleaner", "Rust", "会话管理"],
  openGraph: {
    title: "Agent Chat Cleaner — 看清每个会话",
    description: "面向 Cursor、ChatGPT / Codex 和 Claude Code 的原生会话管理工具。",
    type: "website",
    siteName: "Agent Chat Cleaner",
    images: [{ url: "/og.png", width: 1732, height: 908, alt: "Agent Chat Cleaner — 看清每个会话" }],
  },
  twitter: {
    card: "summary_large_image",
    title: "Agent Chat Cleaner",
    description: "A native session manager for Cursor, ChatGPT / Codex, and Claude Code.",
    images: ["/og.png"],
  },
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="zh-CN">
      <body>{children}</body>
    </html>
  );
}
