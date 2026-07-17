import {
  FIXTURE_LABELS,
  MCP_STATUSES,
  SKILL_STATUS_FILTERS,
  SKILL_STATUSES,
  SOURCE_LABELS,
  SkillMcpPrototypeStore,
  getMcpFormFields,
  mcpPermission,
  skillPermission,
} from "./skill-mcp-prototype-store.js";

const escapeHtml = (value) => String(value ?? "")
  .replaceAll("&", "&amp;")
  .replaceAll("<", "&lt;")
  .replaceAll(">", "&gt;")
  .replaceAll('"', "&quot;")
  .replaceAll("'", "&#39;");

const MCP_UI_ENABLED = false;

const statusBadge = (kind, status) => {
  const map = kind === "mcp" ? MCP_STATUSES : SKILL_STATUSES;
  const meta = map[status] || { label: status, tone: "neutral" };
  return `<span class="status-badge ${meta.tone}">${escapeHtml(meta.label)}</span>`;
};

const statusText = (kind, status) => {
  const map = kind === "mcp" ? MCP_STATUSES : SKILL_STATUSES;
  const meta = map[status] || { label: status, tone: "neutral" };
  const shortLabels = kind === "mcp" ? {
    disconnected: "未连接",
    enabled_unattached: "待 Attach",
    attached: "已 Attach",
    restart_required: "需重启",
    config_error: "配置错误",
  } : {
    not_started: "未开始",
    reading: "安装中",
    read_failed: "读取失败",
    validation_failed: "校验失败",
    directory_committed: "待 Attach",
    committed_attach_failed: "Attach 失败",
    attached_load_unverified: "待验证",
    verified: "可用",
    restart_required: "需重启",
  };
  return `<span class="entity-status-text ${escapeHtml(meta.tone)}"><i></i>${escapeHtml(shortLabels[status] || meta.label)}</span>`;
};

const sourceText = (source) => {
  const shortLabels = { official: "官方", system: "系统", external: "外部" };
  return `<span class="entity-source-text ${escapeHtml(source)}"><i></i>${escapeHtml(shortLabels[source] || SOURCE_LABELS[source] || source)}</span>`;
};

const skillTypeText = (method) => ({
  "Science 分发": "Science",
  "CSSwitch 内置": "内置",
  "GitHub URL": "GitHub",
  "本地文件夹": "本地",
}[method] || method);

const scenarioForFixture = (fixture) => {
  if (fixture === "attach-failed") return "attach-failed";
  if (fixture === "restart-required") return "restart-required";
  return "healthy";
};

class SkillMcpPrototypeUI {
  constructor(root, fixture) {
    this.root = root;
    this.store = new SkillMcpPrototypeStore(fixture);
    this.tab = "skills";
    this.filters = { query: "", source: "all", status: "all" };
    this.drawer = null;
    this.error = "";
    this.mcpDraft = null;
  }

  mount() {
    this.root.addEventListener("click", (event) => this.onClick(event));
    this.root.addEventListener("input", (event) => this.onInput(event));
    this.root.addEventListener("change", (event) => this.onChange(event));
    this.render();
  }

