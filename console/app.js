// Banc d'essai tonAPI. Page statique : tout passe par l'API configurée, rien d'autre n'est contacté.
"use strict";

const FOURNISSEURS = window.TONAPI_FOURNISSEURS;
const ESSAIS = window.TONAPI_ESSAIS;
const SERVICES = window.TONAPI_SERVICES;
const ERREURS = window.TONAPI_ERREURS;

// ---------------------------------------------------------------------------------------------
// Outils
// ---------------------------------------------------------------------------------------------

const $ = (id) => document.getElementById(id);
function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === null || value === undefined || value === false) continue;
    if (key === "class") node.className = value;
    else if (key === "text") node.textContent = value;
    else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else node.setAttribute(key, value === true ? "" : value);
  }
  for (const child of children.flat(Infinity)) {
    if (child === null || child === undefined || child === false) continue;
    node.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
}

/// Chevron des lignes repliables (indique qu'une ligne s'ouvre).
function chevron() {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 16 16");
  svg.setAttribute("class", "icon chev");
  svg.setAttribute("aria-hidden", "true");
  const path = document.createElementNS(ns, "path");
  path.setAttribute("d", "M6 4l4 4-4 4");
  svg.append(path);
  return svg;
}

/// Stockage local : la page reste utilisable s'il est bloqué (navigation privée…).
const store = {
  get(key, fallback) {
    try { const raw = localStorage.getItem(key); return raw === null ? fallback : JSON.parse(raw); } catch { return fallback; }
  },
  set(key, value) {
    try { localStorage.setItem(key, JSON.stringify(value)); return true; } catch { return false; }
  },
};

const iso = (d) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
function mondayOf(date) {
  const d = new Date(date);
  d.setHours(12, 0, 0, 0);
  d.setDate(d.getDate() - ((d.getDay() + 6) % 7));
  return d;
}
function addDays(date, n) { const d = new Date(date); d.setDate(d.getDate() + n); return d; }
const when = (value, tz, withTime = true) => {
  if (!value) return "";
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return String(value);
  return d.toLocaleString("fr-FR", withTime
    ? { timeZone: tz, weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }
    : { timeZone: tz, day: "numeric", month: "short" });
};
const relative = (ms) => {
  const days = Math.round((ms - Date.now()) / 86400000);
  if (days === 0) return "aujourd'hui";
  return days > 0 ? `dans ${days} j` : `il y a ${-days} j`;
};

function pageHead(title, text, actions) {
  return el("header", { class: "page-head" }, el("div", {}, el("h1", { text: title }), text ? el("p", { text }) : null), actions || null);
}
function panel({ title, sub, actions, body, cls }) {
  return el("section", { class: `panel${cls ? ` ${cls}` : ""}` },
    title ? el("div", { class: "panel-head" }, el("div", {}, el("h2", { text: title }), sub ? el("p", { text: sub }) : null), actions || null) : null,
    body);
}
const pill = (tone, text) => el("span", { class: `pill ${tone}`, text });
const empty = (title, text) => el("p", { class: "empty" }, el("b", { text: title }), text);

// ---------------------------------------------------------------------------------------------
// État
// ---------------------------------------------------------------------------------------------

const state = {
  api: store.get("tonapi.api", "http://127.0.0.1:8080"),
  health: null,
  universities: [],
  details: {},
  openapi: null,
  token: null,
  accountId: null,
  pending: null,
  me: null,
  results: {},
  running: false,
  week: mondayOf(new Date()),
  journal: [],
  form: { university: "", establishment: "", username: "", remember: true },
};

function apiBase() { return state.api.replace(/\/+$/, ""); }
function apiIsSafe() {
  try {
    const url = new URL(apiBase());
    const local = ["127.0.0.1", "localhost", "[::1]"].includes(url.hostname);
    return url.protocol === "https:" || (url.protocol === "http:" && local);
  } catch { return false; }
}

// ---------------------------------------------------------------------------------------------
// Comptes mémorisés : jamais de mot de passe, seulement le jeton chiffré rendu par l'API.
// ---------------------------------------------------------------------------------------------

const ACCOUNTS = "tonapi.comptes";
function accounts() { return store.get(ACCOUNTS, []); }
function saveAccounts(list) { store.set(ACCOUNTS, list); renderRail(); }
function accountKey(university, username) { return `${apiBase()}|${university}|${username.toLowerCase()}`; }
function findAccount(university, username) { return accounts().find((a) => a.id === accountKey(university, username)); }
function upsertAccount(patch) {
  const list = accounts();
  const index = list.findIndex((a) => a.id === patch.id);
  if (index >= 0) list[index] = { ...list[index], ...patch };
  else list.push({ label: "", ...patch });
  saveAccounts(list);
}
function forgetAccount(id) { saveAccounts(accounts().filter((a) => a.id !== id)); if (state.accountId === id) state.accountId = null; if (store.get(ACTIVE, null) === id) store.set(ACTIVE, null); }

/// Compte dont la session est ouverte : reprise au rechargement tant que son jeton est valable.
const ACTIVE = "tonapi.session";
function usable(a) { return Boolean(a?.token) && a.api === apiBase() && (!a.expiresAt || Date.parse(a.expiresAt) > Date.now()); }
function resumeAccount(a, run = true) {
  if (!usable(a) || state.running) return;
  state.accountId = a.id;
  state.form = { ...state.form, university: a.university, username: a.username, establishment: a.establishment || "", remember: true };
  state.token = a.token;
  state.pending = null;
  store.set(ACTIVE, a.id);
  upsertAccount({ id: a.id, lastUsed: Date.now() });
  route();
  if (run) runAll();
}
function resumeOnLoad() {
  const id = store.get(ACTIVE, null);
  if (!id) return;
  const a = accounts().find((x) => x.id === id);
  // Les tests ne repartent seuls que sur le banc d'essai : les autres pages n'appellent pas l'université.
  if (usable(a)) resumeAccount(a, !location.hash.replace(/^#\/?/, "").split("?")[0]);
  else store.set(ACTIVE, null);
}
function endSession() {
  state.token = null;
  store.set(ACTIVE, null);
  renderRail();
}

/// Anciennes clés de la première console (un jeton par université, sans identifiant).
(function migrate() {
  try {
    const list = accounts();
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      const m = key && key.match(/^tonapi\.appareil\.(.+)\.([a-z0-9-]+)$/);
      if (!m) continue;
      const id = `${m[1]}|${m[2]}|?`;
      if (!list.some((a) => a.id === id)) {
        list.push({ id, api: m[1], university: m[2], username: "", label: "Appareil reconnu (ancienne console)", token: localStorage.getItem(key), lastUsed: Date.now() });
      }
    }
    store.set(ACCOUNTS, list);
  } catch { /* stockage indisponible */ }
})();

function setToken(token, extra = {}) {
  state.token = token;
  if (!token || !state.accountId) return;
  const account = accounts().find((a) => a.id === state.accountId);
  if (account && account.remember !== false) upsertAccount({ id: state.accountId, token, ...extra });
}

// ---------------------------------------------------------------------------------------------
// Appels à l'API, tous journalisés (sans corps : le journal peut contenir des données de compte).
// ---------------------------------------------------------------------------------------------

async function call(method, path, { body, auth = true } = {}) {
  const headers = { Accept: "application/json" };
  if (body !== undefined) headers["Content-Type"] = "application/json";
  const bearer = auth ? state.token : null;
  if (bearer) headers.Authorization = `Bearer ${bearer}`;
  const started = performance.now();
  const entry = { at: new Date(), method, path: path.replace(/\?.*$/, ""), query: path.includes("?") ? path.slice(path.indexOf("?")) : "", status: 0 };
  let response;
  try {
    response = await fetch(apiBase() + path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body), credentials: "omit", cache: "no-store" });
  } catch {
    entry.ms = Math.round(performance.now() - started);
    entry.code = "network";
    log(entry);
    return { status: 0, ms: entry.ms, data: null, raw: null, problem: { code: "network", title: "API injoignable", detail: "Vérifier l'adresse et que tonapi-server tourne." }, headers: new Headers() };
  }
  entry.ms = Math.round(performance.now() - started);
  entry.status = response.status;
  const renewed = response.headers.get("TonAPI-Session");
  if (renewed && bearer && bearer === state.token) { setToken(renewed); entry.renewed = true; }
  const text = await response.text();
  entry.size = text.length;
  let json = null;
  try { json = text ? JSON.parse(text) : null; } catch { json = null; }
  const isProblem = (response.headers.get("Content-Type") || "").includes("problem+json") || !response.ok;
  if (isProblem) entry.code = json?.code || `http_${response.status}`;
  log(entry);
  return { status: response.status, ms: entry.ms, data: isProblem ? null : json, raw: json ?? text, problem: isProblem ? (json || { code: `http_${response.status}`, title: `HTTP ${response.status}` }) : null, headers: response.headers };
}

