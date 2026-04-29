"use strict";

// ============================================================
// Cluster Management — admin UI
//
// State flow:
//   loadAll()  → loadOverview() + loadKeys() + loadSettings()
//                + loadModels() + loadInferenceLog()
//   navigate() → switches visible section (no fetch — data already
//                refreshes every REFRESH_MS)
// ============================================================

const $ = (id) => document.getElementById(id);
const KEY_STORAGE = "gpucluster.adminKey";
const ROUTE_STORAGE = "gpucluster.route";
const REFRESH_MS = 5000;
const ROUTES = ["overview", "nodes", "models", "keys", "inference", "settings"];
const ROUTE_TITLES = {
  overview:  ["Overview",      "live cluster status"],
  nodes:     ["Nodes",         "live + registered"],
  models:    ["Models",        "registry powering /v1/models"],
  keys:      ["API keys",      "bearer tokens for /v1/*"],
  inference: ["Inference log", "recent /v1/chat/completions calls"],
  settings:  ["Settings",      "cluster-wide configuration"],
};

let lastData = null;
let lastEffectiveUrl = "";
let lastSettings = null;

// ============================================================
// helpers
// ============================================================
function fmt(n) { return Number.isFinite(n) ? n.toLocaleString("en-US") : "—"; }

function relTime(iso) {
  if (!iso) return "—";
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return iso;
  const diff = Math.max(0, (Date.now() - t) / 1000);
  if (diff < 60)    return Math.floor(diff) + "s ago";
  if (diff < 3600)  return Math.floor(diff / 60) + "m ago";
  if (diff < 86400) return Math.floor(diff / 3600) + "h ago";
  return Math.floor(diff / 86400) + "d ago";
}

function escapeHtml(v) {
  if (v === null || v === undefined) return "";
  return String(v).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  }[c]));
}

function pillFor(status) {
  if (!status) return '<span class="pill muted">unknown</span>';
  const s = String(status).toLowerCase();
  if (s === "ok" || s === "online" || s === "healthy" || s === "available")
    return '<span class="pill ok">' + escapeHtml(status) + '</span>';
  if (s === "down" || s === "offline" || s === "revoked" || s === "quarantined" || s === "error")
    return '<span class="pill err">' + escapeHtml(status) + '</span>';
  if (s === "degraded" || s === "draining" || s === "pending_approval" || s === "loading" || s === "downloading")
    return '<span class="pill warn">' + escapeHtml(status) + '</span>';
  return '<span class="pill info">' + escapeHtml(status) + '</span>';
}

// ============================================================
// toast
// ============================================================
function showToast(msg, ok = false) {
  const t = $("toast");
  t.textContent = msg;
  t.classList.toggle("ok", !!ok);
  t.classList.add("show");
  clearTimeout(t._timer);
  t._timer = setTimeout(() => t.classList.remove("show"), 4000);
}

// ============================================================
// modal (replaces prompt() / confirm())
// ============================================================
let modalResolver = null;

function closeModal(value) {
  const dlg = $("modal");
  if (!dlg.open) return;
  // Capture-then-null BEFORE dlg.close() — the native "close" event fires
  // synchronously and would otherwise resolve with the wrong value.
  const r = modalResolver;
  modalResolver = null;
  dlg.close();
  if (r) r(value);
}

function openModal({ title, body, actions }) {
  return new Promise((resolve) => {
    modalResolver = resolve;
    $("modal-title").textContent = title;
    $("modal-body").innerHTML = body || "";
    const foot = $("modal-foot");
    foot.innerHTML = "";
    (actions || []).forEach((a) => {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "btn " + (a.kind || "");
      btn.textContent = a.label;
      btn.addEventListener("click", () => {
        if (a.handler) {
          const result = a.handler();
          if (result === false) return; // handler aborts close
          closeModal(result === undefined ? a.value : result);
        } else {
          closeModal(a.value);
        }
      });
      foot.appendChild(btn);
    });
    $("modal").showModal();
    // focus first input or first non-cancel button
    const firstInput = $("modal-body").querySelector("input, select, textarea");
    if (firstInput) firstInput.focus();
  });
}

function confirmModal(title, message, { confirmLabel = "Confirm", danger = false } = {}) {
  return openModal({
    title,
    body: `<p>${escapeHtml(message)}</p>`,
    actions: [
      { label: "Cancel", value: false },
      { label: confirmLabel, kind: danger ? "danger" : "primary", value: true },
    ],
  });
}

// ============================================================
// mgmt API
// ============================================================
function adminKey() { return ($("adminKey").value || "").trim(); }
function adminHeaders(extra = {}) {
  const k = adminKey();
  const h = { ...extra };
  if (k) h["Authorization"] = "Bearer " + k;
  return h;
}
async function mgmtCall(method, path, body) {
  const opts = { method, headers: adminHeaders({ "Content-Type": "application/json" }) };
  if (body !== undefined) opts.body = JSON.stringify(body);
  const r = await fetch(path, opts);
  let data = null;
  try { data = await r.json(); } catch (_) { /* may be empty */ }
  if (!r.ok) {
    const err = (data && (data.error || data.message)) || ("HTTP " + r.status);
    throw new Error(err);
  }
  return data;
}

