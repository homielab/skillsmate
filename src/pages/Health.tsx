import { useState } from "react";
import type { FindingRow, Kind } from "../api";
import { api } from "../api";
import { useApp } from "../store";
import { Button, Chips, PageHeader, Section, Skeleton, StatTile, StatusMark } from "../ui";

type Level = "all" | "problems" | "warnings" | "tips";
const LEVEL_KIND: Record<Exclude<Level, "all">, Kind> = { problems: 2, warnings: 1, tips: 4 };

export function FindingItem({ finding, showSkill }: { finding: FindingRow; showSkill: boolean }) {
  const { act, openSkill } = useApp();
  return (
    <li className="finding">
      <StatusMark kind={finding.kind} />
      <div className="row-main">
        <div className="row-title">
          <span className="strong">{finding.title}</span>
          {showSkill && finding.skill && (
            <button type="button" className="link-pill" onClick={() => openSkill(finding.key)}>
              {finding.skill} ›
            </button>
          )}
        </div>
        <p className="muted">{finding.detail}</p>
      </div>
      {finding.fixLabel && (
        <Button onClick={() => void act(() => api.prepareFix(finding.index))}>
          {finding.fixLabel}
        </Button>
      )}
    </li>
  );
}

export function HealthPage() {
  const { library, loading, refresh, checkUpdates, checkingUpdates, updateSkill, openSkill, act } =
    useApp();
  const [level, setLevel] = useState<Level>("all");
  const h = library?.health;

  if (!h) {
    return (
      <div className="page-scroll">
        <PageHeader title="Health" busy />
        <Skeleton rows={4} />
      </div>
    );
  }

  const findings =
    level === "all" ? h.findings : h.findings.filter((f) => f.kind === LEVEL_KIND[level]);
  const levels = [
    { id: "all" as const, label: "All", count: h.findings.length },
    { id: "problems" as const, label: "Problems", count: h.problems },
    { id: "warnings" as const, label: "Warnings", count: h.warnings },
    { id: "tips" as const, label: "Tips", count: h.tips },
  ].filter((l) => l.id === "all" || l.count > 0);

  return (
    <div className="page-scroll">
      <PageHeader title="Health" subtitle={h.summary} busy={loading || checkingUpdates}>
        <Button disabled={loading} onClick={() => void refresh()}>
          Refresh
        </Button>
      </PageHeader>

      <div className="tiles">
        <StatTile label="Problems" value={h.problems} note="Broken or not loading" />
        <StatTile label="Warnings" value={h.warnings} note="Working, but not as intended" />
        <StatTile label="Tips" value={h.tips} note="Ways to improve" />
        <StatTile label="Loaded every session" value={h.startup} note={h.startupNote} />
      </div>

      <Section
        title="Updates"
        note={checkingUpdates ? "Checking…" : h.updatesStatus}
        actions={
          <Button
            variant={h.updatesStatus ? "default" : "primary"}
            busy={checkingUpdates}
            onClick={() => void checkUpdates()}
          >
            {h.updatesStatus ? "Check again" : "Check for updates"}
          </Button>
        }
      >
        {h.updates.length === 0 ? (
          <p className="muted">
            {h.updatesStatus
              ? "Every skill from skills.sh matches its published version."
              : "Compares your installed skills with the versions published on skills.sh, file by file. Sends the names of your skills.sh skills to skills.sh."}
          </p>
        ) : (
          <ul className="plain divided">
            {h.updates.map((u) => (
              <li key={u.key} className="finding">
                <StatusMark kind={u.kind} />
                <div className="row-main">
                  <div className="row-title">
                    <span className="strong ellipsis">{u.name}</span>
                    <span>{u.label}</span>
                  </div>
                  <span className="muted small ellipsis">
                    {u.source}
                    {u.summary && ` · ${u.summary}`}
                  </span>
                </div>
                <Button variant="ghost" onClick={() => openSkill(u.key)}>
                  What changed
                </Button>
                <Button variant="primary" onClick={() => updateSkill(u.key)}>
                  Update
                </Button>
              </li>
            ))}
          </ul>
        )}
      </Section>

      <Section
        title="Findings"
        actions={levels.length > 1 && <Chips value={level} onChange={setLevel} options={levels} />}
      >
        {h.brokenLinks > 1 && (
          <div className="banner">
            <span>{h.brokenLinks} links point to skills that no longer exist.</span>
            <Button onClick={() => void act(() => api.prepareFixAllLinks())}>
              Remove all {h.brokenLinks}
            </Button>
          </div>
        )}
        {h.findings.length === 0 ? (
          <p className="muted">
            Nothing to report. Your skills follow the Agent Skills spec and every agent can see
            them.
          </p>
        ) : (
          <ul className="plain divided">
            {findings.map((f) => (
              <FindingItem key={f.index} finding={f} showSkill />
            ))}
          </ul>
        )}
      </Section>
    </div>
  );
}