function log(entry) {
  state.journal.unshift(entry);
  state.journal.length = Math.min(state.journal.length, 200);
  const list = $("journal");
  if (list) { list.prepend(journalItem(entry)); $("journal-empty")?.remove(); }
}

function journalItem(e) {
  const cls = `s${String(e.status)[0] || 0}`;
  return el("li", {},
    el("details", {},
      el("summary", {},
        el("span", { class: cls, text: e.status || "réseau" }),
        el("span", { class: "t", text: e.at.toLocaleTimeString("fr-FR") }),
        el("span", { class: "m", text: e.method }),
        el("span", { class: "path", text: `${e.path}${e.renewed ? "  · jeton renouvelé" : ""}${e.code ? `  · ${e.code}` : ""}` }),
        el("span", { class: "ms", text: `${e.ms} ms` })),
      el("pre", { class: "code", text: JSON.stringify({ requête: `${e.method} ${e.path}${e.query}`, statut: e.status, code: e.code ?? null, durée_ms: e.ms, octets: e.size ?? null, jeton_renouvelé: !!e.renewed }, null, 2) })));
}

// ---------------------------------------------------------------------------------------------
// Rail
// ---------------------------------------------------------------------------------------------

async function checkHealth() {
  const r = await call("GET", "/healthz", { auth: false });
  state.health = r.data ? { ok: true, universities: r.data.universities } : { ok: false, title: r.problem?.title };
  if (r.data) {
    const list = await call("GET", "/v1/universities", { auth: false });
    state.universities = Array.isArray(list.data) ? list.data : [];
  }
  renderRail();
  route();
}

function renderRail() {
  const h = $("health");
  if (h) {
    const ok = state.health?.ok;
    h.replaceChildren(el("span", { class: `dot ${ok ? "ok" : state.health ? "bad" : ""}` }),
      ok ? `En ligne · ${state.health.universities} université(s)` : state.health ? state.health.title || "API injoignable" : "Vérification…");
  }
  const chip = $("active-account");
  if (chip) {
    const a = accounts().find((x) => x.id === state.accountId);
    chip.replaceChildren(state.token
      ? el("div", { class: "account-chip" }, el("b", { text: a?.label || a?.username || "Session active" }), el("span", { text: a ? a.university : "non mémorisée" }))
      : el("div", { class: "account-chip" }, el("b", { text: "Aucune session" }), el("span", { text: `${accounts().length} compte(s) mémorisé(s)` })));
  }
  const count = $("count-accounts");
  if (count) count.textContent = String(accounts().length);
}

// ---------------------------------------------------------------------------------------------
// Fournisseurs : parcours et libellés
// ---------------------------------------------------------------------------------------------

function fournisseur(id) { return FOURNISSEURS.find((f) => f.id === id); }
function serviceLabel(key) {
  if (key === "timetable-groups") return "groupes";
  return (SERVICES.find((s) => s.key === key)?.label || key).toLowerCase();
}
/// Étapes d'un fournisseur, de l'entrée aux données ; le séparateur indique le sens de l'appel.
function routeOf(providerId) {
  const f = fournisseur(providerId);
  if (!f) return null;
  const parts = [];
  f.ligne.forEach((step, i) => { if (i) parts.push(el("span", { class: "sep", "aria-hidden": "true", text: "›" })); parts.push(el("span", { class: "step", text: step })); });
  return el("div", { class: "route", "aria-label": `Parcours : ${f.ligne.join(", puis ")}` }, parts);
}
/// `portal` désigne le portail de l'université : son logiciel est dans le détail (`portal`).
function providerFor(status, detail) {
  if (status.provider === "portal" && detail?.portal === "tiles-portal") return "tiles-portal";
  return status.provider;
}

// ---------------------------------------------------------------------------------------------
// Page : banc d'essai
// ---------------------------------------------------------------------------------------------

const PROBLEMS = {
  service_absent: ["idle", "Non proposé"],
  not_implemented: ["warn", "Non pris en charge"],
  service_login_required: ["warn", "Connexion au service"],
  session_expired: ["bad", "Session expirée"],
  invalid_token: ["bad", "Jeton refusé"],
  invalid_credentials: ["bad", "Identifiants refusés"],
  account_action_required: ["bad", "Action sur le compte"],
  service_unavailable: ["bad", "Injoignable"],
  upstream_changed: ["bad", "Réponse inattendue"],
  rate_limited: ["bad", "Trop d'essais"],
  network: ["bad", "API injoignable"],
};

const PROBES = [
  { key: "profile", label: "Profil" },
  { key: "timetable", label: "Emploi du temps", path: () => `/v1/timetable?from=${iso(state.week)}&to=${iso(addDays(state.week, 6))}` },
  { key: "timetable-groups", label: "Groupes", config: "timetable", path: () => "/v1/timetable/groups" },
  { key: "grades", label: "Notes", path: () => "/v1/grades", separate: "grades" },
  { key: "absences", label: "Absences", path: () => "/v1/absences", separate: "absences" },
  { key: "exams", label: "Examens", path: () => "/v1/exams" },
  { key: "homework", label: "Devoirs", path: () => "/v1/homework", separate: "homework" },
  { key: "mail", label: "Mail", path: () => "/v1/mail/inbox" },
  { key: "apps", label: "Applications", path: () => "/v1/apps" },
  { key: "documents", label: "Documents", path: () => "/v1/documents" },
];

