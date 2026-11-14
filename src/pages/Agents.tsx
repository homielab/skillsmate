import { useMemo, useState } from "react";
import { useApp } from "../store";
import { Chips, EmptyState, PageHeader, SearchField, Skeleton } from "../ui";

type Show = "detected" | "all";

export function AgentsPage() {
  const { library, loading } = useApp();
  const [query, setQuery] = useState("");
  const [show, setShow] = useState<Show>("all");
  const agents = library?.agents;
  const detected = library?.detectedAgents ?? 0;

  const shown = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return (agents ?? []).filter(
      (a) =>
        (show === "all" || a.detected) &&
        (needle === "" ||
          [a.name, a.id, a.globalDir].some((f) => f.toLowerCase().includes(needle))),
    );
  }, [agents, query, show]);

  return (
    <div className="page-fixed">
      <PageHeader
        title="Agents"
        subtitle={
          agents
            ? `${detected} of ${agents.length} supported agents detected on this computer`
            : undefined
        }
        busy={loading}
      />
      <div className="toolbar">
        <SearchField value={query} onChange={setQuery} placeholder="Filter agents" />
        <Chips
          value={show}
          onChange={setShow}
          options={[
            { id: "all", label: "All", count: agents?.length ?? 0 },
            { id: "detected", label: "Detected", count: detected },
          ]}
        />
      </div>
      {!agents ? (
        <Skeleton />
      ) : shown.length === 0 ? (
        <EmptyState
          title="No matches"
          message={
            show === "detected" && detected === 0
              ? "No coding agents were detected on this computer."
              : "Try a different filter."
          }
        />
      ) : (
        <ul className="card list scroll">
          {shown.map((a) => (
            <li key={a.id} className={`row agent${a.detected ? "" : " agent-off"}`}>
              <span className={`dot${a.detected ? " dot-on" : ""}`} title={a.detected ? "Detected" : "Not detected"} />
              <div className="row-main">
                <div className="row-title">
                  <span className={a.detected ? "strong" : ""}>{a.name}</span>
                  {a.universal && <span className="muted small">reads .agents/skills</span>}
                </div>
                <span className="muted small ellipsis mono">{a.globalDir}</span>
              </div>
              <span className="strong small">{a.skills}</span>
              <span className="muted small agent-id">{a.id}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