// ============================================================
// overview / KPIs / services / donut
// ============================================================
function renderKpis(data) {
  const services = data.services || {};
  const order = ["gateway", "mgmt", "coordinator", "openai_api"];
  const okCount = order.filter((k) => (services[k] || {}).status === "ok").length;
  const total = order.length;

  $("kpi-services-val").textContent = okCount;
  $("kpi-services-total").textContent = " / " + total;
  $("kpi-services-detail").innerHTML = okCount === total
    ? '<span class="ok">all services healthy</span>'
    : '<span class="err">' + (total - okCount) + " degraded</span>";
  $("kpi-services").className = "kpi " + (okCount === total ? "ok" : (okCount === 0 ? "err" : "warn"));

  const liveNodes = (data.coordinator && Array.isArray(data.coordinator.nodes)) ? data.coordinator.nodes : [];
  $("kpi-live-nodes-val").textContent = fmt(liveNodes.length);
  const onlineLive = liveNodes.filter((n) =>
    (n.status || "").toLowerCase() === "online" || (n.status || "").toLowerCase() === "ok").length;
  $("kpi-live-nodes-detail").innerHTML = liveNodes.length
    ? '<span class="ok">' + onlineLive + ' online</span> · ' + (liveNodes.length - onlineLive) + ' other'
    : "no heartbeats received";

  let mgmtNodes = data.mgmt;
  if (!Array.isArray(mgmtNodes)) {
    if (mgmtNodes && Array.isArray(mgmtNodes.nodes)) mgmtNodes = mgmtNodes.nodes;
    else mgmtNodes = [];
  }
  const isAuthErr = data.mgmt && data.mgmt.error === "upstream_status" && data.mgmt.status === 401;
  $("kpi-registered-val").textContent = isAuthErr ? "—" : fmt(mgmtNodes.length);
  $("kpi-registered-detail").textContent = isAuthErr
    ? "401 — set admin key"
    : (mgmtNodes.length ? mgmtNodes.length + " in registry" : "registry is empty");

  const models = (data.openai_api && Array.isArray(data.openai_api.data)) ? data.openai_api.data : [];
  $("kpi-models-val").textContent = fmt(models.length);
  $("kpi-models-detail").textContent = models.length ? "ready to serve" : "no models published";

  // sidebar badges
  $("nav-count-nodes").textContent = liveNodes.length || "—";
  $("nav-count-models").textContent = models.length || "—";

  return { liveNodes, mgmtNodes, models, isAuthErr };
}

function renderServices(services) {
  const meta = {
    gateway:     { label: "Gateway",      upstream: "self" },
    mgmt:        { label: "Mgmt backend", upstream: "/api/* → :7100" },
    coordinator: { label: "Coordinator",  upstream: "/cluster/* → :7001" },
    openai_api:  { label: "OpenAI API",   upstream: "/v1/* → :7200" },
  };
  $("services").innerHTML = Object.keys(meta).map((k) => {
    const v = services[k] || { status: "unknown" };
    const ok = v.status === "ok";
    return `
      <div class="svc">
        <span class="dot ${ok ? "ok" : "err"}"></span>
        <span class="name">${meta[k].label}</span>
        <span class="upstream">${meta[k].upstream}</span>
        ${pillFor(v.status)}
      </div>`;
  }).join("");
}

const STATUS_COLORS = {
  online: "var(--ok)", ok: "var(--ok)",
  pending_approval: "var(--warn)", degraded: "var(--warn)", draining: "var(--warn)",
  offline: "var(--muted)",
  quarantined: "var(--err)", revoked: "var(--err)",
};
const colorFor = (s) => STATUS_COLORS[(s || "").toLowerCase()] || "var(--info)";

function renderDonut(nodes) {
  const segments = $("donut-segments");
  const total = nodes.length;
  $("donut-total").textContent = total;

  if (total === 0) {
    segments.innerHTML = "";
    $("legend").innerHTML = '<div class="row"><span class="swatch" style="background: var(--muted)"></span><span>no nodes registered yet</span><span class="count">0</span></div>';
    return;
  }

  const tally = {};
  nodes.forEach((n) => {
    const k = (n.status || "unknown").toLowerCase();
    tally[k] = (tally[k] || 0) + 1;
  });
  const entries = Object.entries(tally).sort((a, b) => b[1] - a[1]);

  const C = 2 * Math.PI * 15.915;
  let offset = 25;
  segments.innerHTML = entries.map(([status, count]) => {
    const len = (count / total) * C;
    const seg = `<circle cx="21" cy="21" r="15.915" fill="none"
       stroke="${colorFor(status)}" stroke-width="3.5"
       stroke-dasharray="${len} ${C - len}" stroke-dashoffset="${-offset}"
       transform="rotate(-90 21 21)"></circle>`;
    offset += len;
    return seg;
  }).join("");

  $("legend").innerHTML = entries.map(([status, count]) => `
    <div class="row">
      <span class="swatch" style="background: ${colorFor(status)}"></span>
      <span>${escapeHtml(status)}</span>
      <span class="count">${count} (${Math.round(count / total * 100)}%)</span>
    </div>`).join("");
}

