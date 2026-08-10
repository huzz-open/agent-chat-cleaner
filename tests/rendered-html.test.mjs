import assert from "node:assert/strict";
import { access, readFile } from "node:fs/promises";
import test from "node:test";

async function render() {
  const workerUrl = new URL("../dist/server/index.js", import.meta.url);
  workerUrl.searchParams.set("test", `${process.pid}-${Date.now()}`);
  const { default: worker } = await import(workerUrl.href);

  return worker.fetch(
    new Request("http://localhost/", { headers: { accept: "text/html" } }),
    { ASSETS: { fetch: async () => new Response("Not found", { status: 404 }) } },
    { waitUntil() {}, passThroughOnException() {} },
  );
}

test("server-renders the Agent Chat Cleaner landing page", async () => {
  const response = await render();
  assert.equal(response.status, 200);
  assert.match(response.headers.get("content-type") ?? "", /^text\/html\b/i);

  const html = await response.text();
  assert.match(html, /<html lang="zh-CN">/);
  assert.match(html, /<title>Agent Chat Cleaner — AI Agent 会话管理工具<\/title>/);
  assert.match(html, /把散落的 Agent 会话/);
  assert.match(html, /Cursor、ChatGPT \/ Codex 与 Claude Code/);
  assert.match(html, /下载 Windows 版/);
  assert.match(html, /共享大文件按会话处理/);
  assert.match(html, /v0\.7\.9/);
  assert.match(html, /https:\/\/github\.com\/huzz-open\/agent-chat-cleaner/);
  assert.doesNotMatch(html, /¥9\.9|支付宝|微信支付|一次付费/);
  assert.doesNotMatch(html, /本地模式|输入确认词|移入隔离区/);
  assert.match(html, /og\.png/);
  assert.doesNotMatch(html, /codex-preview|Your site is taking shape|react-loading-skeleton/);
});

test("ships bilingual, open-source product content without starter artifacts", async () => {
  const [page, layout, css, packageJson, plan] = await Promise.all([
    readFile(new URL("../app/page.tsx", import.meta.url), "utf8"),
    readFile(new URL("../app/layout.tsx", import.meta.url), "utf8"),
    readFile(new URL("../app/globals.css", import.meta.url), "utf8"),
    readFile(new URL("../package.json", import.meta.url), "utf8"),
    readFile(new URL("../agent_chat_cleaner_0f9a8f56.plan.md", import.meta.url), "utf8"),
  ]);

  assert.match(page, /zh:\s*\{/);
  assert.match(page, /en:\s*\{/);
  assert.match(page, /AgentChatCleaner-Windows-x64\.exe/);
  assert.match(page, /https:\/\/github\.com\/huzz-open\/agent-chat-cleaner/);
  assert.match(page, /Cursor, ChatGPT \/ Codex, and Claude Code/);
  assert.match(page, /批量进度/);
  assert.doesNotMatch(page, /Move to quarantine|移入隔离区|本地模式/);
  assert.match(layout, /summary_large_image/);
  assert.match(css, /prefers-reduced-motion:\s*reduce/);
  assert.doesNotMatch(packageJson, /react-loading-skeleton/);
  assert.match(plan, /默认只读扫描/);
  assert.match(plan, /共享大文件只能记录级改写或事务级删除/);

  await access(new URL("../public/og.png", import.meta.url));
  await access(new URL("../public/downloads/AgentChatCleaner-Windows-x64.exe", import.meta.url));
  await assert.rejects(access(new URL("../app/_sites-preview", import.meta.url)));
});