function tz() { return state.me?.timezone || "Europe/Paris"; }
const grade = (g) => (g ? `${g.value}/${g.outOf}` : "non publiée");
function summarize(key, data) {
  switch (key) {
    case "profile": return { line: data.displayName || data.username, items: [data.email, data.studentNumber && `n° ${data.studentNumber}`, data.program].filter(Boolean) };
    case "timetable": return { line: `${data.length} séance(s) du ${when(state.week, tz(), false)} au ${when(addDays(state.week, 6), tz(), false)}`, items: data.slice(0, 8).map((c) => `${when(c.start, tz())} · ${c.title}${c.rooms?.length ? ` · ${c.rooms.join(", ")}` : ""}`) };
    case "timetable-groups": { const s = data.filter((g) => g.selected); return { line: `${data.length} groupe(s), ${s.length} sélectionné(s)`, items: s.slice(0, 6).map((g) => g.path?.length ? g.path.join(" › ") : g.name) }; }
    case "grades": { const ss = data.semesters || []; return { line: ss.length ? `${ss.length} semestre(s)` : "Aucune note publiée", items: ss.map((s) => `${s.name} · moyenne ${grade(s.average)} · ${s.modules?.length || 0} module(s)`) }; }
    case "absences": { const t = data.totals || {}; return { line: `${(data.absences || []).length} absence(s), ${t.unjustified ?? 0} non justifiée(s)`, items: (data.absences || []).slice(0, 5).map((a) => `${when(a.start, tz())}${a.module ? ` · ${a.module}` : ""}`) }; }
    case "homework": return { line: `${data.length} devoir(s)`, items: [...data].sort((a, b) => String(a.due).localeCompare(String(b.due))).slice(0, 6).map((h) => `${when(h.due, tz())} · ${h.title}`) };
    case "mail": return { line: `${(data.messages || []).length} message(s), ${data.unread ?? 0} non lu(s)`, items: (data.messages || []).slice(0, 5).map((m) => `${when(m.date, tz())} · ${m.from?.name || m.from?.address || "?"} · ${m.subject || "(sans objet)"}`) };
    case "apps": return { line: `${data.length} application(s)`, items: data.slice(0, 8).map((a) => a.title) };
    case "exams": return { line: data.length ? `${data.length} épreuve(s)` : "Aucune épreuve publiée", items: data.slice(0, 6).map((e) => `${e.start ? when(e.start, tz()) : "date à venir"} · ${e.title}${e.room ? ` · ${e.room}` : ""}`) };
    case "documents": return { line: `${data.length} document(s), ${data.filter((d) => d.downloadable).length} téléchargeable(s)`, items: data.slice(0, 6).map((d) => `${d.title}${d.year ? ` · ${d.year}` : ""}${d.downloadable ? "" : " · lien"}`) };
    default: return { line: "Réponse reçue", items: [] };
  }
}

function availability(key) {
  const configKey = PROBES.find((p) => p.key === key)?.config || key;
  return (state.me?.services || []).find((s) => s.service === configKey);
}
function providerOf(key) {
  const result = state.results[key];
  if (result?.provider && result.provider !== "session") return result.provider;
  const a = availability(key);
  if (a?.availability !== "api") return a?.availability || null;
  return providerFor(a, state.details[state.me?.university?.id]);
}
function outcome(key) {
  const r = state.results[key];
  if (!r) return { tone: "idle", text: "En attente" };
  if (r.running) return { tone: "run", text: "En cours" };
  if (r.skipped) return { tone: "idle", text: r.skipped };
  if (r.data !== undefined && r.data !== null) return { tone: "ok", text: "OK" };
  const [tone, text] = PROBLEMS[r.problem?.code] || ["bad", r.problem?.title || "Erreur"];
  return { tone, text };
}

function pageBench() {
  const blocks = [];
  if (!state.token) blocks.push(el("div", { class: "split" }, loginPanel(), savedPanel()));
  if (state.pending) blocks.push(mfaPanel());
  if (state.token) blocks.push(sessionPanel(), resultsPanel());
  blocks.push(journalPanel());
  return [
    pageHead("Banc d'essai", "Connecte un compte étudiant et interroge chaque service à travers tonAPI : résultat, fournisseur, parcours et durée."),
    el("div", { class: "stack" }, blocks),
  ];
}

function loginPanel() {
  const f = state.form;
  const uni = state.universities.find((u) => u.id === f.university);
  const status = el("div", { "aria-live": "polite" });
  if (state.health && !state.health.ok) status.append(el("p", { class: "notice bad", text: "API injoignable : vérifie l'adresse dans le menu et que tonapi-server tourne." }));
  else if (!apiIsSafe()) status.append(el("p", { class: "notice bad", text: "Adresse d'API refusée : les identifiants ne partent qu'en https, ou en http vers cette machine." }));
  const account = f.username && f.university ? findAccount(f.university, f.username) : null;

  const form = el("form", { autocomplete: "on", class: "panel-body", onsubmit: (e) => { e.preventDefault(); login(form, status); } },
    el("div", { class: "grid" },
      el("div", {},
        el("label", { for: "f-university", text: "Université" }),
        el("select", { id: "f-university", required: true, onchange: (e) => { state.form.university = e.target.value; state.form.establishment = ""; route(); } },
          el("option", { value: "", text: state.universities.length ? "Choisir une université" : "Aucune université disponible" }),
          state.universities.map((u) => el("option", { value: u.id, selected: u.id === f.university, text: `${u.name}${u.status !== "active" ? ` (${u.status})` : ""}` }))),
        uni ? el("p", { class: "hint", text: `${uni.login?.kind === "cas" ? "CAS" : uni.login?.kind}${uni.login?.mfa ? " · second facteur possible" : ""} · ${uni.id}` }) : null),
      el("div", {},
        el("label", { for: "f-establishment", text: "Composante" }),
        el("select", { id: "f-establishment", onchange: (e) => { state.form.establishment = e.target.value; } },
          el("option", { value: "", text: "Détection automatique" }),
          (uni?.establishments || []).map((e) => el("option", { value: e.id, selected: e.id === f.establishment, text: `${e.name}${e.default ? " (par défaut)" : ""}` })))),
      el("div", {},
        el("label", { for: "f-username", text: "Identifiant" }),
        el("input", { id: "f-username", name: "username", autocomplete: "username", required: true, spellcheck: "false", value: f.username, oninput: (e) => { state.form.username = e.target.value.trim(); } })),
      el("div", {},
        el("label", { for: "f-password", text: "Mot de passe" }),
        el("input", { id: "f-password", name: "password", type: "password", autocomplete: "current-password", required: true }))),
    el("label", { class: "check", style: "margin-top:16px" },
      el("input", { type: "checkbox", id: "f-remember", checked: f.remember, onchange: (e) => { state.form.remember = e.target.checked; } }),
      el("span", {}, "Mémoriser ce compte sur ce navigateur. ", el("a", { href: "#/comptes", text: "Comment ça marche" }))),
    account?.token ? el("p", { class: "notice info", text: "Appareil déjà reconnu pour ce compte : l'université ne devrait pas redemander le second facteur." }) : null,
    el("div", { class: "row", style: "margin-top:16px" },
      el("button", { class: "primary", type: "submit", disabled: !apiIsSafe() || !state.universities.length, text: "Se connecter et tout tester" })),
    el("p", { class: "hint", text: "Le mot de passe part vers l'API puis le champ est vidé. Il n'est jamais mémorisé." }),
    status);
  return panel({ title: "Connexion", sub: "Compte étudiant de l'université choisie", body: form });
}

function savedPanel() {
  const saved = accounts().filter((a) => a.api === apiBase());
  const body = saved.length
    ? el("div", { class: "panel-body accounts-list" }, saved.map((a) =>
      el("button", { type: "button", class: "account-option", "aria-pressed": String(state.accountId === a.id), onclick: () => { state.form = { ...state.form, university: a.university, username: a.username, establishment: a.establishment || "" }; state.accountId = a.id; route(); $("f-password")?.focus(); } },
        el("span", {}, el("b", { text: a.label || a.username || "Compte" }), el("small", { text: `${a.university}${a.username ? ` · ${a.username}` : ""}` })),
        a.token ? pill("ok", "appareil reconnu") : pill("idle", "sans jeton"))).concat(
      saved.filter(usable).length
        ? [el("p", { class: "hint", text: "Session encore ouverte : la reprendre sans mot de passe." }),
          saved.filter(usable).map((a) => el("button", { type: "button", onclick: () => resumeAccount(a), text: `Reprendre ${a.username || a.university}` }))]
        : []))
    : empty("Aucun compte mémorisé", "Coche « Mémoriser ce compte » à la connexion pour le retrouver ici.");
  return panel({ title: "Comptes mémorisés", sub: "Un clic remplit le formulaire", actions: el("a", { href: "#/comptes", text: "Gérer" }), body });
}