function renderQuickstart(ctx, keysCount) {
  // Show quickstart only when something material is missing.
  const noKeys   = keysCount === 0;
  const noModels = ctx.models.length === 0;
  const noNodes  = ctx.liveNodes.length === 0 && ctx.mgmtNodes.length === 0;

  const show = noKeys || noModels || noNodes;
  $("quickstart").hidden = !show;
  $("qs-key").classList.toggle("done",   !noKeys);
  $("qs-model").classList.toggle("done", !noModels);
  $("qs-node").classList.toggle("done",  !noNodes);
}

// ============================================================
// node tables
// ============================================================
function fillTable(tableId, rows, columns, filterId, emptyHint) {
  const tbl = $(tableId);
  const tbody = tbl.querySelector("tbody");
  const filter = ($(filterId).value || "").toLowerCase().trim();
  const filtered = filter
    ? rows.filter((r) => columns.some((c) => String(r[c.key] ?? "").toLowerCase().includes(filter)))
    : rows;

  if (filtered.length === 0) {
    tbody.innerHTML = `<tr><td colspan="${columns.length}" class="empty">
      ${rows.length === 0 ? emptyHint : "no matches"}
    </td></tr>`;
    return;
  }

  tbody.innerHTML = filtered.map((r) => "<tr>" + columns.map((c) => {
    const raw = r[c.key];
    const cell = c.render ? c.render(raw, r) : escapeHtml(raw);
    return `<td class="${c.cls || ""}">${cell}</td>`;
  }).join("") + "</tr>").join("");
}

function renderTables(ctx) {
  $("coord-count").textContent  = ctx.liveNodes.length;
  $("mgmt-count").textContent   = ctx.isAuthErr ? "401" : ctx.mgmtNodes.length;

  fillTable("tbl-coord", ctx.liveNodes, [
    { key: "id", cls: "id" },
    { key: "hostname", render: (v) => escapeHtml(v || "—") },
    { key: "status", render: (v) => pillFor(v) },
    { key: "last_heartbeat", cls: "muted", render: (v) => escapeHtml(v || "—") },
    { key: "last_heartbeat", cls: "muted", render: (v) => escapeHtml(relTime(v)) },
  ], "filter-coord", "no live nodes — workers haven't checked in yet");

  if (ctx.isAuthErr) {
    $("tbl-mgmt").querySelector("tbody").innerHTML =
      '<tr><td colspan="5" class="empty">unauthorized — paste your <code>ADMIN_API_KEY</code> in the sidebar</td></tr>';
    return;
  }
  fillTable("tbl-mgmt", ctx.mgmtNodes, [
    { key: "id", cls: "id" },
    { key: "display_name", render: (v) => escapeHtml(v || "—") },
    { key: "status", render: (v) => pillFor(v) },
    { key: "first_seen", cls: "muted", render: (v) => escapeHtml(v ? relTime(v) : "—") },
    { key: "id", render: (id, row) => nodeActionsHtml(id, row) },
  ], "filter-mgmt", "no nodes registered — generate an enrollment token");
}

function nodeActionsHtml(id, row) {
  const status = (row.status || "").toLowerCase();
  const can = (s) => Array.isArray(s) ? s.includes(status) : status === s;
  const safeId = escapeHtml(id);
  const btn = (action, label, cls = "") =>
    `<button class="row-action ${cls}" data-action="${action}" data-id="${safeId}">${label}</button>`;
  const parts = [];
  if (status === "pending_approval") parts.push(btn("approve", "Approve", "approve"));
  if (can(["online", "degraded"]))   parts.push(btn("drain",   "Drain"));
  if (status !== "revoked")          parts.push(btn("revoke",  "Revoke", "danger"));
  return `<div class="row-actions">${parts.join("")}</div>`;
}

async function handleNodeAction(action, id) {
  if (action !== "approve") {
    const ok = await confirmModal(
      `${action[0].toUpperCase()}${action.slice(1)} node?`,
      `This will ${action} node ${id}.`,
      { confirmLabel: action, danger: action === "revoke" }
    );
    if (!ok) return;
  }
  try {
    await mgmtCall("POST", `/api/v1/nodes/${encodeURIComponent(id)}/${action}`);
    showToast(`Node ${action}d`, true);
    loadOverview();
  } catch (e) {
    showToast(`${action} failed: ${e.message}`);
  }
}

// ============================================================
// API keys
// ============================================================
function keyActionsHtml(row) {
  const safeId = escapeHtml(row.id);
  const safeName = escapeHtml(row.name || "");
  const safeScope = escapeHtml(row.scope || "");
  const revoked = !!row.revoked_at;
  const editBtn = `<button class="row-action" data-action="key-edit"
                   data-id="${safeId}" data-name="${safeName}" data-scope="${safeScope}">Edit</button>`;
  const revokeBtn = revoked
    ? `<button class="row-action" disabled>Revoked</button>`
    : `<button class="row-action" data-action="key-revoke" data-id="${safeId}">Revoke</button>`;
  const deleteBtn = `<button class="row-action danger" data-action="key-delete" data-id="${safeId}">Delete</button>`;
  return `<div class="row-actions">${editBtn}${revokeBtn}${deleteBtn}</div>`;
}

