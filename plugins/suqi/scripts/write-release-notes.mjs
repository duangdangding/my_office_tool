// 用法：GH_TOKEN=... node scripts/write-release-notes.mjs <tag> <notesPath>
// 把 notes 文件内容写入对应 tag 的 GitHub Release body，并回读校验
import { readFileSync } from "node:fs";

const [tag, notesPath] = process.argv.slice(2);
const repo = "duangdangding/ls_quick_start";
const headers = {
  Authorization: `Bearer ${process.env.GH_TOKEN}`,
  Accept: "application/vnd.github+json",
  "Content-Type": "application/json",
};

const rel = await (await fetch(`https://api.github.com/repos/${repo}/releases/tags/${tag}`, { headers })).json();
if (!rel.id) throw new Error(`未找到 ${tag} 的 Release：${JSON.stringify(rel)}`);

const body = readFileSync(notesPath, "utf-8");
const patch = await fetch(`https://api.github.com/repos/${repo}/releases/${rel.id}`, {
  method: "PATCH",
  headers,
  body: JSON.stringify({ body }),
});
if (!patch.ok) throw new Error(`PATCH 失败：${patch.status} ${await patch.text()}`);

// 回读核对：body 首尾完整、assets 数量
const check = await (await fetch(`https://api.github.com/repos/${repo}/releases/${rel.id}`, { headers })).json();
const trimmed = check.body.trim();
console.log("body 长度:", trimmed.length);
console.log("body 首行:", trimmed.split("\n")[0]);
console.log("body 尾行:", trimmed.split("\n").at(-1));
console.log("assets 数量:", check.assets.length);
console.log("assets:", check.assets.map((a) => a.name).join(", "));