async function login(form, status) {
  const f = state.form;
  const password = form.querySelector("#f-password");
  const body = { university: f.university, username: f.username, password: password.value };
  if (f.establishment) body.establishment = f.establishment;
  password.value = "";
  const id = accountKey(f.university, f.username);
  const known = accounts().find((a) => a.id === id);
  if (f.remember && known?.token) body.previousToken = known.token;
  state.accountId = f.remember ? id : null;
  if (!f.remember && known) forgetAccount(id);
  status.replaceChildren(el("p", { class: "notice warn", text: "Connexion à l'université…" }));
  const r = await call("POST", "/v1/auth/login", { body, auth: false });
  if (r.status === 200 && r.data?.token) return signedIn(r.data);
  if (r.status === 202 && r.data?.pending) { state.pending = r.data; return route(); }
  status.replaceChildren(problemNotice(r.problem));
}

function signedIn(data) {
  state.pending = null;
  state.token = data.token;
  store.set(ACTIVE, state.accountId || null);
  if (state.accountId) {
    const previous = accounts().find((a) => a.id === state.accountId);
    upsertAccount({ id: state.accountId, api: apiBase(), university: state.form.university, username: state.form.username, establishment: state.form.establishment, token: data.token, expiresAt: data.expiresAt, lastUsed: Date.now(), deviceSince: previous?.deviceSince || Date.now(), remember: true });
  }
  renderRail();
  route();
  runAll();
}

function mfaPanel() {
  const p = state.pending;
  const status = el("div", { "aria-live": "polite" });
  const code = el("input", { id: "mfa-code", inputmode: "numeric", autocomplete: "one-time-code", maxlength: "8" });
  const remember = el("input", { type: "checkbox", checked: true });
  const send = async (method) => {
    const body = { pending: p.pending, method, rememberDevice: remember.checked };
    if (method === "code") { body.code = code.value.trim(); if (!body.code) return; }
    status.replaceChildren(el("p", { class: "notice warn", text: method === "push" ? "En attente de la validation sur le téléphone (100 s au plus)…" : "Vérification du code…" }));
    const r = await call("POST", "/v1/auth/mfa", { body, auth: false });
    code.value = "";
    if (r.status === 200 && r.data?.token) {
      if (state.accountId && remember.checked) upsertAccount({ id: state.accountId, deviceSince: Date.now() });
      return signedIn(r.data);
    }
    if (r.status === 202 && r.data?.pending) { state.pending = r.data; route(); return; }
    status.replaceChildren(problemNotice(r.problem));
  };
  return panel({
    title: "Second facteur", sub: p.message || "L'université demande une validation supplémentaire",
    body: el("div", { class: "panel-body" },
      p.methods?.push ? el("div", { class: "row" },
        el("button", { class: "primary", type: "button", onclick: () => send("push"), text: "Valider sur mon téléphone" }),
        el("span", { class: "hint", style: "margin:0", text: `Appareil : ${p.methods.push}` })) : null,
      p.methods?.code ? el("form", { class: "row", style: "margin-top:14px; align-items:flex-end", onsubmit: (e) => { e.preventDefault(); send("code"); } },
        el("div", { style: "flex:0 1 200px" }, el("label", { for: "mfa-code", text: "Code à 6 chiffres" }), code),
        el("button", { type: "submit", text: "Envoyer le code" })) : null,
      el("label", { class: "check", style: "margin-top:14px" }, remember, el("span", { text: "Faire mémoriser cet appareil par l'université (30 jours sans second facteur)" })),
      status),
  });
}

function sessionPanel() {
  const me = state.me;
  const est = me?.establishment;
  const establishments = state.universities.find((u) => u.id === me?.university?.id)?.establishments || [];
  const switcher = el("select", { id: "switch-est", disabled: !establishments.length, onchange: async (e) => { const r = await call("PUT", "/v1/me/establishment", { body: { establishment: e.target.value || null } }); if (r.data) runAll(); } },
    el("option", { value: "", text: "Détection automatique" }),
    establishments.map((e) => el("option", { value: e.id, selected: est?.source === "student" && est.id === e.id, text: e.name })));
  const week = el("div", { class: "week" },
    el("button", { type: "button", class: "ghost", "aria-label": "Semaine précédente", title: "Semaine précédente", onclick: () => { state.week = addDays(state.week, -7); rerunTimetable(); }, text: "‹" }),
    el("input", { type: "date", "aria-label": "Semaine de l'emploi du temps", value: iso(state.week), onchange: (e) => { if (e.target.value) { state.week = mondayOf(new Date(e.target.value)); rerunTimetable(); } } }),
    el("button", { type: "button", class: "ghost", "aria-label": "Semaine suivante", title: "Semaine suivante", onclick: () => { state.week = addDays(state.week, 7); rerunTimetable(); }, text: "›" }));
  return panel({
    title: me ? `${me.profile.displayName || me.profile.username}` : "Session active",
    sub: me ? `${me.university.name}${est ? ` · ${est.name} (${est.source === "detected" ? "détectée" : est.source === "student" ? "choisie" : "par défaut"}${est.confidence ? `, ${Math.round(est.confidence * 100)} %` : ""})` : ""}` : "",
    actions: el("div", { class: "row" },
      el("button", { type: "button", class: "primary", onclick: runAll, disabled: state.running, text: state.running ? "Tests en cours…" : "Relancer les tests" }),
      el("button", { type: "button", onclick: refresh, text: "Prolonger la session" }),
      el("button", { type: "button", class: "ghost danger", onclick: logout, text: "Se déconnecter" })),
    body: el("div", { class: "panel-body row", style: "align-items:flex-end; gap:16px" },
      el("div", {}, el("label", { text: "Semaine de l'emploi du temps" }), week),
      el("div", { style: "flex:0 1 280px" }, el("label", { for: "switch-est", text: "Composante" }), switcher)),
  });
}

function resultsPanel() {
  const counts = {};
  for (const probe of PROBES) { const o = outcome(probe.key); counts[o.tone] = (counts[o.tone] || 0) + 1; }
  const stats = el("div", { class: "stats" },
    counts.ok ? pill("ok", `${counts.ok} OK`) : null,
    counts.warn ? pill("warn", `${counts.warn} à voir`) : null,
    counts.bad ? pill("bad", `${counts.bad} en échec`) : null,
    counts.idle ? pill("idle", `${counts.idle} sans données`) : null,
    counts.run ? pill("run", `${counts.run} en cours`) : null,
    state.overviewMs && !state.running ? el("span", { class: "hint", text: `Tous les services en un appel groupé : ${state.overviewMs} ms` }) : null);

  const rows = PROBES.map((probe) => {
    const result = state.results[probe.key];
    const provider = providerOf(probe.key);
    const o = outcome(probe.key);
    let line = "";
    const detail = [];
    const route = provider ? routeOf(provider) : null;
    if (route) detail.push(route);
    if (result?.data !== undefined && result?.data !== null) {
      const s = summarize(probe.key, result.data);
      line = s.line;
      if (s.items.length) detail.push(el("ul", {}, s.items.map((i) => el("li", { text: i }))));
      detail.push(rawDetails(result.data));
    } else if (result?.problem) {
      line = result.problem.detail || result.problem.title || "";
      detail.push(el("p", { style: "margin:0; color:var(--text-2)" }, result.problem.title || "", result.problem.code ? [" · ", el("code", { text: result.problem.code }), " · ", el("a", { href: `#/erreurs?code=${result.problem.code}`, text: "que faire" })] : null));
      if (result.problem.link) detail.push(el("a", { href: result.problem.link, target: "_blank", rel: "noopener noreferrer", text: "Ouvrir le service de l'université" }));
      if (result.problem.code === "service_login_required" && probe.separate) detail.push(serviceLoginForm(probe));
    }
    const open = result?.problem?.code === "service_login_required";
    return el("details", { class: "service", open },
      el("summary", {},
        el("span", { class: "name", text: probe.label }),
        el("span", { class: "provider", text: provider || "aucun fournisseur" }),
        el("span", { class: "line" }, pill(o.tone, o.text), " ", line),
        el("span", { class: "time", text: result?.ms !== undefined ? `${result.ms} ms` : "" }),
        chevron()),
      el("div", { class: "detail" }, detail.length ? detail : el("p", { class: "hint", style: "margin:0", text: "Rien à afficher pour ce service." }),
        provider && fournisseur(provider) ? el("a", { href: `#/fournisseurs?id=${encodeURIComponent(provider)}`, text: `Fiche du fournisseur ${provider}` }) : null));
  });

  return el("div", { class: "stack" },
    panel({ title: "Services", sub: `Semaine du ${when(state.week, tz(), false)} pour l'emploi du temps`, actions: stats, body: el("div", { class: "services" }, rows) }),
    panel({
      title: "Rapport anonyme", sub: "Statuts, fournisseurs et durées seulement, à joindre à une issue",
      actions: copyButton(() => report(), "Copier"),
      body: el("div", { class: "panel-body" }, el("pre", { class: "code", style: "margin:0", text: report() })),
    }));
}

