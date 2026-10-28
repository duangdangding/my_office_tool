import { createServer } from "vite";

const server = await createServer({
  server: { host: "127.0.0.1", port: 7179, strictPort: true },
  logLevel: "error",
});

try {
  await server.listen();
  const [page, script, suqi, lscopy] = await Promise.all([
    fetch("http://127.0.0.1:7179/"),
    fetch("http://127.0.0.1:7179/src/main.ts"),
    fetch("http://127.0.0.1:7179/plugins/suqi/office-plugin.json"),
    fetch("http://127.0.0.1:7179/plugins/lscopy/office-plugin.json"),
  ]);
  if (!page.ok || !script.ok || !suqi.ok || !lscopy.ok) throw new Error("网页或插件清单无法访问");
  const html = await page.text();
  if (!html.includes("办公工具箱") || !html.includes("/src/main.ts")) throw new Error("主页面内容不完整");
  const manifests = await Promise.all([suqi.json(), lscopy.json()]);
  if (manifests.map((item) => item.id).join(",") !== "suqi,lscopy") throw new Error("内置插件清单不完整");
  console.log("网页预览与两个内置插件清单访问正常。");
} finally {
  await server.close();
}
