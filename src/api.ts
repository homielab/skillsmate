// Typed wrappers around the Tauri commands in src-tauri/src. The backend
// builds every string the UI shows; this file only describes the shapes.

import { invoke } from "@tauri-apps/api/core";

/** Status mark: 0 fine, 1 warning, 2 failure, 3 neutral, 4 tip. */
export type Kind = 0 | 1 | 2 | 3 | 4;

export interface Bootstrap {
  version: string;
  theme: "dark" | "light" | null;
  mac: boolean;
  canCheckAppUpdate: boolean;
  online: boolean;
  needsAccess: boolean;
  accessHome: string;
}

export interface SkillRow {
  key: string;
  name: string;
  description: string;
  agents: string;
  source: string;
  meta: string;
  update: string;
  managed: boolean;
  visible: boolean;
}

export interface AgentRow {
  name: string;
  id: string;
  detected: boolean;
  universal: boolean;
  globalDir: string;
  skills: string;
}

export interface Bar {
  label: string;
  value: string;
  fraction: number;
}

export interface Overview {
  skills: string;
  skillsNote: string;
  agents: string;
  agentsNote: string;
  disk: string;
  diskNote: string;
  updated: string;
  updatedNote: string;
  perAgent: Bar[];
  perAgentNote: string;
  perSource: Bar[];
  perSourceNote: string;
  recent: { name: string; source: string; when: string }[];
  attention: { text: string; kind: Kind }[];
}

export interface FindingRow {
  title: string;
  detail: string;
  skill: string;
  key: string;
  kind: Kind;
  fixLabel: string;
  index: number;
}

export interface UpdateRow {
  name: string;
  key: string;
  source: string;
  label: string;
  kind: Kind;
  summary: string;
}

export interface Health {
  summary: string;
  problems: number;
  warnings: number;
  tips: number;
  startup: string;
  startupNote: string;
  findings: FindingRow[];
  brokenLinks: number;
  badge: number;
  updates: UpdateRow[];
  updatesStatus: string;
}

export interface Library {
  overview: Overview;
  skills: SkillRow[];
  status: string;
  agents: AgentRow[];
  detectedAgents: number;
  health: Health;
}

export interface Detail {
  key: string;
  name: string;
  description: string;
  version: string;
  source: string;
  managed: boolean;
  location: string;
  size: string;
  filesCount: string;
  activeTokens: string;
  startupTokens: string;
  updateLabel: string;
  updateKind: Kind;
  updateSummary: string;
  canUpdate: boolean;
  url: string;
  license: string;
  compatibility: string;
  allowedTools: string;
  agents: { name: string; how: string }[];
  findings: FindingRow[];
  /** kind: 0 file header, 1 added, 2 removed, 3 context, 4 note */
  diff: { kind: number; text: string }[];
  /** kind: 0 h1, 1 h2, 2 h3, 3 paragraph, 4 bullet, 5 code, 6 quote */
  body: { kind: number; text: string; depth: number }[];
  files: { path: string; size: string }[];
}

export interface AuditRow {
  name: string;
  source: string;
  kind: Kind;
  label: string;
  detail: string;
  url: string;
}

export interface Audits {
  rows: AuditRow[];
  status: string;
  flagged: number;
  overview: Overview;
}

export interface SkillCard {
  name: string;
  description: string;
  source: string;
  slug: string;
  installs: string;
  url: string;
  installed: boolean;
  installable: boolean;
}

export interface CardList {
  rows: SkillCard[];
  status: string;
}

export interface OfficialRow {
  owner: string;
  stats: string;
  top: string[];
  installed: string;
  url: string;
}

export interface TopicRow {
  slug: string;
  title: string;
  description: string;
  count: string;
}

export interface Confirm {
  title: string;
  message: string;
  confirm: string;
  destructive: boolean;
  busy: string;
}

export const api = {
  bootstrap: () => invoke<Bootstrap>("bootstrap"),
  grantAccess: () => invoke<void>("grant_access"),
  checkAppUpdate: (manual: boolean) =>
    invoke<{ status: string; url: string }>("check_app_update", { manual }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),

  refresh: () => invoke<Library>("refresh"),
  checkUpdates: () =>
    invoke<{ library: Library; note: string | null }>("check_updates"),
  detail: (key: string) => invoke<Detail | null>("detail", { key }),
  checkAudits: () => invoke<Audits>("check_audits"),

  search: (query: string) => invoke<CardList>("search", { query }),
  official: (force: boolean, filter: string) =>
    invoke<{ rows: OfficialRow[]; status: string }>("official", { force, filter }),
  topics: (force: boolean) => invoke<TopicRow[]>("topics", { force }),
  topic: (slug: string) =>
    invoke<{ title: string; description: string; status: string; rows: SkillCard[] }>(
      "topic",
      { slug },
    ),
  recommendations: (fetch: boolean) => invoke<CardList>("recommendations", { fetch }),

  prepareInstall: (source: string, slug: string) =>
    invoke<Confirm>("prepare_install", { source, slug }),
  prepareUpdate: (key: string) => invoke<Confirm | null>("prepare_update", { key }),
  prepareRemove: (key: string) => invoke<Confirm>("prepare_remove", { key }),
  prepareFix: (index: number) => invoke<Confirm>("prepare_fix", { index }),
  prepareFixAllLinks: () => invoke<Confirm>("prepare_fix_all_links"),
  cancelPending: () => invoke<void>("cancel_pending"),
  confirmPending: () => invoke<string>("confirm_pending"),
  readSkillMd: (key: string) => invoke<string>("read_skill_md", { key }),
  saveSkillMd: (key: string, text: string) =>
    invoke<string>("save_skill_md", { key, text }),
  undo: () => invoke<string>("undo"),
};

/** Commands reject with a plain string; anything else is a bug worth seeing. */
export function message(error: unknown): string {
  return typeof error === "string" ? error : String(error);
}
