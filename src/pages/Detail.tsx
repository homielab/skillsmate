import { useEffect, useState } from "react";
import { api, type Detail, message } from "../api";
import { useApp } from "../store";
import { Button, Chips, PageHeader, Pill, Section, Skeleton, StatTile } from "../ui";
import { FindingItem } from "./Health";

type Tab = "instructions" | "changes" | "files";

const DIFF_CLASS = ["diff-file", "diff-add", "diff-del", "diff-ctx", "diff-note"];
const DIFF_PREFIX = ["", "+ ", "− ", "  ", ""];

function Markdown({ blocks }: { blocks: Detail["body"] }) {
  if (blocks.length === 0) return <p className="muted">No instructions.</p>;
  return (
    <div className="markdown">
      {blocks.map((b, i) => {
        switch (b.kind) {
          case 0:
            return <h3 key={i} className="md-h1">{b.text}</h3>;
          case 1:
            return <h4 key={i} className="md-h2">{b.text}</h4>;
          case 2:
            return <h5 key={i} className="md-h3">{b.text}</h5>;
          case 4:
            return (
              <p key={i} className="md-bullet" style={{ marginLeft: b.depth * 16 }}>
                {b.text}
              </p>
            );
          case 5:
            return <pre key={i}>{b.text}</pre>;
          case 6:
            return <blockquote key={i}>{b.text}</blockquote>;
          default:
            return <p key={i}>{b.text}</p>;
        }
      })}
    </div>
  );
}

function Editor({
  skillKey,
  onDone,
}: {
  skillKey: string;
  onDone: (saved: string | null) => void;
}) {
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    api.readSkillMd(skillKey).then(setText, (e) => setError(message(e)));
  }, [skillKey]);

  const save = async () => {
    if (text === null) return;
    setSaving(true);
    try {
      onDone(await api.saveSkillMd(skillKey, text));
    } catch (e) {
      setError(message(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Section title="Edit SKILL.md" note="Your current version is backed up when you save">
      {text === null && !error ? (
        <Skeleton rows={3} />
      ) : (
        <textarea
          className="editor"
          value={text ?? ""}
          spellCheck={false}
          autoFocus
          aria-label="SKILL.md"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if ((e.metaKey || e.ctrlKey) && e.key === "s") {
              e.preventDefault();
              void save();
            } else if (e.key === "Escape") {
              // Escape would leave the page and lose the edit.
              e.stopPropagation();
              e.preventDefault();
            }
          }}
        />
      )}
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      <div className="dialog-actions">
        <Button onClick={() => onDone(null)}>Cancel</Button>
        <Button variant="primary" busy={saving} disabled={text === null} onClick={() => void save()}>
          Save
        </Button>
      </div>
    </Section>
  );
}

