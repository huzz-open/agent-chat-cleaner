import type { Metadata } from "next";
import "./globals.css";

const publicSiteUrl = process.env.NEXT_PUBLIC_SITE_URL ?? "https://agent-chat-cleaner.huzzcj.chatgpt.site";

export const metadata: Metadata = {
  metadataBase: new URL(publicSiteUrl),
  title: "Agent Chat Cleaner — 本地 AI 会话整理工具",
  description: "统一扫描、预览、隔离并恢复 Codex、Claude Code 等本地 AI Agent 会话。原生 Rust，聊天内容不上云。",
  applicationName: "Agent Chat Cleaner",
  keywords: ["Codex", "Claude Code", "AI agent", "session cleaner", "Rust", "本地会话", "清理工具"],
  openGraph: {
    title: "Agent Chat Cleaner — 看清楚，再清干净",
    description: "本地优先、可恢复的 AI 编程 Agent 会话整理工具。",
    type: "website",
    siteName: "Agent Chat Cleaner",
    images: [{ url: "/og.png", width: 1732, height: 908, alt: "Agent Chat Cleaner — 看清楚，再清干净" }],
  },
  twitter: {
    card: "summary_large_image",
    title: "Agent Chat Cleaner",
    description: "See clearly. Clean safely. Local AI agent session cleanup.",
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