function keyStatusPill(row) {
  if (row.revoked_at) return '<span class="pill err">revoked</span>';
  if (row.expires_at && new Date(row.expires_at) < new Date())
    return '<span class="pill warn">expired</span>';
  return '<span class="pill ok">active</span>';
}

async function loadKeys() {
  if (!adminKey()) {
    $("tbl-keys").querySelector("tbody").innerHTML =
      '<tr><td colspan="7" class="empty">enter admin key in the sidebar to view…</td></tr>';
    $("keys-count").textContent = "—";
    $("nav-count-keys").textContent = "—";
    return;
  }
  try {
    const rows = await mgmtCall("GET", "/api/v1/keys");
    const filter = ($("filter-keys").value || "").toLowerCase().trim();
    const filtered = filter
      ? rows.filter((r) => ["name", "prefix", "scope"].some((k) =>
          String(r[k] ?? "").toLowerCase().includes(filter)))
      : rows;
    $("keys-count").textContent = rows.length;
    $("nav-count-keys").textContent = rows.length || "—";
    const tbody = $("tbl-keys").querySelector("tbody");
    if (filtered.length === 0) {
      tbody.innerHTML = `<tr><td colspan="7" class="empty">${rows.length === 0
        ? "no API keys yet — fill the form above to create one"
        : "no matches"}</td></tr>`;
      // also re-evaluate quickstart since the count just changed
      if (lastData) renderQuickstart(renderKpis(lastData), rows.length);
      return;
    }
    if (lastData) renderQuickstart(renderKpis(lastData), rows.length);
    tbody.innerHTML = filtered.map((r) => `
      <tr>
        <td>${escapeHtml(r.name || "—")}</td>
        <td class="muted">${escapeHtml(r.prefix || "")}…</td>
        <td>${pillFor(r.scope)}</td>
        <td class="muted">${escapeHtml(relTime(r.created_at))}</td>
        <td class="muted">${escapeHtml(r.last_used_at ? relTime(r.last_used_at) : "never")}</td>
        <td>${keyStatusPill(r)}</td>
        <td>${keyActionsHtml(r)}</td>
      </tr>`).join("");
  } catch (e) {
    $("tbl-keys").querySelector("tbody").innerHTML =
      `<tr><td colspan="7" class="empty">${escapeHtml(e.message)}</td></tr>`;
    $("keys-count").textContent = "err";
  }
}

async function createKey() {
  const name = $("key-name").value.trim();
  const scope = $("key-scope").value;
  const ttlDays = parseInt($("key-ttl").value, 10);
  if (!name) { showToast("name is required"); $("key-name").focus(); return; }
  try {
    const body = { name, scope };
    if (Number.isFinite(ttlDays) && ttlDays > 0) body.ttl_secs = ttlDays * 86400;
    const res = await mgmtCall("POST", "/api/v1/keys", body);
    $("new-key-token").textContent = res.token;
    $("new-key").hidden = false;
    $("key-name").value = "";
    $("key-ttl").value = "";
    showToast("key created — copy it now", true);
    loadKeys();
  } catch (e) {
    showToast("create failed: " + e.message);
  }
}

async function editKeyDialog(id, currentName, currentScope) {
  const body = `
    <label for="m-key-name">Name</label>
    <input id="m-key-name" type="text" value="${escapeHtml(currentName)}" />
    <label for="m-key-scope">Scope</label>
    <select id="m-key-scope">
      <option value="inference"${currentScope === "inference" ? " selected" : ""}>inference</option>
      <option value="admin"${currentScope === "admin" ? " selected" : ""}>admin</option>
    </select>`;
  const ok = await openModal({
    title: "Edit key",
    body,
    actions: [
      { label: "Cancel", value: false },
      { label: "Save", kind: "primary", value: true },
    ],
  });
  if (!ok) return;
  const newName  = ($("m-key-name").value  || "").trim();
  const newScope = ($("m-key-scope").value || "").trim();
  const update = {};
  if (newName  && newName  !== currentName)  update.name  = newName;
  if (newScope && newScope !== currentScope) update.scope = newScope;
  if (Object.keys(update).length === 0) return;
  try {
    await mgmtCall("PATCH", `/api/v1/keys/${encodeURIComponent(id)}`, update);
    showToast("key updated", true);
    loadKeys();
  } catch (e) { showToast("update failed: " + e.message); }
}

