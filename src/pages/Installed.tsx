import { useMemo, useState } from "react";
import type { SkillRow } from "../api";
import { useApp } from "../store";
import { Button, Chips, EmptyState, PageHeader, Pill, SearchField, Skeleton } from "../ui";

type Filter = "all" | "updates" | "managed" | "local" | "hidden";

const FILTERS: { id: Filter; label: string; test: (s: SkillRow) => boolean }[] = [
  { id: "all", label: "All", test: () => true },
  { id: "updates", label: "Changed", test: (s) => s.update !== "" },
  { id: "managed", label: "From skills.sh", test: (s) => s.managed },
  { id: "local", label: "Local", test: (s) => !s.managed },
  { id: "hidden", label: "Not visible to agents", test: (s) => !s.visible },
];

export function InstalledPage() {
  const { library, loading, refresh, openSkill, go } = useApp();
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");

  const skills = library?.skills;
  const chips = useMemo(
    () =>
      FILTERS.map((f) => ({
        id: f.id,
        label: f.label,
        count: skills?.filter(f.test).length ?? 0,
      })).filter((c) => c.id === "all" || c.count > 0),
    [skills],
  );
  // A chip disappears when its count drops to zero; fall back to All.
  const active = chips.some((c) => c.id === filter) ? filter : "all";

  const shown = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const test = FILTERS.find((f) => f.id === active)?.test ?? (() => true);
    return (skills ?? []).filter(
      (s) =>
        test(s) &&
        (needle === "" ||
          [s.name, s.description, s.source, s.agents].some((f) =>
            f.toLowerCase().includes(needle),
          )),
    );
  }, [skills, query, active]);

  return (
    <div className="page-fixed">
      <PageHeader title="Installed" subtitle={library?.status} busy={loading}>
        <Button disabled={loading} onClick={() => void refresh()}>
          Refresh
        </Button>
      </PageHeader>

      <div className="toolbar">
        <SearchField value={query} onChange={setQuery} placeholder="Filter installed skills" />
        {chips.length > 1 && <Chips value={active} onChange={setFilter} options={chips} />}
      </div>

      {!skills ? (
        <Skeleton />
      ) : shown.length === 0 ? (
        skills.length === 0 ? (
          <EmptyState
            title="No skills installed yet"
            message="Find skills on the Discover, Official and Topics pages, or install them with the skills CLI."
          >
            <Button variant="primary" onClick={() => go("discover")}>
              Discover skills
            </Button>
          </EmptyState>
        ) : (
          <EmptyState title="No matches" message="Try a different filter.">
            <Button
              onClick={() => {
                setQuery("");
                setFilter("all");
              }}
            >
              Clear filters
            </Button>
          </EmptyState>
        )
      ) : (
        <ul className="card list scroll">
          {shown.map((s) => (
            <li key={s.key}>
              <button type="button" className="row row-link" onClick={() => openSkill(s.key)}>
                <div className="row-main">
                  <div className="row-title">
                    <span className="strong ellipsis">{s.name}</span>
                    {s.update && <Pill tone="accent">{s.update}</Pill>}
                    {!s.visible && <Pill tone="warn">Not visible</Pill>}
                  </div>
                  <p className="body clamp-2">{s.description}</p>
                  <span className="muted small ellipsis">
                    {s.visible ? s.agents : "No detected agent can see this skill"}
                  </span>
                </div>
                <div className="row-side">
                  <span className="muted small ellipsis">{s.source}</span>
                  <span className="muted small">{s.meta}</span>
                </div>
                <span className="chevron" aria-hidden="true">
                  ›
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
