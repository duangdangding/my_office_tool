import { basename } from "node:path";
import { readFileSync, writeFileSync } from "node:fs";

const [command, ...args] = process.argv.slice(2);

function optionsAndPaths(values) {
  const options = {};
  const paths = [];
  for (let index = 0; index < values.length; index += 1) {
    const value = values[index];
    if (!value.startsWith("--")) {
      paths.push(value);
      continue;
    }
    const key = value.slice(2);
    const next = values[index + 1];
    if (!next || next.startsWith("--")) throw new Error(`参数 ${value} 缺少值`);
    options[key] = next;
    index += 1;
  }
  return { options, paths };
}

function required(options, name) {
  const value = options[name];
  if (!value) throw new Error(`缺少参数 --${name}`);
  return value;
}

function validateTag(tag) {
  if (!/^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(tag)) {
    throw new Error(`标签格式无效：${tag}`);
  }
}

function createFragment(options) {
  const tag = required(options, "tag");
  const platform = required(options, "platform");
  const repository = required(options, "repository");
  const asset = required(options, "asset");
  const output = required(options, "output");
  validateTag(tag);
  if (!["windows-x86_64", "darwin-aarch64", "darwin-x86_64"].includes(platform)) {
    throw new Error(`不支持的更新平台：${platform}`);
  }
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository)) throw new Error(`仓库地址无效：${repository}`);
  const fileName = basename(asset);
  if (!/^[\x20-\x7E]+$/.test(fileName)) throw new Error(`更新资产名称必须使用 ASCII：${fileName}`);
  const signature = readFileSync(`${asset}.sig`, "utf8").trim();
  if (!signature) throw new Error(`更新签名为空：${asset}.sig`);
  const fragment = {
    version: tag.slice(1),
    notes: `https://github.com/${repository}/releases/tag/${tag}`,
    pub_date: new Date().toISOString(),
    platforms: {
      [platform]: {
        signature,
        url: `https://github.com/${repository}/releases/download/${tag}/${encodeURIComponent(fileName)}`,
      },
    },
  };
  writeFileSync(output, `${JSON.stringify(fragment, null, 2)}\n`, "utf8");
  console.log(`已生成 ${platform} 更新清单：${fileName}`);
}

function mergeFragments(options, paths) {
  const output = required(options, "output");
  if (paths.length !== 3) throw new Error("必须提供 Windows 和两种 macOS 架构的 3 个更新清单片段");
  const fragments = paths.map((path) => JSON.parse(readFileSync(path, "utf8")));
  const versions = new Set(fragments.map((fragment) => fragment.version));
  if (versions.size !== 1) throw new Error("各平台更新清单的版本不一致");
  const platforms = Object.assign({}, ...fragments.map((fragment) => fragment.platforms));
  const expected = ["windows-x86_64", "darwin-aarch64", "darwin-x86_64"];
  if (expected.some((platform) => !platforms[platform]) || Object.keys(platforms).length !== expected.length) {
    throw new Error("更新清单必须且只能包含 Windows x64、macOS ARM64、macOS x64 三个平台");
  }
  const latest = {
    version: fragments[0].version,
    notes: fragments[0].notes,
    pub_date: new Date().toISOString(),
    platforms,
  };
  writeFileSync(output, `${JSON.stringify(latest, null, 2)}\n`, "utf8");
  console.log(`已合并 ${Object.keys(platforms).length} 个平台的 latest.json，版本 ${latest.version}`);
}

const { options, paths } = optionsAndPaths(args);
if (command === "fragment") createFragment(options);
else if (command === "merge") mergeFragments(options, paths);
else throw new Error("用法：update-manifest.mjs fragment|merge [参数]");