function rawDetails(data) {
  const d = el("details", {}, el("summary", { style: "cursor:pointer; color:var(--accent-text); font-size:13px; font-weight:550", text: "Réponse brute" }));
  d.addEventListener("toggle", () => { if (d.open && !d.querySelector("pre")) d.append(el("pre", { class: "code", text: JSON.stringify(data, null, 2) })); });
  return d;
}

function serviceLoginForm(probe) {
  const user = el("input", { "aria-label": "Identifiant du service", placeholder: "Identifiant du service", autocomplete: "username" });
  const pass = el("input", { "aria-label": "Mot de passe du service", placeholder: "Mot de passe", type: "password", autocomplete: "current-password" });
  const status = el("div", { "aria-live": "polite", style: "grid-column:1/-1" });
  return el("form", { class: "inline-form", onsubmit: async (e) => {
    e.preventDefault();
    const body = { service: probe.separate, username: user.value.trim(), password: pass.value };
    pass.value = "";
    const r = await call("POST", "/v1/auth/service-login", { body });
    if (r.status === 200 && r.data?.token) { setToken(r.data.token); runAll(); }
    else status.replaceChildren(problemNotice(r.problem));
  } }, user, pass, el("button", { type: "submit", class: "primary", text: "Se connecter au service" }), status);
}

function problemNotice(problem = {}) {
  const n = el("p", { class: "notice bad" }, [problem.title, problem.detail].filter(Boolean).join(" : "), problem.code ? ` (${problem.code}) ` : " ");
  if (problem.code) n.append(el("a", { href: `#/erreurs?code=${problem.code}`, text: "que faire" }));
  return n;
}

function report() {
  const lines = [`tonAPI, banc d'essai : ${new Date().toISOString().slice(0, 16)}Z`, `université : ${state.me?.university?.id || "?"}`, `semaine : ${iso(state.week)}`];
  for (const probe of PROBES) {
    const r = state.results[probe.key];
    let result = "non testé";
    if (r?.skipped) result = r.skipped;
    else if (r?.data !== undefined && r?.data !== null) result = "ok";
    else if (r?.problem) result = r.problem.code || "erreur";
    lines.push(`${probe.key.padEnd(17)} ${String(providerOf(probe.key) || "?").padEnd(17)} ${result}${r?.ms !== undefined ? ` · ${r.ms} ms` : ""}`);
  }
  return lines.join("\n");
}

async function runAll() {
  if (!state.token || state.running) return;
  state.running = true;
  state.results = {};
  route();
  const me = await call("GET", "/v1/me");
  if (!me.data) {
    state.results.profile = { problem: me.problem, ms: me.ms };
    state.running = false;
    if (me.problem?.code === "invalid_token" || me.problem?.code === "session_expired") {
      endSession();
      if (state.accountId) upsertAccount({ id: state.accountId, expiresAt: new Date().toISOString() });
    }
    return route();
  }
  const info = me.data.data;
  const detail = state.details[info.university.id] || (await call("GET", `/v1/universities/${encodeURIComponent(info.university.id)}`, { auth: false })).data;
  if (detail) state.details[info.university.id] = detail;
  state.me = { ...info, timezone: detail?.timezone };
  state.results.profile = { data: info.profile, provider: me.data.meta?.provider, ms: me.ms };
  route();
  const probes = PROBES.filter((p) => p.path);
  for (const probe of probes) state.results[probe.key] = { running: true };
  route();
  // Un seul appel, services interrogés en parallèle côté serveur ; ms = durée de chaque service.
  const query = `services=${probes.map((p) => p.key).join(",")}&from=${iso(state.week)}&to=${iso(addDays(state.week, 6))}`;
  const all = await call("GET", `/v1/overview?${query}`);
  if (all.data) {
    for (const item of all.data.data) {
      state.results[item.service] = item.problem ? { problem: item.problem, ms: item.durationMs } : { data: item.data, provider: item.provider, ms: item.durationMs };
    }
    state.overviewMs = all.ms;
  } else {
    // Serveur sans `/v1/overview` : un appel par service.
    state.overviewMs = null;
    for (const probe of probes) {
      const r = await call("GET", probe.path());
      state.results[probe.key] = r.data ? { data: r.data.data, provider: r.data.meta?.provider, ms: r.ms } : { problem: r.problem, ms: r.ms };
      route();
    }
  }
  state.running = false;
  route();
}

async function rerunTimetable() {
  if (!state.token) return;
  state.results.timetable = { running: true };
  route();
  const r = await call("GET", PROBES[1].path());
  state.results.timetable = r.data ? { data: r.data.data, provider: r.data.meta?.provider, ms: r.ms } : { problem: r.problem, ms: r.ms };
  route();
}

async function refresh() {
  const r = await call("POST", "/v1/auth/refresh");
  if (r.data?.token) setToken(r.data.token, { expiresAt: r.data.expiresAt });
  route();
}

async function logout() {
  await call("POST", "/v1/auth/logout");
  // Le jeton reste pour reconnaître l'appareil (previousToken), plus pour reprendre la session.
  if (state.accountId) upsertAccount({ id: state.accountId, expiresAt: new Date().toISOString() });
  endSession();
  state.me = null;
  state.results = {};
  renderRail();
  route();
}

function journalPanel() {
  return panel({
    title: "Journal des requêtes", sub: "Statut, heure, méthode, route et durée ; les corps ne sont pas gardés",
    actions: el("button", { type: "button", class: "ghost", onclick: () => { state.journal = []; route(); }, text: "Vider" }),
    body: el("div", {},
      state.journal.length ? null : el("p", { class: "empty", id: "journal-empty" }, el("b", { text: "Aucune requête pour l'instant" }), "Elles apparaissent ici dès que la console appelle l'API."),
      el("ul", { class: "journal", id: "journal" }, state.journal.map(journalItem))),
  });
}

function copyButton(get, label) {
  const b = el("button", { type: "button", text: label });
  b.addEventListener("click", async () => {
    try { await navigator.clipboard.writeText(get()); b.textContent = "Copié"; } catch { b.textContent = "Copie impossible"; }
    setTimeout(() => { b.textContent = label; }, 1500);
  });
  return b;
}

// ---------------------------------------------------------------------------------------------
// Page : comptes mémorisés
// ---------------------------------------------------------------------------------------------