async function handleKeyAction(action, id, name, scope) {
  if (action === "key-revoke") {
    const ok = await confirmModal(
      "Revoke key?",
      "This key can no longer authenticate /v1/* — but stays in the audit log.",
      { confirmLabel: "Revoke", danger: true });
    if (!ok) return;
    try {
      await mgmtCall("POST", `/api/v1/keys/${encodeURIComponent(id)}/revoke`);
      showToast("key revoked", true); loadKeys();
    } catch (e) { showToast("revoke failed: " + e.message); }
  } else if (action === "key-delete") {
    const ok = await confirmModal(
      "Delete key permanently?",
      "Removes it entirely — no audit row is left behind. Prefer Revoke unless you really need to purge.",
      { confirmLabel: "Delete", danger: true });
    if (!ok) return;
    try {
      await mgmtCall("DELETE", `/api/v1/keys/${encodeURIComponent(id)}?purge=true`);
      showToast("key deleted", true); loadKeys();
    } catch (e) { showToast("delete failed: " + e.message); }
  } else if (action === "key-edit") {
    editKeyDialog(id, name || "", scope || "inference");
  }
}

// ============================================================
// settings
// ============================================================
async function loadSettings() {
  if (!adminKey()) { $("settings-meta").textContent = "enter admin key"; return; }
  try {
    const doc = await mgmtCall("GET", "/api/v1/settings");
    lastSettings = doc;
    const v = (k, fallback = "") => {
      const x = doc[k];
      if (x === null || x === undefined) return fallback;
      return typeof x === "string" ? x : String(x);
    };
    $("set-public-url").value    = v("public_base_url");
    $("set-default-model").value = v("default_model", "auto");
    $("set-rate-limit").value    = v("rate_limit_rpm", "60");
    $("set-max-tokens").value    = v("max_tokens_default", "4096");

    const hfTokenSet = !!v("huggingface_api_token").trim();
    $("set-hf-token").value = "";
    $("set-hf-token").placeholder = hfTokenSet
      ? "•••••• (set — leave blank to keep, type to replace)"
      : "hf_•••••••••••• (paste to enable)";

    const savedUrl = v("public_base_url").trim();
    if (!savedUrl && lastEffectiveUrl) {
      try {
        await mgmtCall("PUT", "/api/v1/settings", { public_base_url: lastEffectiveUrl });
        $("set-public-url").value = lastEffectiveUrl;
        $("settings-meta").textContent = `auto-detected: ${lastEffectiveUrl}`;
      } catch (_) {
        $("settings-meta").textContent = "loaded (using auto-detected URL)";
      }
    } else {
      $("settings-meta").textContent = "loaded";
    }
    renderEndpointSnippet(savedUrl || lastEffectiveUrl);
  } catch (e) {
    $("settings-meta").textContent = "error: " + e.message;
  }
}

async function saveSettings() {
  const body = {
    public_base_url:    $("set-public-url").value.trim(),
    default_model:      $("set-default-model").value.trim() || "auto",
    rate_limit_rpm:     parseInt($("set-rate-limit").value, 10) || 60,
    max_tokens_default: parseInt($("set-max-tokens").value, 10) || 4096,
  };
  // HF token: empty input means "keep existing" (we never echo it back).
  const hfToken = $("set-hf-token").value;
  if (hfToken.length > 0) body.huggingface_api_token = hfToken;
  try {
    await mgmtCall("PUT", "/api/v1/settings", body);
    showToast("settings saved", true);
    $("set-hf-token").value = "";
    loadSettings();
  } catch (e) {
    showToast("save failed: " + e.message);
  }
}

function renderEndpointSnippet(baseUrl) {
  const wrap = $("endpoint-snippet");
  if (!baseUrl) { wrap.hidden = true; return; }
  const bu = baseUrl.replace(/\/+$/, "");
  $("endpoint-snippet-content").textContent =
    `Base URL:  ${bu}/v1\nAPI key:   <one of the keys from the API keys tab>\n\n` +
    `# curl\ncurl -H "Authorization: Bearer mgc_..." ${bu}/v1/models`;
  wrap.hidden = false;
}

