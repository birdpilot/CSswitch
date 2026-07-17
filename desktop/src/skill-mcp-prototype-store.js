export const SKILL_STATUSES = Object.freeze({
  not_started: { label: "尚未开始", tone: "neutral" },
  reading: { label: "下载 / 读取中", tone: "warning" },
  read_failed: { label: "下载 / 读取失败", tone: "danger" },
  validation_failed: { label: "安全校验失败", tone: "danger" },
  directory_committed: { label: "文件已提交，等待 attach", tone: "warning" },
  committed_attach_failed: { label: "文件已提交，attach 失败", tone: "danger" },
  attached_load_unverified: { label: "已 attach，load 未验证", tone: "warning" },
  verified: { label: "已验证可用", tone: "success" },
  restart_required: { label: "已 attach，需要重启", tone: "warning" },
});

// 列表筛选只保留用户需要判断的阶段；卡片仍展示上面的精确状态。
export const SKILL_STATUS_FILTERS = Object.freeze({
  available: { label: "可用", statuses: ["verified"] },
  installing: { label: "安装中", statuses: ["reading"] },
  pending: { label: "待完成", statuses: ["not_started", "directory_committed", "attached_load_unverified"] },
  failed: { label: "失败", statuses: ["read_failed", "validation_failed", "committed_attach_failed"] },
  restart: { label: "需重启", statuses: ["restart_required"] },
});

export const MCP_STATUSES = Object.freeze({
  disconnected: { label: "未连接", tone: "neutral" },
  enabled_unattached: { label: "已启用，未 attach", tone: "warning" },
  attached: { label: "已 attach", tone: "success" },
  restart_required: { label: "需要重启", tone: "warning" },
  config_error: { label: "配置错误", tone: "danger" },
});

export const SOURCE_LABELS = Object.freeze({
  official: "官方",
  system: "CSSwitch 系统",
  external: "外部安装",
});

export const FIXTURE_LABELS = Object.freeze({
  healthy: "健康状态",
  "attach-failed": "attach 失败",
  "restart-required": "需要重启",
  empty: "空列表",
});

const BASE_SKILLS = [
  {
    id: "official-pdf",
    name: "PDF",
    description: "由 Claude Science / Anthropic 分发的 PDF 阅读与处理 Skill。",
    source: "official",
    installMethod: "Science 分发",
    agent: "OPERON",
    attached: true,
    status: "verified",
    purpose: "文档理解",
    readOnly: true,
  },
  {
    id: "system-external-skill-tools",
    name: "csswitch-external-skill-tools",
    description: "为外部 Skill 安装流程提供路由提示与系统边界说明。",
    source: "system",
    installMethod: "CSSwitch 内置",
    agent: "OPERON",
    attached: true,
    status: "verified",
    purpose: "安装路由",
    readOnly: true,
  },
  {
    id: "external-internal-comms",
    name: "internal-comms",
    description: "从 GitHub 安装的团队沟通写作 Skill；用于演示外部来源管理。",
    source: "external",
    installMethod: "GitHub URL",
    sourceValue: "https://github.com/anthropics/skills/tree/main/skills/internal-comms",
    agent: "OPERON",
    attached: true,
    status: "verified",
    purpose: "团队沟通",
    readOnly: false,
  },
  {
    id: "external-local-research",
    name: "research-workflow",
    description: "从本地文件夹读取的研究流程 Skill；当前已 attach，但尚未验证 load。",
    source: "external",
    installMethod: "本地文件夹",
    sourceValue: "/Users/demo/skills/research-workflow",
    agent: "OPERON",
    attached: true,
    status: "attached_load_unverified",
    purpose: "研究流程",
    readOnly: false,
  },
];

const BASE_MCPS = [
  {
    id: "system-skill-installer",
    name: "csswitch-skill-installer",
    description: "CSSwitch 托管的本地 stdio connector，用于外部 Skill 安装动作。",
    source: "system",
    transport: "stdio",
    command: "csswitch-gateway",
    args: ["mcp", "skill-installer"],
    agent: "OPERON",
    enabled: true,
    attached: true,
    status: "attached",
    managedBy: "CSSwitch",
    readOnly: true,
  },
  {
    id: "external-research-notes",
    name: "research-notes",
    description: "用户外部添加的 HTTP MCP 原型配置；密钥仅保留在当前页面内存。",
    source: "external",
    transport: "http",
    url: "https://mcp.example.invalid/api",
    headers: [{ key: "Authorization", value: "Bearer prototype-secret" }],
    agent: "未绑定",
    enabled: true,
    attached: false,
    status: "enabled_unattached",
    managedBy: "用户",
    readOnly: false,
  },
];

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