function pageAccounts() {
  const list = accounts();
  const rows = list.map((a) => {
    const label = el("input", { value: a.label || "", "aria-label": "Nom du compte", placeholder: a.username || "Nom du compte", onchange: (e) => upsertAccount({ id: a.id, label: e.target.value.trim() }) });
    const expired = a.expiresAt && new Date(a.expiresAt).getTime() < Date.now();
    const device = a.deviceSince ? a.deviceSince + 30 * 86400000 : null;
    return el("tr", {},
      el("td", { style: "min-width:180px" }, label),
      el("td", {}, el("code", { text: a.university }), el("div", { class: "hint", text: a.username || "identifiant non renseigné" })),
      el("td", {}, a.token ? pill(expired ? "warn" : "ok", expired ? "expirée" : "valable") : pill("idle", "aucun jeton"),
        a.expiresAt ? el("div", { class: "hint", text: `jusqu'au ${new Date(a.expiresAt).toLocaleString("fr-FR")}` }) : null),
      el("td", {}, device ? el("span", { text: `≈ ${relative(device)}` }) : el("span", { class: "hint", text: "inconnu" })),
      el("td", { class: "hint", text: a.lastUsed ? new Date(a.lastUsed).toLocaleString("fr-FR") : "jamais" }),
      el("td", {}, el("div", { class: "row", style: "flex-wrap:nowrap" },
        el("button", { type: "button", onclick: () => { state.form = { ...state.form, university: a.university, username: a.username, establishment: a.establishment || "", remember: true }; state.accountId = a.id; location.hash = "#/"; }, text: "Se connecter avec" }),
        el("button", { type: "button", class: "ghost danger", onclick: () => { forgetAccount(a.id); route(); }, text: "Oublier" }))));
  });
  return [
    pageHead("Comptes mémorisés", "Pour se reconnecter sans refaire le second facteur à chaque essai. Aucun mot de passe n'est gardé.",
      list.length ? el("button", { type: "button", class: "danger", onclick: () => { saveAccounts([]); state.accountId = null; route(); }, text: "Tout oublier" }) : null),
    el("div", { class: "stack" },
      panel({
        title: `${list.length} compte(s) sur ce navigateur`, sub: "Stockage local du navigateur, propre à cette adresse de console",
        body: list.length
          ? el("div", { class: "table-wrap" }, el("table", {},
            el("thead", {}, el("tr", {}, ["Nom", "Université", "Session tonAPI", "Appareil reconnu", "Dernière connexion", ""].map((h) => el("th", { text: h })))),
            el("tbody", {}, rows)))
          : empty("Aucun compte mémorisé", "Coche « Mémoriser ce compte » à la connexion dans le banc d'essai."),
      }),
      el("div", { class: "split" },
        panel({
          title: "Comment ça marche", sub: "Du mot de passe à la reconnexion sans second facteur",
          body: el("div", { class: "panel-body" }, el("ol", { class: "steps" },
            el("li", {}, el("div", {}, el("b", { text: "Connexion avec le mot de passe" }), el("span", { text: "La console l'envoie à POST /v1/auth/login, puis vide le champ. Il n'est écrit nulle part." }))),
            el("li", {}, el("div", {}, el("b", { text: "Second facteur avec « mémoriser cet appareil »" }), el("span", { text: "Le CAS prolonge sa propre session à 30 jours (cookie connect.sid à Montpellier)." }))),
            el("li", {}, el("div", {}, el("b", { text: "Jeton chiffré rendu par l'API" }), el("span", { text: "Les cookies de l'université voyagent chiffrés (AES-256-GCM) dans le jeton, que la console garde pour ce compte." }))),
            el("li", {}, el("div", {}, el("b", { text: "Connexion suivante : previousToken" }), el("span", { text: "La console renvoie l'ancien jeton. Le serveur en reprend les cookies sauf le ticket CAS : le mot de passe reste demandé, l'appareil est reconnu." }))),
            el("li", {}, el("div", {}, el("b", { text: "Renouvellements" }), el("span", { text: "Une réponse peut renvoyer un jeton à jour (en-tête TonAPI-Session) ; la console remplace celui du compte." }))),
            el("li", {}, el("div", {}, el("b", { text: "Rechargement de la page" }), el("span", { text: "Tant que le jeton du compte est valable (12 h par défaut), la console reprend la session et relance les tests sans mot de passe. « Se déconnecter » ou une session refusée y met fin." }))))),
        }),
        panel({
          title: "Limites et sécurité", sub: "À savoir avant de mémoriser un compte",
          body: el("div", { class: "panel-body" }, el("ul", { class: "plain" },
            el("li", { text: "Le jeton est chiffré par le serveur mais reste un secret : il ouvre la session tant qu'elle est valable (12 h par défaut). À éviter sur un ordinateur partagé." }),
            el("li", { text: "Un ancien jeton est accepté comme previousToken jusqu'à 35 jours après son expiration, uniquement pour en reprendre les cookies." }),
            el("li", { text: "Sans TONAPI_SESSION_KEYS, le serveur crée une clé temporaire : les jetons mémorisés deviennent invalides à chaque redémarrage." }),
            el("li", { text: "La durée « appareil reconnu » est une estimation (30 jours depuis la dernière validation) : l'université peut l'écourter." }))),
        }))),
  ];
}

// ---------------------------------------------------------------------------------------------
// Page : couverture
// ---------------------------------------------------------------------------------------------

async function loadDetails() {
  await Promise.all(state.universities.map(async (u) => {
    if (state.details[u.id]) return;
    const r = await call("GET", `/v1/universities/${encodeURIComponent(u.id)}`, { auth: false });
    if (r.data) state.details[u.id] = r.data;
  }));
}

/// État d'un service pour un établissement, du plus sûr au moins sûr, en mots simples.
const ETATS = {
  ok: { ton: "ok", court: "Vérifié", long: "Essayé avec un vrai compte : données reçues." },
  vide: { ton: "warn", court: "Répond, rien à montrer", long: "Essayé avec un vrai compte, qui n'avait encore rien (notes non publiées…) : format à confirmer." },
  code: { ton: "idle", court: "Pas encore essayé ici", long: "Codé et testé sur de faux serveurs, jamais avec un compte de cet établissement." },
  demo: { ton: "idle", court: "Démo", long: "Données fictives de l'université de démonstration." },
  lien: { ton: "idle", court: "Lien", long: "L'API renvoie vers la page de l'université." },
  absent: { ton: "idle", court: "Pas proposé", long: "Ni données ni lien pour ce service." },
};
const VERIFIE = window.TONAPI_VERIFIE || {};

function etat(status, key, service) {
  if (!status || status.availability === "absent") return "absent";
  if (status.availability === "link") return "lien";
  if (status.provider === "demo") return "demo";
  return VERIFIE[key]?.services?.[service] || "code";
}

function serviceTile(service, status, key, detail) {
  const e = etat(status, key, service.key);
  const info = ETATS[e];
  const provider = status?.availability === "api" && e !== "demo" ? providerFor(status, detail) : null;
  const inner = [
    el("span", { class: "tile-name", text: service.label }),
    el("span", { class: `tile-state ${info.ton}`, text: info.court }),
    provider ? el("span", { class: "tile-provider", text: provider }) : null,
  ];
  const attrs = { class: `tile ${e}`, title: info.long };
  if (provider) return el("a", { ...attrs, href: `#/fournisseurs?id=${encodeURIComponent(provider)}` }, inner);
  if (e === "lien") return el("a", { ...attrs, href: status.link, target: "_blank", rel: "noopener noreferrer" }, inner);
  return el("div", attrs, inner);
}

function coverageBlock(title, sub, statuses, key, detail) {
  const byService = Object.fromEntries(statuses.map((s) => [s.service, s]));
  const etats = SERVICES.map((sv) => etat(byService[sv.key], key, sv.key));
  const ok = etats.filter((e) => e === "ok").length;
  const usable = etats.filter((e) => !["absent", "lien"].includes(e)).length;
  const essai = VERIFIE[key];
  const resume = etats.includes("demo")
    ? "Données fictives, pour essayer l'API sans compte."
    : `${usable} services sur ${SERVICES.length} servis par l'API, dont ${ok === 0 ? "aucun vérifié" : `${ok} ${ok > 1 ? "vérifiés" : "vérifié"}`} avec un vrai compte${essai ? ` (${essai.date})` : ""}.`;
  return el("section", { class: "cov-block" },
    el("div", { class: "cov-head" }, el("h3", { text: title }), sub ? el("span", { text: sub }) : null),
    el("p", { class: "cov-summary", text: resume }),
    el("div", { class: "tiles" }, SERVICES.map((sv) => serviceTile(sv, byService[sv.key], key, detail))));
}

