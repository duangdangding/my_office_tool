// 把 design/icon.svg 转成 Tauri 需要的多尺寸 PNG，再合成 icon.ico 与 icon.icns
// 运行：node design/make-icon.mjs
import sharp from "sharp";
import pngToIco from "png-to-ico";
import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const svgPath = join(root, "design", "icon.svg");
const iconsDir = join(root, "src-tauri", "icons");
mkdirSync(iconsDir, { recursive: true });

const png = (size) => sharp(svgPath, { density: 384 }).resize(size, size).png().toBuffer();

const sizes = [256, 128, 64, 48, 32, 24, 16];
const pngBuffers = await Promise.all(sizes.map(png));
// 最高分辨率 png 作为 icon.png
writeFileSync(join(iconsDir, "icon.png"), pngBuffers[0]);
writeFileSync(join(iconsDir, "128x128.png"), pngBuffers[1]);
writeFileSync(join(iconsDir, "128x128@2x.png"), pngBuffers[0]);
writeFileSync(join(iconsDir, "32x32.png"), pngBuffers[4]);

const ico = await pngToIco(pngBuffers);
writeFileSync(join(iconsDir, "icon.ico"), ico);

// macOS 图标容器：icns 头部 + 若干「类型 + 长度 + PNG 数据」块
function makeIcns(entries) {
  const chunks = entries.map(([type, buffer]) => {
    const header = Buffer.alloc(8);
    header.write(type, 0, 4, "ascii");
    header.writeUInt32BE(8 + buffer.length, 4);
    return Buffer.concat([header, buffer]);
  });
  const body = Buffer.concat(chunks);
  const head = Buffer.alloc(8);
  head.write("icns", 0, 4, "ascii");
  head.writeUInt32BE(8 + body.length, 4);
  return Buffer.concat([head, body]);
}

const icnsSizes = [
  ["ic11", 32], ["ic12", 64], ["ic07", 128],
  ["ic13", 256], ["ic14", 512], ["ic09", 512],
  ["ic08", 256], ["ic10", 1024],
];
const icnsEntries = await Promise.all(icnsSizes.map(async ([type, size]) => [type, await png(size)]));
writeFileSync(join(iconsDir, "icon.icns"), makeIcns(icnsEntries));

console.log("icon.png / 128x128.png / 32x32.png / icon.ico / icon.icns 已生成");
