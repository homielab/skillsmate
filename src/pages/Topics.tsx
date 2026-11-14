import { useCallback, useEffect, useState } from "react";
import { api, message, type SkillCard, type TopicRow } from "../api";
import { useApp } from "../store";
import { Button, EmptyState, PageHeader, Skeleton, SkillCardRow } from "../ui";

interface TopicView {
  title: string;
  description: string;
  status: string;
  rows: SkillCard[];
}

export function TopicsPage() {
  const { revision, openUrl } = useApp();
  const [topics, setTopics] = useState<TopicRow[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  /** Bumped by Reload, so the open topic is fetched again too. */
  const [reloads, setReloads] = useState(0);
  const [open, setOpen] = useState<string | null>(null);
  const [topic, setTopic] = useState<TopicView | null>(null);
  const [topicError, setTopicError] = useState("");

  const load = useCallback((force: boolean) => {
    setLoading(true);
    api
      .topics(force)
      .then(
        (rows) => {
          setTopics(rows);
          setError("");
          if (force) setReloads((n) => n + 1);
        },
        (e) => setError(message(e)),
      )
      .finally(() => setLoading(false));
  }, []);
  useEffect(() => load(false), [load]);

  // The open topic. A response for a topic the user has left is dropped.
  useEffect(() => {
    if (!open) return;
    let current = true;
    api.topic(open).then(
      (view) => {
        if (!current) return;
        setTopic(view);
        setTopicError("");
      },
      (e) => current && setTopicError(message(e)),
    );
    return () => {
      current = false;
    };
  }, [open, revision, reloads]);

  if (open) {
    const title = topic?.title ?? topics?.find((t) => t.slug === open)?.title ?? open;
    return (
      <div className="page-fixed">
        <PageHeader
          title={title}
          subtitle={topicError || topic?.status || "Loading…"}
          busy={!topic && !topicError}
          back={{
            label: "Topics",
            onClick: () => {
              setOpen(null);
              setTopic(null);
              setTopicError("");
            },
          }}
        >
          <Button variant="ghost" onClick={() => openUrl(`https://www.skills.sh/topic/${open}`)}>
            Open on skills.sh
          </Button>
        </PageHeader>
        {topic?.description && <p className="muted lead">{topic.description}</p>}
        {topicError ? (
          <EmptyState title="Couldn't load this topic" message={topicError} />
        ) : !topic ? (
          <Skeleton />
        ) : (
          <ul className="card list scroll">
            {topic.rows.map((card) => (
              <SkillCardRow key={`${card.source}/${card.slug}`} card={card} />
            ))}
          </ul>
        )}
      </div>
    );
  }

  return (
    <div className="page-fixed">
      <PageHeader
        title="Topics"
        subtitle={error || "Curated collections of skills by domain"}
        busy={loading}
      >
        <Button disabled={loading} onClick={() => load(true)}>
          Reload
        </Button>
      </PageHeader>
      {!topics ? (
        error ? (
          <EmptyState title="Couldn't load topics" message={error}>
            <Button onClick={() => load(true)}>Try again</Button>
          </EmptyState>
        ) : (
          <Skeleton />
        )
      ) : topics.length === 0 ? (
        <EmptyState title="Topics" message="Curated collections of skills by domain." />
      ) : (
        <ul className="grid scroll">
          {topics.map((t) => (
            <li key={t.slug}>
              <button type="button" className="card tile-card" onClick={() => setOpen(t.slug)}>
                <div className="row-title">
                  <span className="strong big ellipsis">{t.title}</span>
                  <span className="muted small push">{t.count}</span>
                </div>
                <p className="muted clamp-3">{t.description}</p>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
