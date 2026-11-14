// The window: sidebar, the open page, and the overlays. Owns the state every
// page shares and exposes it through `AppCtx`.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, type Audits, type Bootstrap, type Confirm, type Library, message } from "./api";
import {
  AccessScreen,
  type Command,
  CommandPalette,
  ConfirmDialog,
  Toast,
  type ToastSpec,
} from "./overlays";
import { AgentsPage } from "./pages/Agents";
import { AuditsPage } from "./pages/Audits";
import { DetailPage } from "./pages/Detail";
import { DiscoverPage } from "./pages/Discover";
import { HealthPage } from "./pages/Health";
import { InstalledPage } from "./pages/Installed";
import { OfficialPage } from "./pages/Official";
import { OverviewPage } from "./pages/Overview";
import { TopicsPage } from "./pages/Topics";
import { AppCtx, type AppContext, type PageId } from "./store";
import { Button, Logo } from "./ui";

/** How long a result toast stays up. */
const TOAST_MS = 8000;

const NAV: { group: string; pages: { id: PageId; label: string }[] }[] = [
  {
    group: "Library",
    pages: [
      { id: "overview", label: "Overview" },
      { id: "installed", label: "Installed" },
      { id: "health", label: "Health" },
      { id: "audits", label: "Audits" },
    ],
  },
  {
    group: "Explore",
    pages: [
      { id: "discover", label: "Discover" },
      { id: "official", label: "Official" },
      { id: "topics", label: "Topics" },
    ],
  },
  { group: "Setup", pages: [{ id: "agents", label: "Agents" }] },
];
const PAGES = NAV.flatMap((g) => g.pages);

const PAGE_VIEWS: Record<PageId, () => React.JSX.Element> = {
  overview: OverviewPage,
  installed: InstalledPage,
  health: HealthPage,
  audits: AuditsPage,
  discover: DiscoverPage,
  official: OfficialPage,
  topics: TopicsPage,
  agents: AgentsPage,
};

export function App() {
  const [boot, setBoot] = useState<Bootstrap | null>(null);
  const [fatal, setFatal] = useState("");

  const loadBoot = useCallback(() => {
    api.bootstrap().then(setBoot, (e) => setFatal(message(e)));
  }, []);
  useEffect(loadBoot, [loadBoot]);

  useEffect(() => {
    if (!boot) return;
    const root = document.documentElement;
    root.classList.toggle("mac", boot.mac);
    if (boot.theme) root.dataset.theme = boot.theme;
  }, [boot]);

  if (fatal) return <div className="access">{fatal}</div>;
  if (!boot) return null;
  if (boot.needsAccess) return <AccessScreen home={boot.accessHome} onGranted={loadBoot} />;
  return <Shell boot={boot} />;
}