export function DetailPage({
  skillKey,
  backLabel,
  onBack,
}: {
  skillKey: string;
  backLabel: string;
  onBack: () => void;
}) {
  const { revision, act, updateSkill, checkUpdates, checkingUpdates, openUrl, notify, refresh } =
    useApp();
  const [d, setDetail] = useState<Detail | null>(null);
  const [tab, setTab] = useState<Tab>("instructions");
  const [editing, setEditing] = useState(false);

  // Rebuilt after every scan or update check. Goes back if the skill is gone.
  useEffect(() => {
    let current = true;
    api.detail(skillKey).then(
      (detail) => {
        if (!current) return;
        if (detail) setDetail(detail);
        else onBack();
      },
      (e) => current && notify(message(e), "error"),
    );
    return () => {
      current = false;
    };
    // `onBack` and `notify` are stable in effect; only the data matters here.
  }, [skillKey, revision]);

  const back = { label: backLabel, onClick: onBack };
  if (!d) {
    return (
      <div className="page-scroll">
        <PageHeader title="" back={back} busy />
        <Skeleton rows={4} />
      </div>
    );
  }

  const hasChanges = d.diff.length > 0;
  const tabs = [
    { id: "instructions" as const, label: "SKILL.md" },
    ...(hasChanges ? [{ id: "changes" as const, label: "What changed" }] : []),
    { id: "files" as const, label: "Files", count: d.files.length },
  ];
  const active = tab === "changes" && !hasChanges ? "instructions" : tab;

  return (
    <div className="page-scroll">
      <PageHeader title={d.name} back={back}>
        {d.canUpdate && (
          <Button variant="primary" onClick={() => updateSkill(d.key)}>
            Update
          </Button>
        )}
        {d.managed && (
          <Button busy={checkingUpdates} onClick={() => void checkUpdates()}>
            {checkingUpdates ? "Checking…" : "Check for updates"}
          </Button>
        )}
        {!editing && <Button onClick={() => setEditing(true)}>Edit SKILL.md</Button>}
        {d.url && (
          <Button variant="ghost" onClick={() => openUrl(d.url)}>
            View on skills.sh
          </Button>
        )}
        <Button variant="danger" onClick={() => void act(() => api.prepareRemove(d.key))}>
          Remove…
        </Button>
      </PageHeader>

      <div className="detail-lead">
        <div className="row-title">
          {d.version && <Pill>v{d.version}</Pill>}
          <Pill>{d.source}</Pill>
          {d.updateLabel === "Update available" && <Pill tone="accent">{d.updateLabel}</Pill>}
        </div>
        <p className="body">{d.description}</p>
      </div>

      <div className="tiles">
        <StatTile label="Size" value={d.size} note={d.filesCount} />
        <StatTile label="When active" value={d.activeTokens} note="Tokens, loaded on use" />
        <StatTile label="Every session" value={d.startupTokens} note="Tokens, name and description" />
        <StatTile label="Version" value={d.updateLabel} note={d.updateSummary} />
      </div>

      {d.findings.length > 0 && (
        <Section title="Health">
          <ul className="plain divided">
            {d.findings.map((f) => (
              <FindingItem key={f.index} finding={f} showSkill={false} />
            ))}
          </ul>
        </Section>
      )}

      <div className="detail-grid">
        <div className="detail-main">
          {editing ? (
            <Editor
              skillKey={d.key}
              onDone={(saved) => {
                setEditing(false);
                if (saved) {
                  notify(saved, "undo");
                  void refresh();
                }
              }}
            />
          ) : (
            <Section actions={<Chips value={active} onChange={setTab} options={tabs} />} className="tabbed">
              {active === "instructions" && <Markdown blocks={d.body} />}
              {active === "changes" && (
                <>
                  <p className="muted small">On skills.sh: {d.updateSummary}</p>
                  <div className="diff">
                    {d.diff.map((row, i) => (
                      <div key={i} className={DIFF_CLASS[row.kind] ?? "diff-ctx"}>
                        {DIFF_PREFIX[row.kind] ?? ""}
                        {row.text}
                      </div>
                    ))}
                  </div>
                </>
              )}
              {active === "files" && (
                <ul className="plain files">
                  {d.files.map((f) => (
                    <li key={f.path}>
                      <span className="mono ellipsis">{f.path}</span>
                      <span className="muted small">{f.size}</span>
                    </li>
                  ))}
                </ul>
              )}
            </Section>
          )}
        </div>

        <aside className="detail-side">
          <Section title="Used by">
            {d.agents.length === 0 ? (
              <p className="muted">No detected agent can see this skill.</p>
            ) : (
              <ul className="plain kv">
                {d.agents.map((a) => (
                  <li key={`${a.name}/${a.how}`}>
                    <span>{a.name}</span>
                    <span className="muted small">{a.how}</span>
                  </li>
                ))}
              </ul>
            )}
          </Section>
          <Section title="Details">
            <dl className="facts">
              <dt>Location</dt>
              <dd className="mono">{d.location}</dd>
              {d.license && (
                <>
                  <dt>License</dt>
                  <dd>{d.license}</dd>
                </>
              )}
              {d.compatibility && (
                <>
                  <dt>Requires</dt>
                  <dd>{d.compatibility}</dd>
                </>
              )}
              {d.allowedTools && (
                <>
                  <dt>Pre-approved tools</dt>
                  <dd>{d.allowedTools}</dd>
                </>
              )}
            </dl>
          </Section>
        </aside>
      </div>
    </div>
  );
}
