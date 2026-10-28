// 从 src/main.ts 提取真实的 showItemMenu，用桩 DOM 验证四种边缘场景下菜单始终在视口内
import { readFileSync } from "node:fs";

const src = readFileSync(new URL("../src/main.ts", import.meta.url), "utf-8");
const start = src.indexOf("function showItemMenu");
if (start < 0) throw new Error("showItemMenu not found");
const line = src.slice(start, src.indexOf("\n", start))
  .replace("function showItemMenu(id: string, anchor: HTMLElement)", "function showItemMenu(id, anchor)")
  .replace(/"\)!/g, "\")"); // 去掉 TS 类型标注与非空断言

const MENU_W = 116, MENU_H = 153; // .menu 宽度由 CSS 固定为 116px；高度按 4 个按钮实测约 153px

function makeEnv(vw, vh, box) {
  const fakeMenu = {
    className: "", innerHTML: "", style: {},
    offsetWidth: MENU_W, offsetHeight: MENU_H,
    remove() {}, addEventListener() {},
    querySelector: () => ({ addEventListener() {} }),
  };
  const document = {
    querySelector: () => ({ remove() {}, addEventListener() {} }),
    createElement: () => fakeMenu,
    body: { append() {} },
  };
  const window = { innerWidth: vw, innerHeight: vh, addEventListener() {} };
  const anchor = { getBoundingClientRect: () => box };
  return { document, window, anchor, fakeMenu };
}

const fn = new Function(
  "document", "window", "items",
  `${line}; return showItemMenu;`
);

const scenarios = [
  ["列表-第一行（顶部+左侧按钮）", 520, 640, { left: 28, right: 60, top: 40, bottom: 70 }],
  ["列表-最后一行（底部）", 520, 640, { left: 28, right: 60, top: 596, bottom: 626 }],
  ["卡片-右下角（右侧+底部）", 520, 640, { left: 430, right: 502, top: 596, bottom: 626 }],
  ["卡片-右上角（右侧）", 520, 640, { left: 430, right: 502, top: 40, bottom: 70 }],
  ["极窄窗口（最小 600x420，按钮贴右）", 600, 420, { left: 540, right: 592, top: 380, bottom: 410 }],
];

let fail = 0;
for (const [name, vw, vh, box] of scenarios) {
  const env = makeEnv(vw, vh, box);
  const showItemMenu = fn(env.document, env.window, []);
  showItemMenu("x", env.anchor);
  const left = parseFloat(env.fakeMenu.style.left), top = parseFloat(env.fakeMenu.style.top);
  const right = left + MENU_W, bottom = top + MENU_H;
  const ok = left >= 6 && top >= 6 && right <= vw - 6 && bottom <= vh - 6;
  if (!ok) fail++;
  console.log(`${ok ? "PASS" : "FAIL"} ${name}: left=${left} top=${top} right=${right} bottom=${bottom} (视口 ${vw}x${vh})`);
}
process.exit(fail ? 1 : 0);