  render() {
    if (!MCP_UI_ENABLED && this.tab === "mcp") this.tab = "skills";
    const kind = this.tab === "skills" ? "skill" : "mcp";
    const items = this.store.list(kind, this.filters);
    const statusMap = kind === "skill" ? SKILL_STATUS_FILTERS : MCP_STATUSES;
    const fixtureOptions = Object.entries(FIXTURE_LABELS).map(([value, label]) =>
      `<option value="${value}"${this.store.fixture === value ? " selected" : ""}>${escapeHtml(label)}</option>`
    ).join("");
    const statusOptions = Object.entries(statusMap).map(([value, meta]) =>
      `<option value="${value}"${this.filters.status === value ? " selected" : ""}>${escapeHtml(meta.label)}</option>`
    ).join("");

    this.root.innerHTML = `
      <div class="prototype-toolbar">
        <div class="prototype-tabs" role="tablist" aria-label="Skill 与 MCP">
          <button class="prototype-tab ${this.tab === "skills" ? "active" : ""}" type="button" role="tab" aria-selected="${this.tab === "skills"}" data-action="set-tab" data-tab="skills">Skills</button>
          <button class="prototype-tab" type="button" role="tab" aria-selected="false" aria-disabled="true" disabled>MCP 暂未开放</button>
        </div>
        <div class="prototype-tools">
          <label>原型场景 <select data-action="fixture">${fixtureOptions}</select></label>
          <button class="btn" type="button" data-action="reset">重置页面</button>
          <button class="btn primary" type="button" data-action="${this.tab === "skills" ? "open-install" : "open-mcp-form"}">${this.tab === "skills" ? "安装外部 Skill" : "新建外部 MCP"}</button>
        </div>
      </div>
      <section class="surface filter-surface">
        <div class="filter-row">
          <input type="search" placeholder="搜索名称、用途或 Agent" aria-label="搜索" data-filter="query" value="${escapeHtml(this.filters.query)}" />
          <select aria-label="来源" data-filter="source">
            <option value="all">全部来源</option>
            ${Object.entries(SOURCE_LABELS).map(([value, label]) => `<option value="${value}"${this.filters.source === value ? " selected" : ""}>${escapeHtml(label)}</option>`).join("")}
          </select>
          <select aria-label="状态" data-filter="status">
            <option value="all">全部状态</option>${statusOptions}
          </select>
          <button class="btn" type="button" data-action="clear-filters">清除筛选</button>
        </div>
      </section>
      ${this.tab === "mcp" && this.filters.source === "official" ? `<div class="prototype-note">目前没有可靠的官方 MCP 清单，因此官方来源保持空状态，不虚构 connector。</div>` : ""}
      <div class="entity-grid entity-list ${kind === "skill" ? "skill-list" : "mcp-list"}">${items.length ? `<div class="entity-list-head" aria-hidden="true"><span>名称</span><span>来源</span><span>类型</span><span>Agent</span><span>状态</span><span>操作</span></div>${items.map((item) => kind === "skill" ? this.renderSkillCard(item) : this.renderMcpCard(item)).join("")}` : this.renderEmpty(kind)}</div>
      ${this.renderDrawer()}
    `;
  }

  renderEmpty(kind) {
    const officialMcp = kind === "mcp" && this.filters.source === "official";
    return `<div class="prototype-empty"><h3>${officialMcp ? "没有可确认的官方 MCP" : "没有符合条件的项目"}</h3><p>${officialMcp ? "官方清单未验证前，这里保持空白。CSSwitch 系统组件和外部配置会显示在各自来源下。" : "调整搜索或筛选条件，也可以切换原型场景重新查看。"}</p></div>`;
  }

  renderSkillCard(skill) {
    const retry = skill.status === "committed_attach_failed" && skillPermission(skill, "retry_attach")
      ? `<button class="btn" type="button" data-action="skill-retry" data-id="${escapeHtml(skill.id)}">重试</button>` : "";
    const verify = skill.status === "attached_load_unverified" && skillPermission(skill, "verify_load")
      ? `<button class="btn" type="button" data-action="skill-verify" data-id="${escapeHtml(skill.id)}">验证</button>` : "";
    const restart = skill.status === "restart_required" && skillPermission(skill, "restart")
      ? `<button class="btn" type="button" data-action="skill-restart" data-id="${escapeHtml(skill.id)}">重启</button>` : "";
    return `<article class="entity-card entity-row">
      <div class="entity-primary"><h3>${escapeHtml(skill.name)}</h3><p>${escapeHtml(skill.description)}</p></div>
      <div class="entity-cell entity-source">${sourceText(skill.source)}</div>
      <div class="entity-cell entity-type">${escapeHtml(skillTypeText(skill.installMethod))}</div>
      <div class="entity-cell entity-agent">${escapeHtml(skill.agent)}</div>
      <div class="entity-cell entity-status">${statusText("skill", skill.status)}</div>
      <div class="card-actions"><button class="btn" type="button" data-action="skill-detail" data-id="${escapeHtml(skill.id)}">详情</button>${retry}${verify}${restart}</div>
    </article>`;
  }