function fixtureState(fixture) {
  if (fixture === "empty") return { skills: [], mcps: [] };
  const skills = clone(BASE_SKILLS);
  const mcps = clone(BASE_MCPS);
  if (fixture === "attach-failed") {
    const skill = skills.find((item) => item.source === "external");
    skill.status = "committed_attach_failed";
    skill.attached = false;
    skill.agent = "未绑定";
    const mcp = mcps.find((item) => item.source === "external");
    mcp.status = "config_error";
  }
  if (fixture === "restart-required") {
    const skill = skills.find((item) => item.source === "external");
    skill.status = "restart_required";
    const mcp = mcps.find((item) => item.source === "external");
    mcp.status = "restart_required";
    mcp.attached = true;
    mcp.agent = "OPERON";
  }
  return { skills, mcps };
}

export function skillPermission(skill, action) {
  if (!skill) return false;
  if (skill.source !== "external") return action === "view";
  return ["view", "detach", "retry_attach", "verify_load", "restart", "uninstall"].includes(action);
}

export function mcpPermission(mcp, action) {
  if (!mcp) return false;
  if (mcp.source !== "external") return action === "view";
  return ["view", "edit", "toggle", "attach", "detach", "delete"].includes(action);
}

export function getMcpFormFields(transport) {
  if (transport === "stdio") return ["name", "command", "args"];
  if (transport === "http" || transport === "sse") return ["name", "url", "headers"];
  return ["name"];
}

export class SkillMcpPrototypeStore {
  constructor(fixture = "healthy") {
    this.reset(fixture);
  }

  reset(fixture = this.fixture || "healthy") {
    this.fixture = FIXTURE_LABELS[fixture] ? fixture : "healthy";
    const state = fixtureState(this.fixture);
    this.skills = state.skills;
    this.mcps = state.mcps;
    this.install = null;
    this.revision = (this.revision || 0) + 1;
    return this.snapshot();
  }

  snapshot() {
    return clone({
      fixture: this.fixture,
      skills: this.skills,
      mcps: this.mcps,
      install: this.install,
      revision: this.revision,
    });
  }

