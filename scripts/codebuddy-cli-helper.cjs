#!/usr/bin/env node
// CodeBuddy CLI apiKeyHelper for wb-switch.
//
// 只把当前选中账号的 token（Bearer xxx）打印到 stdout，CodeBuddy CLI 会把它
// 作为 Authorization 头使用。当前选中账号由 wb-switch 写入
// ~/.codebuddy-rotate/state.json（activeAccountId / active 索引）。
//
// 注意首行的 shebang：**发布版**写的是 `#!/usr/bin/env node`，但 wb-switch 安装
// helper 时会把它替换成 Node.js 的绝对路径（如 `#!/home/me/.nvm/.../bin/node`）。
// 原因：CodeBuddy CLI 用**自己的子 shell** 执行 helper，那个 shell 的 PATH 不一定
// 包含 nvm；依赖 PATH 的旧版本 helper 会以退出码 127（env: 'node': No such file
// or directory）失败，CLI 随即静默回落到自身登录态，账号切换看起来成功却不生效。
// 改动本文件时请保留首行形态，安装逻辑依赖它做替换。
"use strict";

const fs = require("fs");
const os = require("os");
const path = require("path");

const rotateDir = process.env.CODEBUDDY_ROTATE_DIR || path.join(os.homedir(), ".codebuddy-rotate");
const accountsFile =
  process.env.WB_SWITCH_ACCOUNTS_FILE || path.join(os.homedir(), ".wb-switch", "accounts.json");
const stateFile = path.join(rotateDir, "state.json");

function fail(message) {
  process.stderr.write(`wb-switch: ${message}\n`);
  process.exit(1);
}

function readJson(file, fallback) {
  try {
    const text = fs.readFileSync(file, "utf8").replace(/^\uFEFF/, "");
    return JSON.parse(text);
  } catch {
    return fallback;
  }
}

const accounts = readJson(accountsFile, null);
if (!Array.isArray(accounts) || accounts.length === 0) {
  fail("no accounts available");
}

let state = readJson(stateFile, {});
if (typeof state !== "object" || state === null || Array.isArray(state)) {
  state = {};
}

function resolveIndex(state, accounts) {
  let index = 0;
  const activeId = typeof state.activeAccountId === "string" ? state.activeAccountId : "";
  if (activeId) {
    const found = accounts.findIndex((account) => account && account.id === activeId);
    if (found !== -1) return found;
  }
  const legacy = Number.parseInt(state.active, 10);
  if (Number.isFinite(legacy)) index = legacy;
  return ((index % accounts.length) + accounts.length) % accounts.length;
}

const index = resolveIndex(state, accounts);
const account = accounts[index];
const token = account && typeof account.access_token === "string" ? account.access_token.trim() : "";
if (!token) {
  fail("selected account has no access token");
}
process.stdout.write(`Bearer ${token}\n`);