  renderMcpCard(mcp) {
    const attach = mcpPermission(mcp, "attach") && !mcp.attached
      ? `<button class="btn" type="button" data-action="mcp-attach" data-id="${escapeHtml(mcp.id)}">Attach</button>` : "";
    const detach = mcpPermission(mcp, "detach") && mcp.attached
      ? `<button class="btn" type="button" data-action="mcp-detach" data-id="${escapeHtml(mcp.id)}">Detach</button>` : "";
    const enable = mcpPermission(mcp, "toggle") && !mcp.enabled
      ? `<button class="btn" type="button" data-action="mcp-toggle" data-id="${escapeHtml(mcp.id)}">启用</button>` : "";
    return `<article class="entity-card entity-row">
      <div class="entity-primary"><h3>${escapeHtml(mcp.name)}</h3><p>${escapeHtml(mcp.description)}</p></div>
      <div class="entity-cell entity-source">${sourceText(mcp.source)}</div>
      <div class="entity-cell entity-type">${escapeHtml(mcp.transport)}</div>
      <div class="entity-cell entity-agent">${escapeHtml(mcp.agent)}</div>
      <div class="entity-cell entity-status">${statusText("mcp", mcp.status)}</div>
      <div class="card-actions"><button class="btn" type="button" data-action="mcp-detail" data-id="${escapeHtml(mcp.id)}">详情</button>${enable}${attach}${detach}</div>
    </article>`;
  }

  renderDrawer() {
    if (!this.drawer) return "";
    if (this.drawer.type === "skill-detail") return this.renderSkillDetail(this.store.findSkill(this.drawer.id));
    if (this.drawer.type === "mcp-detail") return this.renderMcpDetail(this.store.findMcp(this.drawer.id));
    if (this.drawer.type === "install") return this.renderInstallWizard();
    if (this.drawer.type === "mcp-form") return this.renderMcpForm();
    if (this.drawer.type === "confirm") return this.renderConfirm();
    return "";
  }

  drawerShell(kicker, title, body) {
    return `<div class="prototype-drawer-backdrop" data-action="backdrop-close"><aside class="prototype-drawer" role="dialog" aria-modal="true" aria-label="${escapeHtml(title)}"><div class="drawer-head"><div><div class="section-kicker">${escapeHtml(kicker)}</div><h2>${escapeHtml(title)}</h2></div><button class="drawer-close" type="button" data-action="close-drawer">关闭</button></div><div class="drawer-body">${body}</div></aside></div>`;
  }

