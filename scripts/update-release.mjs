// 在用户确认 GitHub Actions 构建成功后，更新指定 Release 的说明并回读核对。
// 用法：GITHUB_REPOSITORY=owner/repo GH_TOKEN=... node scripts/update-release.mjs v0.1.0 release-notes-0.1.0.md
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

const [tag, notesPath] = process.argv.slice(2);
const repo = process.env.GITHUB_REPOSITORY;
if (!repo || !/^[\w.-]+\/[\w.-]+$/.test(repo)) {
  throw new Error("请通过 GITHUB_REPOSITORY 指定仓库，例如 owner/my_office_tool");
}
if (!tag || !/^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(tag) || !notesPath) {
  throw new Error("用法：node scripts/update-release.mjs <vX.Y.Z> <release-notes-X.Y.Z.md>");
}

let token = process.env.GH_TOKEN?.trim();
if (!token) {
  const output = execFileSync("git", ["credential", "fill"], {
    input: "protocol=https\nhost=github.com\n\n",
    encoding: "utf8",
  });
  token = output.split(/\r?\n/).find((line) => line.startsWith("password="))?.slice(9).trim();
}
if (!token) throw new Error("未能从 GH_TOKEN 或 Git 凭据管理器获取 GitHub token");

const headers = {
  Authorization: `Bearer ${token}`,
  Accept: "application/vnd.github+json",
  "Content-Type": "application/json",
  "User-Agent": "office-toolbox-release",
};
const request = async (url, options = {}) => {
  const response = await fetch(url, { ...options, headers });
  const data = await response.json();
  if (!response.ok) throw new Error(`GitHub API ${response.status}：${data.message ?? "请求失败"}`);
  return data;
};

const body = readFileSync(notesPath, "utf8");
const release = await request(`https://api.github.com/repos/${repo}/releases/tags/${tag}`);
await request(`https://api.github.com/repos/${repo}/releases/${release.id}`, {
  method: "PATCH",
  body: JSON.stringify({ body }),
});
const check = await request(`https://api.github.com/repos/${repo}/releases/${release.id}`);
if (check.body !== body) throw new Error("Release 说明回读结果与本地文件不一致");

const assets = check.assets.map((asset) => asset.name);
console.log(`已更新 ${check.html_url}`);
console.log(`说明首行：${check.body.split("\n")[0]}`);
console.log(`说明尾行：${check.body.trimEnd().split("\n").at(-1)}`);
console.log(`资产数量：${assets.length}`);
assets.forEach((name) => console.log(`- ${name}`));