function Shell({ boot }: { boot: Bootstrap }) {
  const [page, setPage] = useState<PageId>("overview");
  // Pages stay mounted once visited, so filters, searches and scroll
  // positions survive moving around.
  const [visited, setVisited] = useState<PageId[]>(["overview"]);
  /** Frontmatter name of the skill open in the detail view. */
  const [skill, setSkill] = useState<string | null>(null);

  const [library, setLibrary] = useState<Library | null>(null);
  const [loading, setLoading] = useState(true);
  const [revision, setRevision] = useState(0);
  const [checkingUpdates, setCheckingUpdates] = useState(false);
  const [audits, setAudits] = useState<Audits | null>(null);
  const [checkingAudits, setCheckingAudits] = useState(false);
  const [installing, setInstalling] = useState<string | null>(null);

  const [dialog, setDialog] = useState<Confirm | null>(null);
  const [toast, setToast] = useState<ToastSpec | null>(null);
  const [palette, setPalette] = useState(false);
  const toastTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const [appUpdate, setAppUpdate] = useState({ status: "", url: "" });
  const [checkingApp, setCheckingApp] = useState(false);

  // ---------------------------------------------------------------- toast

  const show = useCallback((spec: ToastSpec | null) => {
    clearTimeout(toastTimer.current);
    setToast(spec);
    if (spec && !spec.busy) {
      toastTimer.current = setTimeout(() => setToast(null), TOAST_MS);
    }
  }, []);
  const notify = useCallback(
    (text: string, kind?: "error" | "undo") =>
      show({ text, error: kind === "error", undo: kind === "undo" }),
    [show],
  );

  // -------------------------------------------------------------- library

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      setLibrary(await api.refresh());
      // Audit results describe the previous scan; drop them.
      setAudits(null);
      setRevision((n) => n + 1);
    } catch (e) {
      notify(message(e), "error");
    } finally {
      setLoading(false);
    }
  }, [notify]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const checkUpdates = useCallback(async () => {
    setCheckingUpdates(true);
    try {
      const result = await api.checkUpdates();
      setLibrary(result.library);
      setRevision((n) => n + 1);
      if (result.note) notify(result.note);
    } catch (e) {
      notify(message(e), "error");
    } finally {
      setCheckingUpdates(false);
    }
  }, [notify]);

  const checkAudits = useCallback(async () => {
    setCheckingAudits(true);
    try {
      const result = await api.checkAudits();
      setAudits(result);
      setLibrary((lib) => lib && { ...lib, overview: result.overview });
    } catch (e) {
      notify(message(e), "error");
    } finally {
      setCheckingAudits(false);
    }
  }, [notify]);

  // ----------------------------------------------------------- navigation

  const go = useCallback((next: PageId) => {
    setSkill(null);
    setPage(next);
    setVisited((v) => (v.includes(next) ? v : [...v, next]));
  }, []);
  const openSkill = useCallback((key: string) => setSkill(key), []);
  const openUrl = useCallback(
    (url: string) => {
      api.openUrl(url).catch((e) => notify(message(e), "error"));
    },
    [notify],
  );

  // -------------------------------------------------------------- actions

  const act = useCallback(
    async (prepare: () => Promise<Confirm | null>) => {
      try {
        const confirm = await prepare();
        if (confirm) setDialog(confirm);
      } catch (e) {
        notify(message(e), "error");
      }
    },
    [notify],
  );

  const install = useCallback(
    (source: string, slug: string) => {
      setInstalling(`${source}/${slug}`);
      show({ text: `Getting ${slug} from skills.sh…`, busy: true });
      void act(() => api.prepareInstall(source, slug).finally(() => show(null))).finally(() =>
        setInstalling(null),
      );
    },
    [act, show],
  );

  const updateSkill = useCallback(
    (key: string) => {
      void act(async () => {
        const confirm = await api.prepareUpdate(key);
        // Not checked yet: find out what's published first.
        if (!confirm) void checkUpdates();
        return confirm;
      });
    },
    [act, checkUpdates],
  );

  const acceptDialog = async () => {
    const confirm = dialog;
    setDialog(null);
    if (!confirm) return;
    show({ text: confirm.busy, busy: true });
    try {
      show({ text: await api.confirmPending(), undo: true });
    } catch (e) {
      notify(message(e), "error");
    }
    await refresh();
  };

  const dismissDialog = () => {
    setDialog(null);
    void api.cancelPending();
  };

  const undo = async () => {
    show({ text: "Undoing…", busy: true });
    try {
      notify(await api.undo());
    } catch (e) {
      notify(message(e), "error");
    }
    await refresh();
  };

  // ----------------------------------------------------------- app update

  const checkApp = useCallback(async (manual: boolean) => {
    setCheckingApp(true);
    try {
      const result = await api.checkAppUpdate(manual);
      if (result.status || result.url) setAppUpdate(result);
    } catch {
      if (manual) setAppUpdate({ status: "Couldn't check for updates", url: "" });
    } finally {
      setCheckingApp(false);
    }
  }, []);
  useEffect(() => {
    if (boot.canCheckAppUpdate) void checkApp(false);
  }, [boot.canCheckAppUpdate, checkApp]);

  // ------------------------------------------------------------- keyboard

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key === "k") {
        e.preventDefault();
        setPalette((open) => !open);
      } else if (mod && e.key === "r") {
        // Rescan instead of reloading the web view.
        e.preventDefault();
        void refresh();
      } else if (mod && /^[1-8]$/.test(e.key)) {
        e.preventDefault();
        go(PAGES[Number(e.key) - 1].id);
      } else if (e.key === "Escape" && !e.defaultPrevented) {
        if (palette) setPalette(false);
        else if (!dialog && skill) setSkill(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [refresh, go, palette, dialog, skill]);

  const commands = useMemo<Command[]>(() => {
    const pages = PAGES.map((p, i) => ({
      id: `page:${p.id}`,
      label: p.label,
      hint: `Page · ⌘${i + 1}`,
      run: () => go(p.id),
    }));
    const actions: Command[] = [
      { id: "act:refresh", label: "Refresh", hint: "Action · ⌘R", run: () => void refresh() },
      {
        id: "act:updates",
        label: "Check for skill updates",
        hint: "Action",
        run: () => {
          go("health");
          void checkUpdates();
        },
      },
      {
        id: "act:audits",
        label: "Check security audits",
        hint: "Action",
        run: () => {
          go("audits");
          void checkAudits();
        },
      },
    ];
    const skills = (library?.skills ?? []).map((s) => ({
      id: `skill:${s.key}`,
      label: s.name,
      hint: `Skill · ${s.source}`,
      run: () => openSkill(s.key),
    }));
    return [...pages, ...actions, ...skills];
  }, [library, go, refresh, checkUpdates, checkAudits, openSkill]);

  const ctx: AppContext = {
    boot,
    library,
    loading,
    revision,
    refresh,
    go,
    openSkill,
    openUrl,
    act,
    install,
    installing,
    updateSkill,
    notify,
    checkUpdates,
    checkingUpdates,
    audits,
    checkAudits,
    checkingAudits,
  };

  const badges: Partial<Record<PageId, string>> = {
    installed: library?.skills.length ? String(library.skills.length) : "",
    health: library?.health.badge ? String(library.health.badge) : "",
    audits: audits?.flagged ? `${audits.flagged} flagged` : "",
    agents: library?.detectedAgents ? String(library.detectedAgents) : "",
  };

  return (
    <AppCtx.Provider value={ctx}>
      <div className="app">
        <aside className="sidebar">
          <div className="titlebar" data-tauri-drag-region />
          <div className="brand">
            <Logo />
            <div>
              <strong>SkillsMate</strong>
              <span>for skills.sh</span>
            </div>
          </div>
          <button type="button" className="palette-open" onClick={() => setPalette(true)}>
            <span>Go to…</span>
            <kbd>{boot.mac ? "⌘K" : "Ctrl K"}</kbd>
          </button>
          <nav>
            {NAV.map((group) => (
              <div key={group.group} className="nav-group">
                <h3>{group.group}</h3>
                {group.pages.map((p) => (
                  <button
                    key={p.id}
                    type="button"
                    className={`nav-item${page === p.id ? " on" : ""}`}
                    aria-current={page === p.id && !skill ? "page" : undefined}
                    onClick={() => go(p.id)}
                  >
                    <span>{p.label}</span>
                    {badges[p.id] && <span className="nav-badge">{badges[p.id]}</span>}
                  </button>
                ))}
              </div>
            ))}
          </nav>
          <footer className="sidebar-foot">
            <span>SkillsMate {boot.version}</span>
            {appUpdate.status && <span>{appUpdate.status}</span>}
            {!boot.online && <span>Online features are unavailable.</span>}
            {boot.canCheckAppUpdate &&
              (appUpdate.url ? (
                <Button variant="primary" onClick={() => openUrl(appUpdate.url)}>
                  Download
                </Button>
              ) : (
                <Button busy={checkingApp} onClick={() => void checkApp(true)}>
                  {checkingApp ? "Checking…" : "Check for updates"}
                </Button>
              ))}
          </footer>
        </aside>

        <main className="main">
          <div className="titlebar" data-tauri-drag-region />
          {visited.map((id) => {
            const View = PAGE_VIEWS[id];
            return (
              <div key={id} className="page" hidden={id !== page || skill !== null}>
                <View />
              </div>
            );
          })}
          {skill !== null && (
            <div className="page">
              <DetailPage
                key={skill}
                skillKey={skill}
                backLabel={PAGES.find((p) => p.id === page)?.label ?? "Back"}
                onBack={() => setSkill(null)}
              />
            </div>
          )}
          {toast && <Toast toast={toast} onUndo={() => void undo()} onDismiss={() => show(null)} />}
        </main>
      </div>

      {dialog && (
        <ConfirmDialog
          confirm={dialog}
          onAccept={() => void acceptDialog()}
          onDismiss={dismissDialog}
        />
      )}
      {palette && <CommandPalette commands={commands} onClose={() => setPalette(false)} />}
    </AppCtx.Provider>
  );
}