  renderSkillDetail(skill) {
    if (!skill) return "";
    const canManage = skill.source === "external";
    const retry = skill.status === "committed_attach_failed" ? `<button class="btn" data-action="skill-retry" data-id="${escapeHtml(skill.id)}">重试 attach</button>` : "";
    const verify = skill.status === "attached_load_unverified" ? `<button class="btn" data-action="skill-verify" data-id="${escapeHtml(skill.id)}">模拟 load 验证</button>` : "";
    const restart = skill.status === "restart_required" ? `<button class="btn" data-action="skill-restart" data-id="${escapeHtml(skill.id)}">模拟重启完成</button>` : "";
    const manage = canManage ? `<button class="btn" data-action="skill-detach" data-id="${escapeHtml(skill.id)}" ${skill.attached ? "" : "disabled"}>detach</button><button class="btn danger" data-action="confirm-skill-delete" data-id="${escapeHtml(skill.id)}">隔离卸载</button>` : "";
    const readOnlyNote = canManage ? "" : `<div class="prototype-note">${skill.source === "official" ? "官方 Skill 由 Science / Anthropic 分发" : "系统 Skill 由 CSSwitch 管理"}，此处只读，不提供卸载操作。</div>`;
    const body = `${statusBadge("skill", skill.status)}<p class="detail-description">${escapeHtml(skill.description)}</p>${readOnlyNote}<div class="detail-list">
      <div class="detail-row"><span>来源</span><strong>${escapeHtml(SOURCE_LABELS[skill.source])}</strong></div>
      <div class="detail-row"><span>安装方式</span><strong>${escapeHtml(skill.installMethod)}</strong></div>
      <div class="detail-row"><span>来源位置</span><strong>${escapeHtml(skill.sourceValue || "由分发方管理")}</strong></div>
      <div class="detail-row"><span>Agent</span><strong>${escapeHtml(skill.agent)}</strong></div>
      <div class="detail-row"><span>加载判断</span><strong>${escapeHtml(SKILL_STATUSES[skill.status]?.label || skill.status)}</strong></div>
    </div><div class="detail-actions">${retry}${verify}${restart}${manage}</div>`;
    return this.drawerShell("SKILL DETAILS", skill.name, body);
  }

  renderMcpDetail(mcp) {
    if (!mcp) return "";
    const endpoint = mcp.transport === "stdio" ? [mcp.command, ...(mcp.args || [])].join(" ") : mcp.url;
    const headerSummary = (mcp.headers || []).length ? mcp.headers.map((header) => `${header.key}: ••••••`).join("；") : "无";
    const edit = mcpPermission(mcp, "edit") ? `<button class="btn" data-action="open-mcp-edit" data-id="${escapeHtml(mcp.id)}">编辑</button>` : "";
    const toggle = mcpPermission(mcp, "toggle") ? `<button class="btn" data-action="mcp-toggle" data-id="${escapeHtml(mcp.id)}">${mcp.enabled ? "停用" : "启用"}</button>` : "";
    const attach = mcpPermission(mcp, "attach") && !mcp.attached ? `<button class="btn" data-action="mcp-attach" data-id="${escapeHtml(mcp.id)}">attach 到 OPERON</button>` : "";
    const detach = mcpPermission(mcp, "detach") && mcp.attached ? `<button class="btn" data-action="mcp-detach" data-id="${escapeHtml(mcp.id)}">detach</button>` : "";
    const remove = mcpPermission(mcp, "delete") ? `<button class="btn danger" data-action="confirm-mcp-delete" data-id="${escapeHtml(mcp.id)}">删除</button>` : "";
    const note = mcp.source === "system" ? `<div class="prototype-note">这是 CSSwitch 托管的系统 connector，只展示用途与绑定状态，不提供编辑或删除。</div>` : "";
    const body = `${statusBadge("mcp", mcp.status)}<p class="detail-description">${escapeHtml(mcp.description)}</p>${note}<div class="detail-list">
      <div class="detail-row"><span>来源</span><strong>${escapeHtml(SOURCE_LABELS[mcp.source])}</strong></div>
      <div class="detail-row"><span>Transport</span><strong>${escapeHtml(mcp.transport)}</strong></div>
      <div class="detail-row"><span>Command / URL</span><strong>${escapeHtml(endpoint || "未配置")}</strong></div>
      <div class="detail-row"><span>Headers</span><strong>${escapeHtml(headerSummary)}</strong></div>
      <div class="detail-row"><span>Agent</span><strong>${escapeHtml(mcp.agent)}</strong></div>
      <div class="detail-row"><span>托管方</span><strong>${escapeHtml(mcp.managedBy)}</strong></div>
    </div><div class="detail-actions">${edit}${toggle}${attach}${detach}${remove}</div>`;
    return this.drawerShell("MCP DETAILS", mcp.name, body);
  }

