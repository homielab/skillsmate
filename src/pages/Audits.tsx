import { useApp } from "../store";
import { Button, EmptyState, PageHeader, StatusMark } from "../ui";

export function AuditsPage() {
  const { audits, checkAudits, checkingAudits, openUrl, library } = useApp();
  const count = library?.skills.length ?? 0;
  const subtitle = checkingAudits
    ? `Checking ${count} ${count === 1 ? "skill" : "skills"}…`
    : (audits?.status ?? "Checks your installed skills against skills.sh security audits");

  return (
    <div className="page-fixed">
      <PageHeader title="Security audits" subtitle={subtitle} busy={checkingAudits}>
        <Button variant="ghost" onClick={() => openUrl("https://www.skills.sh/audits")}>
          All audits
        </Button>
        <Button
          variant={audits ? "default" : "primary"}
          disabled={checkingAudits}
          onClick={() => void checkAudits()}
        >
          {audits ? "Check again" : "Check my skills"}
        </Button>
      </PageHeader>

      {!audits || audits.rows.length === 0 ? (
        <EmptyState
          title={audits ? "No skills to audit" : "Check your skills for security issues"}
          message="skills.sh partners (Gen Agent Trust Hub, Socket, Snyk and others) audit published skills. Checking sends the names and sources of your installed skills to skills.sh."
        />
      ) : (
        <ul className="card list scroll">
          {audits.rows.map((row) => (
            <li key={`${row.source}/${row.name}`} className="row">
              <StatusMark kind={row.kind} />
              <div className="row-main">
                <div className="row-title">
                  <span className="strong ellipsis">{row.name}</span>
                  <span className="muted small ellipsis">{row.source}</span>
                </div>
                {row.detail && <p className="body pre-line">{row.detail}</p>}
              </div>
              <span className="strong">{row.label}</span>
              {row.url && (
                <Button variant="ghost" onClick={() => openUrl(row.url)}>
                  Details
                </Button>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