// ============================================================
// model registry
// ============================================================
async function loadModels() {
  if (!adminKey()) {
    $("tbl-models-reg").querySelector("tbody").innerHTML =
      '<tr><td colspan="8" class="empty">enter admin key in the sidebar to view…</td></tr>';
    $("models-reg-count").textContent = "—";
    return;
  }
  try {
    const rows = await mgmtCall("GET", "/api/v1/models");
    const filter = ($("filter-models-reg").value || "").toLowerCase().trim();
    const filtered = filter
      ? rows.filter((r) => ["id", "display_name", "status", "hf_repo"].some((k) =>
          String(r[k] ?? "").toLowerCase().includes(filter)))
      : rows;
    $("models-reg-count").textContent = rows.length;
    const tbody = $("tbl-models-reg").querySelector("tbody");
    if (filtered.length === 0) {
      tbody.innerHTML = `<tr><td colspan="8" class="empty">${rows.length === 0
        ? "no models registered — open the form above to add one"
        : "no matches"}</td></tr>`;
      return;
    }
    tbody.innerHTML = filtered.map((r) => {
      const hasHf = (r.hf_repo || "").length > 0 && (r.hf_file || "").length > 0;
      const hfCell = hasHf
        ? `<span class="muted small">${escapeHtml(r.hf_repo)}<br>${escapeHtml(r.hf_file)}</span>`
        : `<span class="pill muted">manual</span>`;
      const loadBtn = hasHf
        ? `<button class="row-action" data-action="model-load" data-id="${escapeHtml(r.id)}">Load…</button>`
        : "";
      return `
      <tr>
        <td class="id">${escapeHtml(r.id)}</td>
        <td>${escapeHtml(r.display_name || "—")}</td>
        <td>${hfCell}</td>
        <td>${pillFor(r.status)}</td>
        <td>${r.is_default ? '<span class="pill ok">default</span>' : '<span class="pill muted">—</span>'}</td>
        <td class="muted small">${escapeHtml(r.loaded_on_node || "—")}</td>
        <td class="muted">${escapeHtml(relTime(r.created_at))}</td>
        <td>
          <div class="row-actions">
            ${loadBtn}
            <button class="row-action" data-action="model-toggle"
                    data-id="${escapeHtml(r.id)}" data-status="${escapeHtml(r.status)}">
              ${r.status === "disabled" ? "Enable" : "Disable"}
            </button>
            ${r.is_default ? "" : `<button class="row-action" data-action="model-default" data-id="${escapeHtml(r.id)}">Set default</button>`}
            <button class="row-action danger" data-action="model-delete" data-id="${escapeHtml(r.id)}">Delete</button>
          </div>
        </td>
      </tr>`;
    }).join("");
  } catch (e) {
    $("tbl-models-reg").querySelector("tbody").innerHTML =
      `<tr><td colspan="8" class="empty">${escapeHtml(e.message)}</td></tr>`;
    $("models-reg-count").textContent = "err";
  }
}

async function createModel() {
  const id = $("model-id").value.trim();
  if (!id) { showToast("Model ID is required"); $("model-id").focus(); return; }
  const hfRepo = $("model-hf-repo").value.trim();
  const hfFile = $("model-hf-file").value.trim();
  if ((hfRepo && !hfFile) || (!hfRepo && hfFile)) {
    showToast("HF repo and HF file must be set together");
    return;
  }
  const body = {
    id,
    display_name: $("model-name").value.trim(),
    status:       $("model-status").value,
    is_default:   $("model-default").value === "true",
    hf_repo:      hfRepo,
    hf_file:      hfFile,
  };
  try {
    await mgmtCall("POST", "/api/v1/models", body);
    showToast("model added", true);
    $("model-id").value = ""; $("model-name").value = "";
    $("model-hf-repo").value = ""; $("model-hf-file").value = "";
    loadModels();
  } catch (e) {
    showToast("add failed: " + e.message);
  }
}

async function loadModelDialog(modelId) {
  // candidate workers from last /overview snapshot
  let candidates = [];
  try {
    const cn = lastData && lastData.coordinator && lastData.coordinator.nodes;
    if (Array.isArray(cn)) {
      candidates = cn.filter((n) => n.control_endpoint).map((n) => n.node_id || n.id);
    }
  } catch (_) { /* fall through */ }

  let body;
  if (candidates.length > 0) {
    const opts = candidates.map((c) => `<option value="${escapeHtml(c)}">${escapeHtml(c)}</option>`).join("");
    body = `
      <p>The selected worker will download the GGUF from HuggingFace and restart its <code>llama-server</code>.</p>
      <label for="m-load-node">Worker</label>
      <select id="m-load-node">${opts}</select>`;
  } else {
    body = `
      <p>No worker has registered a control endpoint yet. Paste a node ID to dispatch anyway.</p>
      <label for="m-load-node">Node ID</label>
      <input id="m-load-node" type="text" placeholder="node id" />`;
  }
  const ok = await openModal({
    title: `Load ${modelId}`,
    body,
    actions: [
      { label: "Cancel", value: false },
      { label: "Load", kind: "primary", value: true },
    ],
  });
  if (!ok) return;
  const nodeId = ($("m-load-node").value || "").trim();
  if (!nodeId) { showToast("node id required"); return; }
  try {
    await mgmtCall("POST",
      `/api/v1/models/${encodeURIComponent(modelId)}/load?node_id=${encodeURIComponent(nodeId)}`);
    showToast(`download dispatched to ${nodeId}`, true);
    loadModels();
  } catch (e) {
    showToast("load failed: " + e.message);
  }
}

async function handleModelAction(action, id, status) {
  if (action === "model-delete") {
    const ok = await confirmModal(
      "Delete model?",
      `Removes ${id} from /v1/models immediately.`,
      { confirmLabel: "Delete", danger: true });
    if (!ok) return;
    try { await mgmtCall("DELETE", `/api/v1/models/${encodeURIComponent(id)}`); showToast("deleted", true); loadModels(); }
    catch (e) { showToast("delete failed: " + e.message); }
  } else if (action === "model-toggle") {
    const next = status === "disabled" ? "available" : "disabled";
    try { await mgmtCall("PATCH", `/api/v1/models/${encodeURIComponent(id)}`, { status: next }); showToast(`set to ${next}`, true); loadModels(); }
    catch (e) { showToast("update failed: " + e.message); }
  } else if (action === "model-default") {
    try { await mgmtCall("PATCH", `/api/v1/models/${encodeURIComponent(id)}`, { is_default: true }); showToast("default set", true); loadModels(); }
    catch (e) { showToast("update failed: " + e.message); }
  } else if (action === "model-load") {
    loadModelDialog(id);
  }
}