  installStepBody(install) {
    if (install.step === 1) {
      return `<div class="wizard-step-label">STEP 1 · 选择来源</div><div class="wizard-form"><div class="choice-grid"><button class="choice-card ${install.method === "github" ? "active" : ""}" type="button" data-action="install-method" data-method="github"><strong>GitHub URL</strong><span>模拟从公开仓库读取 Skill 目录。</span></button><button class="choice-card ${install.method === "folder" ? "active" : ""}" type="button" data-action="install-method" data-method="folder"><strong>本地文件夹</strong><span>模拟读取你已选择的本地目录。</span></button></div><div class="field"><label>Skill 名称</label><input aria-label="Skill 名称" data-install-field="name" value="${escapeHtml(install.name)}" /></div><div class="field"><label>${install.method === "github" ? "GitHub URL" : "本地文件夹路径"}</label><input aria-label="Skill 来源" data-install-field="sourceValue" value="${escapeHtml(install.sourceValue)}" /></div><div class="field"><label>模拟结果</label><select aria-label="Skill 安装模拟结果" data-install-field="scenario"><option value="healthy"${install.scenario === "healthy" ? " selected" : ""}>已 attach，load 未验证</option><option value="verified"${install.scenario === "verified" ? " selected" : ""}>已验证可用</option><option value="read-failed"${install.scenario === "read-failed" ? " selected" : ""}>下载 / 读取失败</option><option value="validation-failed"${install.scenario === "validation-failed" ? " selected" : ""}>安全校验失败</option><option value="attach-failed"${install.scenario === "attach-failed" ? " selected" : ""}>文件提交后 attach 失败</option><option value="restart-required"${install.scenario === "restart-required" ? " selected" : ""}>需要重启</option></select></div></div>`;
    }
    if (install.step === 2) {
      return `<div class="wizard-step-label">STEP 2 · 确认来源</div><div class="wizard-summary"><div class="wizard-summary-row"><span>安装方式</span><strong>${install.method === "github" ? "GitHub URL" : "本地文件夹"}</strong></div><div class="wizard-summary-row"><span>Skill 名称</span><strong>${escapeHtml(install.name)}</strong></div><div class="wizard-summary-row"><span>来源</span><strong>${escapeHtml(install.sourceValue)}</strong></div><div class="wizard-summary-row"><span>目标 Agent</span><strong>OPERON</strong></div></div><div class="prototype-note" style="margin-top:12px">下一步仅模拟读取与安全校验，不会访问网络或本地文件。</div>`;
    }
    if (install.step === 3) return `<div class="wizard-step-label">STEP 3 · 下载 / 读取与校验</div><div class="stage-panel"><div class="stage-marker">READ</div><h3>来源已进入模拟读取阶段</h3><p>将检查目录结构、路径边界与基础文件规则。不会判断第三方内容是否可信。</p></div>`;
    if (install.step === 4) return `<div class="wizard-step-label">STEP 4 · 提交目录</div><div class="stage-panel"><div class="stage-marker">FILES</div><h3>校验通过，准备模拟提交文件</h3><p>“文件已提交”只代表目录阶段完成，不能据此显示 Skill 已可用。</p></div>`;
    if (install.step === 5) return `<div class="wizard-step-label">STEP 5 · attach 到 OPERON</div><div class="stage-panel"><div class="stage-marker">AGENT</div><h3>文件已提交，等待 attach</h3><p>attach 是独立阶段。失败时会保留“文件已提交”的真实状态，并提供重试与安全卸载。</p></div>`;
    const meta = SKILL_STATUSES[install.status] || SKILL_STATUSES.not_started;
    const failed = ["read_failed", "validation_failed", "committed_attach_failed"].includes(install.status);
    const success = install.status === "verified";
    const explanatory = {
      read_failed: "来源读取失败，未提交任何文件，也没有执行 attach。",
      validation_failed: "安全校验未通过，未提交文件，也没有执行 attach。",
      committed_attach_failed: "文件已经提交，但 attach 到 OPERON 失败；当前不能视为可用。",
      attached_load_unverified: "已 attach 到 OPERON，但尚未证明运行时能够 load。",
      verified: "本次模拟已完成 attach 与 load 验证，可视为可用。",
      restart_required: "已 attach，但必须重启隔离 Science 后才能确认可用。",
    }[install.status] || "流程尚未完成。";
    let actions = `<button class="btn" type="button" data-action="close-drawer">关闭</button>`;
    if (install.status === "committed_attach_failed" && install.resultId) actions = `<button class="btn primary" data-action="skill-retry" data-id="${escapeHtml(install.resultId)}">重试 attach</button><button class="btn danger" data-action="confirm-skill-delete" data-id="${escapeHtml(install.resultId)}">安全卸载</button>`;
    if (install.status === "attached_load_unverified" && install.resultId) actions = `<button class="btn primary" data-action="skill-verify" data-id="${escapeHtml(install.resultId)}">模拟 load 验证</button><button class="btn" data-action="close-drawer">稍后处理</button>`;
    if (install.status === "restart_required" && install.resultId) actions = `<button class="btn primary" data-action="skill-restart" data-id="${escapeHtml(install.resultId)}">模拟重启完成</button><button class="btn" data-action="close-drawer">稍后处理</button>`;
    return `<div class="wizard-step-label">STEP 6 · 结果</div><div class="stage-panel ${failed ? "failure" : success ? "success" : ""}"><div class="stage-marker">${failed ? "STOP" : success ? "READY" : "CHECK"}</div><h3>${escapeHtml(meta.label)}</h3><p>${escapeHtml(explanatory)}</p></div><div class="wizard-actions">${actions}</div>`;
  }

