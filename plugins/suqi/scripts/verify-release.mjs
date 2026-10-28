const tag = process.argv[2] || "v0.16.4";
const headers = { Authorization: `Bearer ${process.env.GH_TOKEN}`, Accept: "application/vnd.github+json", "User-Agent": "suqi-release" };
const rel = await (await fetch(`https://api.github.com/repos/duangdangding/ls_quick_start/releases/tags/${tag}`, { headers })).json();
console.log("name:", rel.name, "| tag:", rel.tag_name, "| assets:", rel.assets.length);
rel.assets.forEach(a => console.log(" -", a.name));
console.log("body head:", JSON.stringify(rel.body.slice(0, 50)));
console.log("body tail:", JSON.stringify(rel.body.slice(-50)));
