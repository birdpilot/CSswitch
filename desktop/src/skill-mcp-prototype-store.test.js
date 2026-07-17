import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import {
  SkillMcpPrototypeStore,
  getMcpFormFields,
  mcpPermission,
  skillPermission,
} from "./skill-mcp-prototype-store.js";

test("Skill 来源权限只允许外部项目被管理", () => {
  const store = new SkillMcpPrototypeStore("healthy");
  const official = store.skills.find((item) => item.source === "official");
  const system = store.skills.find((item) => item.source === "system");
  const external = store.skills.find((item) => item.source === "external");
  assert.equal(skillPermission(official, "uninstall"), false);
  assert.equal(skillPermission(system, "detach"), false);
  assert.equal(skillPermission(external, "uninstall"), true);
  assert.throws(() => store.uninstallSkill(system.id), /不允许卸载/);
});

test("Skill 安装把文件提交、attach 和 load 验证分开", () => {
  const store = new SkillMcpPrototypeStore("healthy");
  store.beginSkillInstall("healthy");
  assert.equal(store.advanceSkillInstall().step, 2);
  assert.equal(store.advanceSkillInstall().status, "reading");
  assert.equal(store.advanceSkillInstall().status, "directory_committed");
  assert.equal(store.advanceSkillInstall().step, 5);
  const result = store.advanceSkillInstall();
  assert.equal(result.status, "attached_load_unverified");
  const skill = store.findSkill(result.resultId);
  assert.equal(skill.attached, true);
  assert.equal(skill.status, "attached_load_unverified");
  assert.equal(store.verifySkillLoad(skill.id).status, "verified");
});

test("attach 失败保留已提交状态并支持重试与安全卸载", () => {
  const store = new SkillMcpPrototypeStore("healthy");
  store.beginSkillInstall("attach-failed");
  for (let step = 0; step < 5; step += 1) store.advanceSkillInstall();
  const result = store.install;
  const skill = store.findSkill(result.resultId);
  assert.equal(skill.status, "committed_attach_failed");
  assert.equal(skill.attached, false);
  assert.equal(store.retrySkillAttach(skill.id).status, "attached_load_unverified");
  assert.equal(store.uninstallSkill(skill.id), true);
  assert.equal(store.findSkill(skill.id), null);
});

test("读取失败和校验失败不会产生外部 Skill 记录", () => {
  for (const scenario of ["read-failed", "validation-failed"]) {
    const store = new SkillMcpPrototypeStore("empty");
    store.beginSkillInstall(scenario);
    store.advanceSkillInstall();
    store.advanceSkillInstall();
    const result = store.advanceSkillInstall();
    assert.equal(result.status, scenario === "read-failed" ? "read_failed" : "validation_failed");
    assert.equal(store.skills.length, 0);
  }
});

test("fixture 可表达 attach 失败、重启要求和空状态", () => {
  const attachFailed = new SkillMcpPrototypeStore("attach-failed");
  assert.ok(attachFailed.skills.some((item) => item.status === "committed_attach_failed"));
  const restart = new SkillMcpPrototypeStore("restart-required");
  assert.ok(restart.skills.some((item) => item.status === "restart_required"));
  assert.ok(restart.mcps.some((item) => item.status === "restart_required"));
  const empty = new SkillMcpPrototypeStore("empty");
  assert.deepEqual(empty.list("skill"), []);
  assert.deepEqual(empty.list("mcp"), []);
});

test("Skill 状态筛选按用户阶段聚合，卡片仍保留精确状态", () => {
  const healthy = new SkillMcpPrototypeStore("healthy");
  assert.deepEqual(healthy.list("skill", { status: "available" }).map((item) => item.status), ["verified", "verified", "verified"]);
  assert.deepEqual(healthy.list("skill", { status: "pending" }).map((item) => item.status), ["attached_load_unverified"]);

  const failed = new SkillMcpPrototypeStore("attach-failed");
  assert.deepEqual(failed.list("skill", { status: "failed" }).map((item) => item.status), ["committed_attach_failed"]);

  const restart = new SkillMcpPrototypeStore("restart-required");
  assert.deepEqual(restart.list("skill", { status: "restart" }).map((item) => item.status), ["restart_required"]);
});