  renderInstallWizard() {
    const install = this.store.install || this.store.beginSkillInstall(scenarioForFixture(this.store.fixture));
    const steps = Array.from({ length: 6 }, (_, index) => `<span class="wizard-step ${index + 1 < install.step ? "done" : index + 1 === install.step ? "active" : ""}"></span>`).join("");
    const nextLabels = { 1: "继续确认", 2: "模拟读取与校验", 3: "继续提交目录", 4: "继续 attach", 5: "执行 attach" };
    const navigation = install.step < 6 ? `<div class="wizard-actions"><button class="btn" type="button" data-action="close-drawer">取消</button><button class="btn primary" type="button" data-action="install-next">${escapeHtml(nextLabels[install.step])}</button></div>` : "";
    const error = this.error ? `<div class="prototype-note" style="border-left-color:var(--red);background:var(--red-soft)">${escapeHtml(this.error)}</div>` : "";
    return this.drawerShell("EXTERNAL SKILL", "安装外部 Skill", `<div class="wizard-steps">${steps}</div>${error}${this.installStepBody(install)}${navigation}`);
  }

  renderMcpForm() {
    const draft = this.mcpDraft;
    const fields = getMcpFormFields(draft.transport);
    const argsField = fields.includes("args") ? `<div class="field"><label>Args（空格分隔）</label><input aria-label="MCP Args" data-mcp-field="args" value="${escapeHtml(draft.args || "")}" placeholder="--port 3000" /></div>` : "";
    const endpointFields = draft.transport === "stdio"
      ? `<div class="field"><label>Command</label><input aria-label="MCP Command" data-mcp-field="command" value="${escapeHtml(draft.command || "")}" placeholder="node 或可执行文件路径" /></div>${argsField}`
      : `<div class="field"><label>URL</label><input aria-label="MCP URL" data-mcp-field="url" value="${escapeHtml(draft.url || "")}" placeholder="https://mcp.example.com/${draft.transport}" /></div><div class="field"><label>Headers（每行 Key: Value）</label><textarea aria-label="MCP Headers" data-mcp-field="headersText" placeholder="Authorization: Bearer token">${escapeHtml(draft.headersText || "")}</textarea></div><div class="secret-note">Header 值只存在当前页面内存中；详情默认遮罩，不写入 localStorage 或日志。</div>`;
    const error = this.error ? `<div class="prototype-note" style="border-left-color:var(--red);background:var(--red-soft)">${escapeHtml(this.error)}</div>` : "";
    const body = `${error}<div class="wizard-form"><div class="field"><label>名称</label><input aria-label="MCP 名称" data-mcp-field="name" value="${escapeHtml(draft.name || "")}" placeholder="my-mcp" /></div><div class="field"><label>Transport</label><select aria-label="MCP Transport" data-mcp-field="transport"><option value="stdio"${draft.transport === "stdio" ? " selected" : ""}>stdio</option><option value="http"${draft.transport === "http" ? " selected" : ""}>http</option><option value="sse"${draft.transport === "sse" ? " selected" : ""}>sse</option></select></div>${endpointFields}<label class="ssh-bridge-toggle"><input type="checkbox" aria-label="保存后启用" data-mcp-field="enabled" ${draft.enabled ? "checked" : ""} /><span>保存后启用</span></label></div><div class="wizard-actions"><button class="btn" data-action="close-drawer">取消</button><button class="btn primary" data-action="save-mcp">保存${draft.id ? "修改" : "草稿"}</button></div>`;
    return this.drawerShell("EXTERNAL MCP", draft.id ? "编辑 MCP" : "新建 MCP", body);
  }

