import { useEffect, useRef, useState } from "react";
import { api, type CardList, message } from "../api";
import { useApp } from "../store";
import { EmptyState, PageHeader, SearchField, Skeleton, SkillCardRow, useDebouncedValue } from "../ui";

/** Delay after the last keystroke before searching. */
const SEARCH_DEBOUNCE_MS = 250;

export function DiscoverPage() {
  const { revision } = useApp();
  const [query, setQuery] = useState("");
  const settled = useDebouncedValue(query.trim(), SEARCH_DEBOUNCE_MS);
  const [results, setResults] = useState<CardList | null>(null);
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState("");
  const [recs, setRecs] = useState<CardList | null>(null);
  const fetchedRecs = useRef(false);

  // Search. A response is dropped when a newer query has replaced it. Runs
  // again when the library changes, so "Installed" marks stay right.
  useEffect(() => {
    if (settled.length < 2) {
      setResults(null);
      setSearching(false);
      setError("");
      return;
    }
    let current = true;
    setSearching(true);
    api
      .search(settled)
      .then(
        (list) => {
          if (!current) return;
          setResults(list);
          setError("");
        },
        (e) => {
          if (!current) return;
          setResults(null);
          setError(message(e));
        },
      )
      .finally(() => current && setSearching(false));
    return () => {
      current = false;
    };
  }, [settled, revision]);

  // Recommendations: downloaded once, then recomputed from what's cached
  // whenever the library changes.
  useEffect(() => {
    let current = true;
    api
      .recommendations(!fetchedRecs.current)
      .then((list) => current && setRecs(list))
      .catch(() => current && setRecs({ rows: [], status: "Couldn't reach skills.sh" }));
    fetchedRecs.current = true;
    return () => {
      current = false;
    };
  }, [revision]);

  const typing = query.trim().length >= 2;

  return (
    <div className="page-fixed">
      <PageHeader
        title="Discover"
        subtitle="Search the skills.sh directory"
        busy={searching || (!typing && !recs)}
      />
      <div className="toolbar">
        <SearchField
          value={query}
          onChange={setQuery}
          placeholder='Search skills, e.g. "react" or "pdf"'
          autoFocus
        />
      </div>

      {typing ? (
        error ? (
          <EmptyState title="Search failed" message={error} />
        ) : !results ? (
          <Skeleton />
        ) : results.rows.length === 0 ? (
          <EmptyState title="No matches" message={results.status} />
        ) : (
          <>
            <h2 className="list-title">
              Results <span className="muted small">{results.status}</span>
            </h2>
            <ul className="card list scroll">
              {results.rows.map((card) => (
                <SkillCardRow key={`${card.source}/${card.slug}`} card={card} />
              ))}
            </ul>
          </>
        )
      ) : !recs ? (
        <>
          <h2 className="list-title">
            Recommended for you <span className="muted small">Looking at what you have…</span>
          </h2>
          <Skeleton />
        </>
      ) : recs.rows.length === 0 ? (
        <EmptyState title="No recommendations yet" message={recs.status} />
      ) : (
        <>
          <h2 className="list-title">
            Recommended for you <span className="muted small">{recs.status}</span>
          </h2>
          <ul className="card list scroll">
            {recs.rows.map((card) => (
              <SkillCardRow key={`${card.source}/${card.slug}`} card={card} />
            ))}
          </ul>
        </>
      )}
    </div>
  );
}