  list(kind, filters = {}) {
    const collection = kind === "mcp" ? this.mcps : this.skills;
    const query = String(filters.query || "").trim().toLowerCase();
    return collection.filter((item) => {
      if (filters.source && filters.source !== "all" && item.source !== filters.source) return false;
      if (filters.status && filters.status !== "all") {
        const skillGroup = kind === "skill" ? SKILL_STATUS_FILTERS[filters.status] : null;
        if (skillGroup ? !skillGroup.statuses.includes(item.status) : item.status !== filters.status) return false;
      }
      if (!query) return true;
      return [item.name, item.description, item.agent, item.transport, item.purpose]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(query));
    }).map(clone);
  }

  findSkill(id) {
    return this.skills.find((item) => item.id === id) || null;
  }

  findMcp(id) {
    return this.mcps.find((item) => item.id === id) || null;
  }

  beginSkillInstall(defaultScenario = "healthy") {
    this.install = {
      step: 1,
      method: "github",
      sourceValue: "https://github.com/anthropics/skills/tree/main/skills/internal-comms",
      name: "internal-comms-copy",
      scenario: defaultScenario,
      status: "not_started",
      resultId: null,
    };
    return clone(this.install);
  }

  updateSkillInstall(patch) {
    if (!this.install) throw new Error("安装流程尚未开始");
    this.install = { ...this.install, ...clone(patch) };
    return clone(this.install);
  }

  advanceSkillInstall() {
    const install = this.install;
    if (!install) throw new Error("安装流程尚未开始");
    if (install.step === 1) {
      if (!String(install.name || "").trim()) throw new Error("请填写 Skill 名称");
      if (!String(install.sourceValue || "").trim()) throw new Error("请填写来源");
      install.step = 2;
      return clone(install);
    }
    if (install.step === 2) {
      install.step = 3;
      install.status = "reading";
      return clone(install);
    }
    if (install.step === 3) {
      if (install.scenario === "read-failed") {
        install.step = 6;
        install.status = "read_failed";
        return clone(install);
      }
      if (install.scenario === "validation-failed") {
        install.step = 6;
        install.status = "validation_failed";
        return clone(install);
      }
      install.step = 4;
      install.status = "directory_committed";
      return clone(install);
    }
    if (install.step === 4) {
      install.step = 5;
      return clone(install);
    }
    if (install.step === 5) {
      install.step = 6;
      install.status = install.scenario === "attach-failed"
        ? "committed_attach_failed"
        : install.scenario === "restart-required"
          ? "restart_required"
          : install.scenario === "verified"
            ? "verified"
            : "attached_load_unverified";
      const id = "external-prototype-" + (this.revision + this.skills.length + 1);
      const attached = !["committed_attach_failed", "validation_failed", "read_failed"].includes(install.status);
      this.skills.push({
        id,
        name: install.name.trim(),
        description: "通过安装向导创建的外部 Skill 原型记录。",
        source: "external",
        installMethod: install.method === "github" ? "GitHub URL" : "本地文件夹",
        sourceValue: install.sourceValue.trim(),
        agent: attached ? "OPERON" : "未绑定",
        attached,
        status: install.status,
        purpose: "原型安装",
        readOnly: false,
      });
      install.resultId = id;
      this.revision += 1;
      return clone(install);
    }
    return clone(install);
  }

  retrySkillAttach(id) {
    const skill = this.findSkill(id);
    if (!skillPermission(skill, "retry_attach")) throw new Error("该 Skill 不允许 attach");
    skill.attached = true;
    skill.agent = "OPERON";
    skill.status = "attached_load_unverified";
    return clone(skill);
  }

  verifySkillLoad(id) {
    const skill = this.findSkill(id);
    if (!skillPermission(skill, "verify_load")) throw new Error("该 Skill 不允许验证");
    skill.attached = true;
    skill.agent = "OPERON";
    skill.status = "verified";
    return clone(skill);
  }

  completeSkillRestart(id) {
    const skill = this.findSkill(id);
    if (!skillPermission(skill, "restart")) throw new Error("该 Skill 不允许重启操作");
    skill.attached = true;
    skill.agent = "OPERON";
    skill.status = "verified";
    return clone(skill);
  }

  detachSkill(id) {
    const skill = this.findSkill(id);
    if (!skillPermission(skill, "detach")) throw new Error("该 Skill 不允许 detach");
    skill.attached = false;
    skill.agent = "未绑定";
    skill.status = "not_started";
    return clone(skill);
  }

  uninstallSkill(id) {
    const skill = this.findSkill(id);
    if (!skillPermission(skill, "uninstall")) throw new Error("该 Skill 不允许卸载");
    this.skills = this.skills.filter((item) => item.id !== id);
    return true;
  }

  saveMcp(input, existingId = null) {
    const transport = ["stdio", "http", "sse"].includes(input.transport) ? input.transport : "stdio";
    const valid = String(input.name || "").trim()
      && (transport === "stdio"
        ? String(input.command || "").trim()
        : /^https?:\/\//.test(String(input.url || "").trim()));
    const previous = existingId ? this.findMcp(existingId) : null;
    if (previous && !mcpPermission(previous, "edit")) throw new Error("该 MCP 不允许编辑");
    const enabled = input.enabled !== false;
    const item = {
      id: previous ? previous.id : "external-mcp-" + (this.revision + this.mcps.length + 1),
      name: String(input.name || "未命名 MCP").trim(),
      description: String(input.description || "用户外部添加的 MCP 原型配置。"),
      source: "external",
      transport,
      command: transport === "stdio" ? String(input.command || "").trim() : "",
      args: transport === "stdio" ? String(input.args || "").split(/\s+/).filter(Boolean) : [],
      url: transport === "stdio" ? "" : String(input.url || "").trim(),
      headers: transport === "stdio" ? [] : clone(input.headers || []),
      agent: previous && previous.attached ? "OPERON" : "未绑定",
      enabled,
      attached: !!(previous && previous.attached),
      status: valid ? (enabled ? (previous && previous.attached ? "attached" : "enabled_unattached") : "disconnected") : "config_error",
      managedBy: "用户",
      readOnly: false,
    };
    if (previous) Object.assign(previous, item);
    else this.mcps.push(item);
    this.revision += 1;
    return clone(item);
  }

  toggleMcp(id) {
    const mcp = this.findMcp(id);
    if (!mcpPermission(mcp, "toggle")) throw new Error("该 MCP 不允许启停");
    mcp.enabled = !mcp.enabled;
    if (!mcp.enabled) {
      mcp.attached = false;
      mcp.agent = "未绑定";
      mcp.status = "disconnected";
    } else {
      mcp.status = "enabled_unattached";
    }
    return clone(mcp);
  }

  attachMcp(id) {
    const mcp = this.findMcp(id);
    if (!mcpPermission(mcp, "attach")) throw new Error("该 MCP 不允许 attach");
    if (!mcp.enabled || mcp.status === "config_error") throw new Error("请先修复并启用配置");
    mcp.attached = true;
    mcp.agent = "OPERON";
    mcp.status = "attached";
    return clone(mcp);
  }

  detachMcp(id) {
    const mcp = this.findMcp(id);
    if (!mcpPermission(mcp, "detach")) throw new Error("该 MCP 不允许 detach");
    mcp.attached = false;
    mcp.agent = "未绑定";
    mcp.status = mcp.enabled ? "enabled_unattached" : "disconnected";
    return clone(mcp);
  }

  deleteMcp(id) {
    const mcp = this.findMcp(id);
    if (!mcpPermission(mcp, "delete")) throw new Error("该 MCP 不允许删除");
    this.mcps = this.mcps.filter((item) => item.id !== id);
    return true;
  }
}