  renderConfirm() {
    const { entity, id } = this.drawer;
    const item = entity === "skill" ? this.store.findSkill(id) : this.store.findMcp(id);
    if (!item) return "";
    const verb = entity === "skill" ? "隔离卸载" : "删除";
    const description = entity === "skill"
      ? "将先模拟 detach，再从 CSSwitch 管理区安全移除。此动作不会触碰真实文件。"
      : "将从当前页面内存移除该外部 MCP 配置；刷新页面会恢复 fixture。";
    return this.drawerShell("CONFIRM", `${verb} ${item.name}`, `<div class="confirm-box"><p>${escapeHtml(description)}</p><div class="detail-actions"><button class="btn" data-action="close-drawer">取消</button><button class="btn danger" data-action="confirm-delete-now" data-entity="${entity}" data-id="${escapeHtml(id)}">确认${verb}</button></div></div>`);
  }

  openMcpForm(id = null) {
    const item = id ? this.store.findMcp(id) : null;
    this.mcpDraft = item ? {
      id: item.id,
      name: item.name,
      transport: item.transport,
      command: item.command || "",
      args: (item.args || []).join(" "),
      url: item.url || "",
      headersText: (item.headers || []).map((header) => `${header.key}: ${header.value}`).join("\n"),
      enabled: item.enabled,
    } : { id: null, name: "", transport: "stdio", command: "", args: "", url: "", headersText: "", enabled: true };
    this.drawer = { type: "mcp-form" };
    this.error = "";
  }

