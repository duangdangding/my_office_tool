const [tag, notesPath] = process.argv.slice(2);
const repo = "duangdangding/ls_quick_start";
const headers = { Authorization: `Bearer ${process.env.GH_TOKEN}`, Accept: "application/vnd.github+json", "Content-Type": "application/json" };

async function main() {
  const rel = await (await fetch(`https://api.github.com/repos/${repo}/releases/tags/${tag}`, { headers })).json();
  const body = require("fs").readFileSync(notesPath, "utf-8");
  const res = await fetch(`https://api.github.com/repos/${repo}/releases/${rel.id}`, { method: "PATCH", headers, body: JSON.stringify({ body }) });
  console.log("status:", res.status, "release id:", rel.id);
}

main().catch((e) => { console.error(e); process.exit(1); });
