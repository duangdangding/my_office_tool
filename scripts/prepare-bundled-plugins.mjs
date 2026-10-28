import { cpSync, existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = join(root, "plugins");
const target = join(root, ".generated", "plugins");
// CI 可将预编译程序按插件内的相对路径暂存到这里；每次重新生成源码资源时自动合并。
const prebuilt = join(root, ".generated", "plugin-binaries");
const ignoredDirectories = new Set([".git", "node_modules", "target", "dist", "build", ".next", "__pycache__", ".generated"]);

// 保留发布根目录，避免 Windows 删除后立即重建时出现短暂 EPERM。
mkdirSync(target, { recursive: true });
for (const entry of readdirSync(target)) {
  rmSync(join(target, entry), { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
}

for (const entry of readdirSync(source, { withFileTypes: true })) {
  if (!entry.isDirectory() || !existsSync(join(source, entry.name, "office-plugin.json"))) continue;
  cpSync(join(source, entry.name), join(target, entry.name), {
    recursive: true,
    filter(path) {
      const name = path.split(/[\\/]/).pop() ?? "";
      if (ignoredDirectories.has(name) || name.endsWith(".log") || name.endsWith(".db") || name === "data") return false;
      return true;
    },
  });
  const prebuiltPlugin = join(prebuilt, entry.name);
  if (existsSync(prebuiltPlugin)) {
    cpSync(prebuiltPlugin, join(target, entry.name), { recursive: true });
    console.log(`已合并预编译插件：${entry.name}`);
  }
  console.log(`已整理发布插件：${entry.name}`);
}
