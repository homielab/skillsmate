import { useCallback, useEffect, useState } from "react";
import { api, message, type OfficialRow } from "../api";
import { useApp } from "../store";
import { Button, EmptyState, PageHeader, Pill, SearchField, Skeleton, useDebouncedValue } from "../ui";

export function OfficialPage() {
  const { revision, openUrl } = useApp();
  const [filter, setFilter] = useState("");
  const settled = useDebouncedValue(filter, 120);
  const [view, setView] = useState<{ rows: OfficialRow[]; status: string } | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  const load = useCallback(
    (force: boolean) => {
      let current = true;
      setLoading(true);
      api
        .official(force, settled)
        .then(
          (v) => {
            if (!current) return;
            setView(v);
            setError("");
          },
          (e) => current && setError(message(e)),
        )
        .finally(() => current && setLoading(false));
      return () => {
        current = false;
      };
    },
    [settled],
  );
  // Also when the library changes: the "installed" counts depend on it.
  useEffect(() => load(false), [load, revision]);

  return (
    <div className="page-fixed">
      <PageHeader
        title="Official"
        subtitle={error || view?.status || "Loading from skills.sh…"}
        busy={loading}
      >
        <Button disabled={loading} onClick={() => load(true)}>
          Reload
        </Button>
      </PageHeader>
      <div className="toolbar">
        <SearchField value={filter} onChange={setFilter} placeholder="Filter by company or skill" />
      </div>

      {!view ? (
        error ? (
          <EmptyState title="Couldn't load official skills" message={error}>
            <Button onClick={() => load(true)}>Try again</Button>
          </EmptyState>
        ) : (
          <Skeleton />
        )
      ) : view.rows.length === 0 ? (
        <EmptyState
          title={filter ? "No matches" : "Official skills"}
          message={
            filter
              ? "Try a different filter."
              : "Skills published by the companies that build the technology."
          }
        />
      ) : (
        <ul className="grid scroll">
          {view.rows.map((row) => (
            <li key={row.owner}>
              <button type="button" className="card tile-card" onClick={() => openUrl(row.url)}>
                <div className="row-title">
                  <span className="strong big ellipsis">{row.owner}</span>
                  {row.installed && <Pill>{row.installed}</Pill>}
                  <span className="external" aria-hidden="true">
                    ↗
                  </span>
                </div>
                <span className="muted small">{row.stats}</span>
                <div className="tags">
                  {row.top.map((name) => (
                    <span key={name} className="tag">
                      {name}
                    </span>
                  ))}
                </div>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
