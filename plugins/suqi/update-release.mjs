// 更新 GitHub Release notes：GET release id → PATCH body → GET 核对
// 用法: node update-release.mjs <tag> <notesPath>
import { execSync } from "node:child_process";
import { readFileSync } from "node:fs";

const [tag, notesPath] = process.argv.slice(2);
const repo = "duangdangding/ls_quick_start";

// 从 Windows 凭据管理器取 token（不明文打印）
const out = execSync("git credential fill", { input: "protocol=https\nhost=github.com\n\n", encoding: "utf-8" });
const token = out.split(/\r?\n/).find((l) => l.startsWith("password="))?.slice("password=".length).trim();
if (!token) { console.error("未能获取 GitHub token"); process.exit(1); }

const headers = { Authorization: `Bearer ${token}`, Accept: "application/vnd.github+json", "Content-Type": "application/json" };
const body = readFileSync(notesPath, "utf-8");

const rel = await (await fetch(`https://api.github.com/repos/${repo}/releases/tags/${tag}`, { headers })).json();
if (!rel.id) { console.error("未找到 release:", rel); process.exit(1); }

const patchResp = await fetch(`https://api.github.com/repos/${repo}/releases/${rel.id}`, { method: "PATCH", headers, body: JSON.stringify({ body }) });
if (!patchResp.ok) { console.error("PATCH 失败:", patchResp.status, await patchResp.text()); process.exit(1); }

// 核对：body 首尾完整、assets 数量
const check = await (await fetch(`https://api.github.com/repos/${repo}/releases/${rel.id}`, { headers })).json();
const firstLine = check.body.split("\n")[0];
const lastLine = check.body.trimEnd().split("\n").at(-1);
console.log(`body 长度: ${check.body.length}（写入 ${body.length}）`);
console.log(`首行: ${firstLine}`);
console.log(`尾行: ${lastLine}`);
console.log(`assets 数量: ${check.assets.length}`);
check.assets.forEach((a) => console.log(`  - ${a.name}`));