test("MCP transport 推导 stdio 与 http/sse 的不同字段", () => {
  assert.deepEqual(getMcpFormFields("stdio"), ["name", "command", "args"]);
  assert.deepEqual(getMcpFormFields("http"), ["name", "url", "headers"]);
  assert.deepEqual(getMcpFormFields("sse"), ["name", "url", "headers"]);
});

test("外部 MCP 支持草稿、启停和 attach/detach，系统 MCP 保持只读", () => {
  const store = new SkillMcpPrototypeStore("healthy");
  const system = store.mcps.find((item) => item.source === "system");
  assert.equal(mcpPermission(system, "edit"), false);
  assert.throws(() => store.toggleMcp(system.id), /不允许启停/);

  const created = store.saveMcp({
    name: "local-tools",
    transport: "stdio",
    command: "node",
    args: "server.js --stdio",
    enabled: true,
  });
  assert.equal(created.status, "enabled_unattached");
  assert.equal(store.attachMcp(created.id).status, "attached");
  assert.equal(store.detachMcp(created.id).status, "enabled_unattached");
  assert.equal(store.toggleMcp(created.id).status, "disconnected");
});

test("错误的 MCP 配置如实标为配置错误，Header 只留在内存对象", () => {
  const store = new SkillMcpPrototypeStore("empty");
  const invalid = store.saveMcp({ name: "bad-http", transport: "http", url: "not-a-url", enabled: true });
  assert.equal(invalid.status, "config_error");
  const valid = store.saveMcp({
    name: "remote-http",
    transport: "http",
    url: "https://mcp.example.invalid/api",
    headers: [{ key: "Authorization", value: "Bearer memory-only" }],
    enabled: true,
  });
  assert.equal(valid.status, "enabled_unattached");
  assert.equal(store.findMcp(valid.id).headers[0].value, "Bearer memory-only");
});

test("原型模块不包含 Tauri 调用或持久化写入，导航默认隐藏", async () => {
  const [uiSource, storeSource, htmlSource, mainSource] = await Promise.all([
    readFile(new URL("./skill-mcp-prototype.js", import.meta.url), "utf8"),
    readFile(new URL("./skill-mcp-prototype-store.js", import.meta.url), "utf8"),
    readFile(new URL("./index.html", import.meta.url), "utf8"),
    readFile(new URL("./main.js", import.meta.url), "utf8"),
  ]);
  assert.doesNotMatch(uiSource + storeSource, /__TAURI__|\binvoke\s*\(|localStorage\.setItem/);
  assert.match(htmlSource, /class="nav-item prototype-nav"[^>]*hidden/);
  assert.match(htmlSource, /data-page-target="switch">\s*<span>模型连接<\/span>/);
  assert.doesNotMatch(htmlSource, /data-page-target="profiles"/);
  assert.match(htmlSource, /<h2>配置方案<\/h2>/);
  assert.match(htmlSource, /id="oneClickBtn">一键开始<\/button>[\s\S]*id="openBrowserBtn">浏览器打开<\/button>[\s\S]*id="stopBtn">全部停止<\/button>/);
  assert.match(mainSource, /class="profile-list-head"/);
  assert.match(mainSource, /class="profile-model-select"/);
  assert.match(uiSource, /<span>名称<\/span><span>来源<\/span><span>类型<\/span><span>Agent<\/span><span>状态<\/span><span>操作<\/span>/);
  assert.doesNotMatch(uiSource, /sourceBadge\(/);
});

test("MCP 管理页在后端就绪前保持禁用", async () => {
  const source = await readFile(new URL("./skill-mcp-prototype.js", import.meta.url), "utf8");
  assert.match(source, /const MCP_UI_ENABLED = false/);
  assert.match(source, /disabled>MCP 暂未开放<\/button>/);
});