// ============================================================
// inference log
// ============================================================
async function loadInferenceLog() {
  if (!adminKey()) {
    $("tbl-inflog").querySelector("tbody").innerHTML =
      '<tr><td colspan="8" class="empty">enter admin key in the sidebar to view…</td></tr>';
    $("inflog-count").textContent = "—";
    return;
  }
  const params = new URLSearchParams();
  const st = $("inflog-status").value;
  if (st) params.set("status", st);
  const node = $("inflog-node").value.trim();
  if (node) params.set("node_id", node);
  const model = $("inflog-model").value.trim();
  if (model) params.set("model", model);
  params.set("limit", "200");
  try {
    const rows = await mgmtCall("GET", "/api/v1/inference/recent?" + params.toString());
    $("inflog-count").textContent = rows.length;
    const tbody = $("tbl-inflog").querySelector("tbody");
    if (rows.length === 0) {
      tbody.innerHTML = `<tr><td colspan="8" class="empty">no requests match the current filter</td></tr>`;
      return;
    }
    tbody.innerHTML = rows.map((r) => {
      const ok = r.status_code >= 200 && r.status_code < 300;
      const statusPill = ok
        ? `<span class="pill ok">${r.status_code}</span>`
        : `<span class="pill err">${r.status_code}</span>`;
      const tokens = (r.prompt_tokens != null || r.completion_tokens != null || r.total_tokens != null)
        ? `${r.prompt_tokens ?? "?"} / ${r.completion_tokens ?? "?"} / ${r.total_tokens ?? "?"}`
        : "—";
      const latency = r.latency_ms != null ? `${r.latency_ms} ms` : "—";
      const errCell = r.error_type
        ? `<span class="pill warn">${escapeHtml(r.error_type)}</span>` +
          (r.error_message ? `<div class="muted small" style="margin-top: 4px; max-width: 280px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;" title="${escapeHtml(r.error_message)}">${escapeHtml(r.error_message)}</div>` : "")
        : "—";
      return `
      <tr>
        <td class="muted" title="${escapeHtml(r.created_at)}">${escapeHtml(relTime(r.created_at))}</td>
        <td>${statusPill}</td>
        <td class="id">${escapeHtml(r.model || "—")}</td>
        <td class="muted small" title="${escapeHtml(r.inference_url || "")}">${escapeHtml(r.node_id || "—")}</td>
        <td class="muted small">${escapeHtml(tokens)}</td>
        <td class="muted">${escapeHtml(latency)}</td>
        <td class="muted small">${escapeHtml(r.api_key_prefix || "—")}</td>
        <td>${errCell}</td>
      </tr>`;
    }).join("");
  } catch (e) {
    $("tbl-inflog").querySelector("tbody").innerHTML =
      `<tr><td colspan="8" class="empty">${escapeHtml(e.message)}</td></tr>`;
    $("inflog-count").textContent = "err";
  }
}

// ============================================================
// main load
// ============================================================
async function loadOverview() {
  const key = adminKey();
  const headers = key ? { "Authorization": "Bearer " + key } : {};

  try {
    const res = await fetch("/overview", { headers, cache: "no-store" });
    if (!res.ok) throw new Error("HTTP " + res.status);
    const data = await res.json();
    lastData = data;

    $("raw").textContent = JSON.stringify(data, null, 2);
    const ctx = renderKpis(data);
    renderServices(data.services || {});
    renderDonut(ctx.mgmtNodes.length > 0 ? ctx.mgmtNodes : ctx.liveNodes);
    renderTables(ctx);

    const ep = data.endpoint || {};
    lastEffectiveUrl = ep.effective_base_url || window.location.origin;
    $("set-public-url").placeholder = ep.derived_public_base_url || window.location.origin;
    renderEndpointSnippet(lastEffectiveUrl);

    // Quickstart needs the keys count too — kicked off from loadKeys() once it
    // resolves; here we pass the current sidebar badge as a best guess.
    const keyBadge = parseInt($("nav-count-keys").textContent, 10);
    renderQuickstart(ctx, Number.isFinite(keyBadge) ? keyBadge : 0);

    const now = new Date();
    $("last-updated").textContent = "updated " + now.toLocaleTimeString();
  } catch (e) {
    showToast("Failed to load /overview: " + e.message);
    $("last-updated").textContent = "fetch failed";
  }
}

function loadAll() {
  loadOverview();
  loadKeys();
  loadSettings();
  loadModels();
  loadInferenceLog();
}

