import { cpSync, existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = join(root, "plugins");
const target = join(root, ".generated", "plugins");
const ignoredDirectories = new Set([".git", "node_modules", "target", "dist", "build", ".next", "__pycache__", ".generated"]);

rmSync(target, { recursive: true, force: true });
mkdirSync(target, { recursive: true });

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
  console.log(`已整理发布插件：${entry.name}`);
}
