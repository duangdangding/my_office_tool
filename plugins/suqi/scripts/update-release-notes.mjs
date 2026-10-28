// 更新指定 tag 的 Release notes：GET /releases/tags/<tag> 拿 id → PATCH body → 重新 GET 核对
const [tag, notesPath] = process.argv.slice(2);
const repo = "duangdangding/ls_quick_start";
const headers = { Authorization: `Bearer ${process.env.GH_TOKEN}`, Accept: "application/vnd.github+json", "Content-Type": "application/json", "User-Agent": "suqi-release" };
import fs from "fs";

const rel = await (await fetch(`https://api.github.com/repos/${repo}/releases/tags/${tag}`, { headers })).json();
if (!rel.id) { console.error("release not found:", JSON.stringify(rel)); process.exit(1); }
const body = fs.readFileSync(notesPath, "utf-8");
const patch = await (await fetch(`https://api.github.com/repos/${repo}/releases/${rel.id}`, { method: "PATCH", headers, body: JSON.stringify({ body }) })).json();
if (!patch.id) { console.error("patch failed:", JSON.stringify(patch)); process.exit(1); }
// 核对：body 首尾完整、assets 数量
const check = await (await fetch(`https://api.github.com/repos/${repo}/releases/tags/${tag}`, { headers })).json();
console.log("tag:", check.tag_name);
console.log("assets:", check.assets.length, check.assets.map((a) => a.name).join(", "));
console.log("body head:", check.body.slice(0, 50).replace(/\n/g, "\\n"));
console.log("body tail:", check.body.slice(-50).replace(/\n/g, "\\n"));