// ============================================================
// router
// ============================================================
function navigate(route, { replace = false } = {}) {
  if (!ROUTES.includes(route)) route = "overview";

  document.querySelectorAll(".route").forEach((s) => {
    s.hidden = s.dataset.route !== route;
  });
  document.querySelectorAll(".nav a").forEach((a) => {
    a.classList.toggle("active", a.dataset.route === route);
  });

  const [title, meta] = ROUTE_TITLES[route];
  $("section-title").textContent = title;
  $("section-meta").textContent = meta;

  try { localStorage.setItem(ROUTE_STORAGE, route); } catch (_) { /* ignore */ }
  const target = "#" + route;
  if (location.hash !== target) {
    if (replace) history.replaceState(null, "", target);
    else         history.pushState(null,  "", target);
  }
}

function currentRoute() {
  const fromHash = (location.hash || "").replace(/^#/, "");
  if (ROUTES.includes(fromHash)) return fromHash;
  try {
    const stored = localStorage.getItem(ROUTE_STORAGE);
    if (stored && ROUTES.includes(stored)) return stored;
  } catch (_) { /* ignore */ }
  return "overview";
}

window.addEventListener("hashchange", () => navigate(currentRoute(), { replace: true }));

// ============================================================
// wiring
// ============================================================
// Restore admin key BEFORE first fetch so reloads stay authenticated.
try {
  const stored = localStorage.getItem(KEY_STORAGE);
  if (stored) $("adminKey").value = stored;
} catch (_) { /* ignore */ }

function refreshAuthHint() {
  const hint = $("auth-hint");
  if (adminKey()) {
    hint.textContent = "key saved — admin tabs unlocked";
    hint.classList.add("ok");
  } else {
    hint.textContent = "no key saved — admin tabs will stay empty";
    hint.classList.remove("ok");
  }
}
refreshAuthHint();

$("adminKey").addEventListener("input", () => {
  const v = $("adminKey").value;
  try {
    if (v) localStorage.setItem(KEY_STORAGE, v);
    else   localStorage.removeItem(KEY_STORAGE);
  } catch (_) { /* ignore */ }
  refreshAuthHint();
});

$("auth-form").addEventListener("submit", (e) => {
  e.preventDefault();
  const v = $("adminKey").value;
  try { if (v) localStorage.setItem(KEY_STORAGE, v); } catch (_) {}
  refreshAuthHint();
  loadAll();
});

$("refresh").addEventListener("click", loadAll);

document.querySelectorAll(".nav a").forEach((a) => {
  a.addEventListener("click", (e) => {
    e.preventDefault();
    navigate(a.dataset.route);
  });
});

// table filters — re-render from cached data, no extra fetch
$("filter-coord").addEventListener("input", () => lastData && renderTables(renderKpis(lastData)));
$("filter-mgmt").addEventListener("input",  () => lastData && renderTables(renderKpis(lastData)));
$("filter-keys").addEventListener("input",  loadKeys);
$("filter-models-reg").addEventListener("input", loadModels);

$("inflog-status").addEventListener("change", loadInferenceLog);
$("inflog-node").addEventListener("input",   loadInferenceLog);
$("inflog-model").addEventListener("input",  loadInferenceLog);

$("settings-form").addEventListener("submit", (e) => { e.preventDefault(); saveSettings(); });
$("model-form").addEventListener("submit",    (e) => { e.preventDefault(); createModel(); });
$("key-form").addEventListener("submit",      (e) => { e.preventDefault(); createKey(); });

$("new-key-copy").addEventListener("click", () => {
  navigator.clipboard.writeText($("new-key-token").textContent || "")
    .then(() => showToast("copied", true))
    .catch(() => showToast("copy failed — select manually"));
});
$("new-key-dismiss").addEventListener("click", () => {
  $("new-key").hidden = true;
  $("new-key-token").textContent = "";
});

// modal close button + Escape (dialog handles Esc natively, but we still
// need to resolve the promise so callers see a "cancelled" result).
$("modal-close").addEventListener("click", () => closeModal(false));
$("modal").addEventListener("close", () => {
  if (modalResolver) {
    const r = modalResolver;
    modalResolver = null;
    r(false);
  }
});

// generic delegated row-action handler
document.addEventListener("click", (e) => {
  const btn = e.target.closest("[data-action]");
  if (!btn || !btn.dataset.id) return;
  const a = btn.dataset.action;
  if (["approve", "revoke", "drain"].includes(a))
    return handleNodeAction(a, btn.dataset.id);
  if (a === "key-revoke" || a === "key-delete" || a === "key-edit")
    return handleKeyAction(a, btn.dataset.id, btn.dataset.name, btn.dataset.scope);
  if (a === "model-delete" || a === "model-toggle" || a === "model-default" || a === "model-load")
    return handleModelAction(a, btn.dataset.id, btn.dataset.status);
});

// keyboard: R to refresh anywhere outside an input
document.addEventListener("keydown", (e) => {
  if (e.key === "r" && !e.metaKey && !e.ctrlKey && document.activeElement.tagName !== "INPUT") {
    e.preventDefault(); loadAll();
  }
});

// boot
navigate(currentRoute(), { replace: true });
loadAll();
setInterval(loadAll, REFRESH_MS);