function loginText(d) {
  const kind = { cas: "CAS de l'université", demo: "identifiants de démonstration" }[d.login.kind] || d.login.kind;
  return `Connexion : ${kind}${d.login.mfa ? ", second facteur possible" : ""}`;
}

function pageCoverage() {
  const body = el("div", { class: "stack" }, el("p", { class: "empty", "aria-live": "polite" }, el("b", { text: "Chargement" }), "Lecture du détail de chaque université…"));
  if (state.health && !state.health.ok) body.replaceChildren(empty("API injoignable", "Vérifie l'adresse dans le menu et que tonapi-server tourne."));
  else if (state.health && !state.universities.length) body.replaceChildren(empty("Aucune université servie", "Ajoute un dossier dans universities/ puis lance tonapi validate."));
  else loadDetails().then(() => {
    const cards = [];
    for (const u of state.universities) {
      const d = state.details[u.id];
      if (!d) continue;
      const ests = d.establishments || [];
      // Avec des composantes, chacune est ce qu'un étudiant reçoit vraiment : pas de bloc université.
      const blocks = ests.length
        ? ests.map((e) => coverageBlock(e.name, e.default ? "composante par défaut" : "", e.services, `${d.id}/${e.id}`, d))
        : [coverageBlock("Toute l'université", "", d.services, d.id, d)];
      cards.push(panel({ title: d.name, sub: `${loginText(d)} · état ${d.status}`, body: el("div", { class: "panel-body stack" }, blocks) }));
    }
    body.replaceChildren(...cards);
  });
  const legend = el("ul", { class: "cov-legend" },
    ["ok", "vide", "code", "lien", "absent"].map((k) => el("li", {},
      el("span", { class: `tile-state ${ETATS[k].ton}`, text: ETATS[k].court }),
      el("span", { text: ETATS[k].long }))));
  return [
    pageHead("Couverture", "Ce qu'un étudiant obtient dans chaque établissement, et ce qui a vraiment été vérifié avec un compte de cet établissement."),
    panel({ title: "Comment lire", body: el("div", { class: "panel-body" }, legend) }),
    body,
  ];
}

// ---------------------------------------------------------------------------------------------
// Page : fournisseurs
// ---------------------------------------------------------------------------------------------

