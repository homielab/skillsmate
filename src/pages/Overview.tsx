import { useApp } from "../store";
import { BarList, Button, PageHeader, Section, Skeleton, StatTile, StatusMark } from "../ui";

export function OverviewPage() {
  const { library, loading, refresh, go, checkAudits, checkingAudits, audits } = useApp();
  const o = library?.overview;

  return (
    <div className="page-scroll">
      <PageHeader title="Overview" subtitle="Global skills on this computer" busy={loading}>
        <Button disabled={loading} onClick={() => void refresh()}>
          Refresh
        </Button>
      </PageHeader>

      {!o ? (
        <Skeleton rows={4} />
      ) : (
        <>
          <div className="tiles">
            <StatTile label="Skills installed" value={o.skills} note={o.skillsNote} onClick={() => go("installed")} />
            <StatTile label="Agents using skills" value={o.agents} note={o.agentsNote} onClick={() => go("agents")} />
            <StatTile label="Disk used" value={o.disk} note={o.diskNote} />
            <StatTile label="Last updated" value={o.updated} note={o.updatedNote} />
          </div>

          <div className="columns">
            <Section
              title="Needs attention"
              actions={
                <Button
                  busy={checkingAudits}
                  onClick={() => {
                    go("audits");
                    if (!audits) void checkAudits();
                  }}
                >
                  Check security audits
                </Button>
              }
            >
              <ul className="plain">
                {o.attention.map((row) => (
                  <li key={row.text} className="attention">
                    <StatusMark kind={row.kind} />
                    <span>{row.text}</span>
                  </li>
                ))}
              </ul>
              {library.health.badge > 0 && (
                <div>
                  <Button onClick={() => go("health")}>Open Health</Button>
                </div>
              )}
            </Section>

            <Section
              title="Recently updated"
              actions={<Button onClick={() => go("installed")}>View all installed</Button>}
            >
              {o.recent.length === 0 ? (
                <p className="muted">
                  Skills installed with the skills CLI show their update history here.
                </p>
              ) : (
                <ul className="plain">
                  {o.recent.map((row) => (
                    <li key={`${row.source}/${row.name}`} className="recent">
                      <div>
                        <span className="strong ellipsis">{row.name}</span>
                        <span className="muted small ellipsis">{row.source}</span>
                      </div>
                      <span className="muted small">{row.when}</span>
                    </li>
                  ))}
                </ul>
              )}
            </Section>
          </div>

          <div className="columns">
            <Section title="Skills per agent" note={o.perAgentNote}>
              <BarList rows={o.perAgent} empty="No detected agent can see your skills yet." />
            </Section>
            <Section title="Top sources" note={o.perSourceNote}>
              <BarList rows={o.perSource} empty="No skills tracked by skills.sh yet." />
            </Section>
          </div>
        </>
      )}
    </div>
  );
}