  onClick(event) {
    const target = event.target.closest("[data-action]");
    if (!target) return;
    const action = target.dataset.action;
    const id = target.dataset.id;
    if (action === "backdrop-close" && event.target !== target) return;
    try {
      if (action === "set-tab") {
        const nextTab = target.dataset.tab;
        if (nextTab === "mcp" && !MCP_UI_ENABLED) return;
        this.tab = nextTab;
        this.filters = { query: "", source: "all", status: "all" };
        this.drawer = null;
      }
      else if (action === "reset") { this.store.reset(); this.drawer = null; this.error = ""; }
      else if (action === "clear-filters") this.filters = { query: "", source: "all", status: "all" };
      else if (action === "close-drawer" || action === "backdrop-close") { this.drawer = null; this.error = ""; }
      else if (action === "open-install") { this.store.beginSkillInstall(scenarioForFixture(this.store.fixture)); this.drawer = { type: "install" }; this.error = ""; }
      else if (action === "install-method") { this.store.updateSkillInstall({ method: target.dataset.method, sourceValue: target.dataset.method === "github" ? "https://github.com/anthropics/skills/tree/main/skills/internal-comms" : "/Users/demo/skills/my-skill" }); }
      else if (action === "install-next") { this.store.advanceSkillInstall(); this.error = ""; }
      else if (action === "skill-detail") this.drawer = { type: "skill-detail", id };
      else if (action === "skill-retry") { this.store.retrySkillAttach(id); this.drawer = { type: "skill-detail", id }; }
      else if (action === "skill-verify") { this.store.verifySkillLoad(id); this.drawer = { type: "skill-detail", id }; }
      else if (action === "skill-restart") { this.store.completeSkillRestart(id); this.drawer = { type: "skill-detail", id }; }
      else if (action === "skill-detach") { this.store.detachSkill(id); this.drawer = { type: "skill-detail", id }; }
      else if (action === "confirm-skill-delete") this.drawer = { type: "confirm", entity: "skill", id };
      else if (action === "open-mcp-form") this.openMcpForm();
      else if (action === "open-mcp-edit") this.openMcpForm(id);
      else if (action === "mcp-detail") this.drawer = { type: "mcp-detail", id };
      else if (action === "mcp-toggle") this.store.toggleMcp(id);
      else if (action === "mcp-attach") { this.store.attachMcp(id); this.drawer = this.drawer ? { type: "mcp-detail", id } : null; }
      else if (action === "mcp-detach") { this.store.detachMcp(id); this.drawer = this.drawer ? { type: "mcp-detail", id } : null; }
      else if (action === "confirm-mcp-delete") this.drawer = { type: "confirm", entity: "mcp", id };
      else if (action === "confirm-delete-now") { if (target.dataset.entity === "skill") this.store.uninstallSkill(id); else this.store.deleteMcp(id); this.drawer = null; }
      else if (action === "save-mcp") this.saveMcpDraft();
      this.render();
    } catch (error) {
      this.error = String(error.message || error);
      this.render();
    }
  }

  onInput(event) {
    const filter = event.target.dataset.filter;
    if (filter) { this.filters[filter] = event.target.value; this.render(); return; }
    const installField = event.target.dataset.installField;
    if (installField) { this.store.updateSkillInstall({ [installField]: event.target.value }); return; }
    const mcpField = event.target.dataset.mcpField;
    if (mcpField) this.mcpDraft[mcpField] = event.target.type === "checkbox" ? event.target.checked : event.target.value;
  }

  onChange(event) {
    if (event.target.dataset.action === "fixture") {
      this.store.reset(event.target.value);
      this.filters = { query: "", source: "all", status: "all" };
      this.drawer = null;
      this.render();
      return;
    }
    const filter = event.target.dataset.filter;
    if (filter) { this.filters[filter] = event.target.value; this.render(); return; }
    const installField = event.target.dataset.installField;
    if (installField) { this.store.updateSkillInstall({ [installField]: event.target.value }); this.render(); return; }
    const mcpField = event.target.dataset.mcpField;
    if (mcpField) {
      this.mcpDraft[mcpField] = event.target.type === "checkbox" ? event.target.checked : event.target.value;
      if (mcpField === "transport") this.render();
    }
  }

  saveMcpDraft() {
    const headers = String(this.mcpDraft.headersText || "").split("\n").map((line) => {
      const index = line.indexOf(":");
      return index > 0 ? { key: line.slice(0, index).trim(), value: line.slice(index + 1).trim() } : null;
    }).filter(Boolean);
    const saved = this.store.saveMcp({ ...this.mcpDraft, headers }, this.mcpDraft.id);
    this.drawer = { type: "mcp-detail", id: saved.id };
    this.error = "";
  }
}

export function mountSkillMcpPrototype(root, fixture = "healthy") {
  const ui = new SkillMcpPrototypeUI(root, fixture);
  ui.mount();
  return ui;
}