function pageProviders(params) {
  const focus = params.get("id");
  const filter = params.get("service") || "";
  const ids = focus === "portal" ? ["portal", "tiles-portal"] : focus ? [focus] : [];
  const shown = FOURNISSEURS.filter((f) => !filter || f.services.includes(filter));
  const filters = el("div", { class: "filters", role: "group", "aria-label": "Filtrer par service" },
    el("button", { type: "button", "aria-pressed": String(!filter), onclick: () => { location.hash = "#/fournisseurs"; }, text: "Tous" }),
    SERVICES.filter((s) => FOURNISSEURS.some((f) => f.services.includes(s.key))).map((s) =>
      el("button", { type: "button", "aria-pressed": String(filter === s.key), onclick: () => { location.hash = `#/fournisseurs?service=${s.key}`; }, text: s.label })));
  const cards = shown.map((f) => {
    const essai = ESSAIS[f.essai];
    return el("article", { class: `panel card${ids.includes(f.id) ? " focus" : ""}`, id: `f-${f.id}` },
      el("div", { class: "panel-body" },
        el("h2", {}, el("code", { text: f.id }), pill(essai.ton, essai.label)),
        el("p", { class: "logiciel", text: `${f.logiciel} · ${f.services.map(serviceLabel).join(", ")}` }),
        routeOf(f.id),
        el("p", { text: f.comment }),
        f.essais.length ? el("div", {}, el("h3", { text: "Essais réels" }), el("ul", {}, f.essais.map((x) => el("li", { text: x })))) : null,
        f.limites.length ? el("div", {}, el("h3", { text: "Limites" }), el("ul", {}, f.limites.map((x) => el("li", { text: x })))) : null,
        el("details", {}, el("summary", { text: "Configuration TOML" }), el("pre", { class: "code", text: f.config })),
        el("p", { class: "hint", style: "margin:0" }, "Code : ", f.fichiers.map((x, i) => [i ? ", " : "", el("code", { text: x })]))));
  });
  setTimeout(() => { const t = document.querySelector(".card.focus"); if (t) t.scrollIntoView({ block: "center", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" }); }, 0);
  return [
    pageHead("Fournisseurs", "Un fournisseur correspond à un logiciel, jamais à une université. Pour chacun : le parcours suivi, ce qui a été essayé pour de vrai, et ses limites."),
    filters,
    shown.length ? el("div", { class: "cards" }, cards) : panel({ body: empty("Aucun fournisseur pour ce service", "Choisis un autre filtre.") }),
  ];
}

// ---------------------------------------------------------------------------------------------
// Page : explorateur d'API
// ---------------------------------------------------------------------------------------------

const explorer = { op: null };

function skeleton(schema, spec, depth = 0) {
  if (!schema || depth > 4) return null;
  if (schema.$ref) return skeleton(spec.components.schemas[schema.$ref.split("/").pop()], spec, depth + 1);
  if (schema.example !== undefined) return schema.example;
  if (schema.enum) return schema.enum[0];
  const type = Array.isArray(schema.type) ? schema.type.find((t) => t !== "null") : schema.type;
  if (type === "object" || schema.properties) return Object.fromEntries(Object.entries(schema.properties || {}).filter(([k]) => (schema.required || []).includes(k)).map(([k, v]) => [k, skeleton(v, spec, depth + 1)]));
  if (type === "array") return [];
  if (type === "boolean") return false;
  if (type === "integer" || type === "number") return 0;
  if (schema.oneOf || schema.anyOf) return skeleton((schema.oneOf || schema.anyOf)[0], spec, depth + 1);
  return "";
}

function pageExplorer() {
  const list = el("nav", { class: "panel endpoints", "aria-label": "Routes de l'API" }, el("p", { class: "empty" }, el("b", { text: "Chargement" }), "Lecture de /openapi.json…"));
  const pane = el("section", { class: "panel" }, empty("Choisis une route", "Le formulaire reprend ses paramètres et un squelette du corps JSON."));
  const load = state.openapi ? Promise.resolve() : call("GET", "/openapi.json", { auth: false }).then((r) => { state.openapi = r.data; });
  load.then(() => {
    const spec = state.openapi;
    if (!spec) { list.replaceChildren(empty("Spécification indisponible", "L'API ne répond pas sur /openapi.json : vérifie l'adresse dans le menu.")); return; }
    const groups = {};
    for (const [path, ops] of Object.entries(spec.paths)) for (const [method, op] of Object.entries(ops)) (groups[op.tags?.[0] || "autres"] ||= []).push({ path, method, op });
    list.replaceChildren(...Object.entries(groups).flatMap(([tag, ops]) => [el("h3", { text: tag }), ...ops.map((o) =>
      el("button", { type: "button", class: "endpoint", "aria-pressed": String(explorer.op?.path === o.path && explorer.op?.method === o.method), onclick: () => { explorer.op = o; route(); } },
        el("span", { class: `method ${o.method}`, text: o.method.toUpperCase() }), el("span", { text: o.path })))]));
    if (explorer.op) pane.replaceChildren(...explorerForm(explorer.op, spec));
  });
  return [
    pageHead("Explorateur d'API", "Toutes les routes de la spécification OpenAPI, appelées avec la session active.",
      el("a", { class: "button", href: `${apiBase()}/docs`, target: "_blank", rel: "noopener noreferrer", text: "Documentation /docs" })),
    el("div", { class: "explorer" }, list, pane),
  ];
}

function explorerForm(o, spec) {
  const params = (o.op.parameters || []).map((p) => ({ p, input: el("input", { id: `p-${p.name}`, value: p.name === "from" ? iso(state.week) : p.name === "to" ? iso(addDays(state.week, 6)) : p.name === "id" ? (state.me?.university?.id || state.universities[0]?.id || "") : "" }) }));
  const bodySchema = o.op.requestBody?.content?.["application/json"]?.schema;
  const textarea = bodySchema ? el("textarea", { id: "req-body", spellcheck: "false" }, JSON.stringify(skeleton(bodySchema, spec), null, 2)) : null;
  const secured = (o.op.security || []).some((s) => Object.keys(s).length);
  const useToken = el("input", { type: "checkbox", checked: secured && !!state.token });
  const out = el("div", {});
  const build = () => {
    let path = o.path;
    const query = new URLSearchParams();
    for (const { p, input } of params) {
      if (!input.value) continue;
      if (p.in === "path") path = path.replace(`{${p.name}}`, encodeURIComponent(input.value));
      else if (p.in === "query") query.set(p.name, input.value);
    }
    return path + (query.toString() ? `?${query}` : "");
  };
  const send = async () => {
    let body;
    if (textarea) { try { body = JSON.parse(textarea.value); } catch { out.replaceChildren(el("p", { class: "notice bad", text: "Corps JSON invalide." })); return; } }
    out.replaceChildren(el("p", { class: "hint", text: "Envoi…" }));
    const r = await call(o.method.toUpperCase(), build(), { body, auth: useToken.checked });
    if (r.data?.token && /\/auth\/(login|mfa|refresh|service-login)$/.test(o.path)) { state.token = r.data.token; renderRail(); }
    const tone = r.status >= 200 && r.status < 300 ? "ok" : r.status === 202 ? "warn" : "bad";
    out.replaceChildren(
      el("div", { class: "response-meta" },
        pill(tone, String(r.status || "réseau")),
        el("span", { text: `${r.ms} ms` }),
        el("span", { text: r.headers.get("Content-Type") || "sans Content-Type" }),
        r.headers.get("TonAPI-Session") ? el("span", { text: "TonAPI-Session : jeton renouvelé" }) : null,
        r.headers.get("Cache-Control") ? el("span", { text: `Cache-Control : ${r.headers.get("Cache-Control")}` }) : null),
      el("pre", { class: "code", text: typeof r.raw === "string" ? r.raw : JSON.stringify(r.raw, null, 2) }));
  };
  const curl = () => {
    let body = textarea?.value;
    if (body) { try { const j = JSON.parse(body); for (const k of ["password", "code", "pending", "previousToken"]) if (k in j) j[k] = `<${k}>`; body = JSON.stringify(j); } catch { /* corps tel quel */ } }
    return [`curl -X ${o.method.toUpperCase()} '${apiBase()}${build()}'`, useToken.checked ? `  -H "Authorization: Bearer $TONAPI_TOKEN"` : null, body ? `  -H 'Content-Type: application/json' -d '${body.replace(/'/g, "'\\''")}'` : null].filter(Boolean).join(" \\\n");
  };
  return [
    el("div", { class: "panel-head" },
      el("div", {}, el("div", { class: "endpoint-title" }, el("span", { class: `method ${o.method}`, text: o.method.toUpperCase() }), el("code", { text: o.path })),
        el("p", { text: o.op.summary || o.op.description || "" })),
      el("div", { class: "row" }, copyButton(curl, "Copier en curl"), el("button", { class: "primary", type: "button", onclick: send, text: "Envoyer" }))),
    el("div", { class: "panel-body" },
      params.length ? el("div", { class: "grid" }, params.map(({ p, input }) => el("div", {}, el("label", { for: `p-${p.name}`, text: `${p.name} · ${p.in}${p.required ? ", requis" : ""}` }), input))) : null,
      textarea ? el("div", { style: params.length ? "margin-top:14px" : null }, el("label", { for: "req-body", text: "Corps JSON (squelette d'après le schéma)" }), textarea) : null,
      el("label", { class: "check", style: params.length || textarea ? "margin-top:14px" : null }, useToken, el("span", { text: state.token ? "Envoyer le jeton de la session active" : "Envoyer le jeton (aucune session active)" })),
      el("p", { class: "hint", text: "La commande curl remplace le jeton par $TONAPI_TOKEN et masque mots de passe et codes." }),
      el("div", { style: "margin-top:14px" }, out)),
  ];
}

// ---------------------------------------------------------------------------------------------
// Page : codes d'erreur
// ---------------------------------------------------------------------------------------------

function pageErrors(params) {
  const focus = params.get("code");
  setTimeout(() => { const t = document.getElementById(`e-${focus}`); if (t) t.scrollIntoView({ block: "center" }); }, 0);
  return [
    pageHead("Codes d'erreur", "Réponses au format RFC 9457 (application/problem+json). Un client se fie à code, jamais au texte de title ou detail."),
    panel({
      title: `${ERREURS.length} codes stables`, sub: "Définis dans crates/tonapi-core/src/problem.rs",
      body: el("div", { class: "table-wrap" }, el("table", {},
        el("thead", {}, el("tr", {}, ["Code", "HTTP", "Titre", "Cause habituelle", "Que faire"].map((h) => el("th", { text: h })))),
        el("tbody", {}, ERREURS.map((e) => el("tr", { id: `e-${e.code}`, style: e.code === focus ? "background:var(--accent-soft)" : null },
          el("td", {}, el("code", { text: e.code })), el("td", { class: "mono", text: e.status }), el("td", { text: e.titre }), el("td", { text: e.cause }), el("td", { text: e.faire })))))),
    }),
  ];
}

// ---------------------------------------------------------------------------------------------
// Routeur, thème, démarrage
// ---------------------------------------------------------------------------------------------

const PAGES = { "": pageBench, comptes: pageAccounts, couverture: pageCoverage, fournisseurs: pageProviders, api: pageExplorer, erreurs: pageErrors };

function route() {
  const [path, query] = location.hash.replace(/^#\/?/, "").split("?");
  const page = PAGES[path] ? path : "";
  for (const a of document.querySelectorAll(".nav a")) a.setAttribute("aria-current", a.dataset.page === page ? "page" : "false");
  const focused = document.activeElement?.id;
  $("view").replaceChildren(...PAGES[page](new URLSearchParams(query || "")));
  if (focused && $(focused)) $(focused).focus();
}

/// Thème : automatique (système), clair ou sombre, gardé sur ce navigateur.
function applyTheme(theme) {
  if (theme === "light" || theme === "dark") document.documentElement.dataset.theme = theme;
  else delete document.documentElement.dataset.theme;
  for (const b of document.querySelectorAll(".theme button")) b.setAttribute("aria-pressed", String(b.dataset.theme === (theme || "auto")));
}
for (const b of document.querySelectorAll(".theme button")) {
  b.addEventListener("click", () => { store.set("tonapi.theme", b.dataset.theme); applyTheme(b.dataset.theme); });
}
applyTheme(store.get("tonapi.theme", "auto"));

$("api").value = state.api;
$("api").addEventListener("change", (e) => { state.api = e.target.value.trim(); store.set("tonapi.api", state.api); state.details = {}; state.openapi = null; checkHealth(); });
window.addEventListener("hashchange", () => { route(); $("view").focus({ preventScroll: true }); });
renderRail();
route();
checkHealth();
resumeOnLoad();
