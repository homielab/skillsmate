// The app-wide state every page reads, provided by <App>.

import { createContext, useContext } from "react";
import type { Audits, Bootstrap, Confirm, Library } from "./api";

export type PageId =
  | "overview"
  | "installed"
  | "health"
  | "audits"
  | "discover"
  | "official"
  | "topics"
  | "agents";

export interface AppContext {
  boot: Bootstrap;
  /** `null` until the first scan finishes. */
  library: Library | null;
  /** A scan is running. */
  loading: boolean;
  /** Grows every time the library changes; pages re-read what depends on it. */
  revision: number;
  refresh: () => Promise<void>;

  go: (page: PageId) => void;
  openSkill: (key: string) => void;
  openUrl: (url: string) => void;

  /** Asks the backend to prepare a change, confirms it with the user, runs it
   * and offers Undo. `null` from `prepare` means there was nothing to ask. */
  act: (prepare: () => Promise<Confirm | null>) => Promise<void>;
  install: (source: string, slug: string) => void;
  /** `source/slug` of the skill being downloaded for install, if any. */
  installing: string | null;
  updateSkill: (key: string) => void;
  /** Shows a line at the bottom: a result, an error, or a change that can be
   * undone. */
  notify: (text: string, kind?: "error" | "undo") => void;

  checkUpdates: () => Promise<void>;
  checkingUpdates: boolean;

  audits: Audits | null;
  checkAudits: () => Promise<void>;
  checkingAudits: boolean;
}

export const AppCtx = createContext<AppContext | null>(null);

export function useApp(): AppContext {
  const ctx = useContext(AppCtx);
  if (!ctx) throw new Error("useApp must be used inside <App>");
  return ctx;
}
