// Things that sit above the pages: confirmation dialog, toast, command
// palette and the first-launch access screen of the Mac App Store build.

import { useEffect, useMemo, useRef, useState } from "react";
import { api, type Confirm, message } from "./api";
import { Button, Logo, Spinner } from "./ui";

export function ConfirmDialog({
  confirm,
  onAccept,
  onDismiss,
}: {
  confirm: Confirm;
  onAccept: () => void;
  onDismiss: () => void;
}) {
  const cancel = useRef<HTMLButtonElement>(null);
  // Focus starts on Cancel, so Enter never confirms by accident: several of
  // these actions remove things.
  useEffect(() => cancel.current?.focus(), []);
  return (
    <div className="scrim" onMouseDown={onDismiss}>
      <div
        className="dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="dialog-title"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            onDismiss();
          }
        }}
      >
        <h2 id="dialog-title">{confirm.title}</h2>
        <p className="body pre-line">{confirm.message}</p>
        <div className="dialog-actions">
          <Button ref={cancel} onClick={onDismiss}>
            Cancel
          </Button>
          <Button variant={confirm.destructive ? "danger" : "primary"} onClick={onAccept}>
            {confirm.confirm}
          </Button>
        </div>
      </div>
    </div>
  );
}

export interface ToastSpec {
  text: string;
  busy?: boolean;
  undo?: boolean;
  error?: boolean;
}

export function Toast({
  toast,
  onUndo,
  onDismiss,
}: {
  toast: ToastSpec;
  onUndo: () => void;
  onDismiss: () => void;
}) {
  return (
    <div className={`toast${toast.error ? " toast-error" : ""}`} role="status" aria-live="polite">
      {toast.busy && <Spinner size={14} />}
      {toast.error && <span className="toast-bang">!</span>}
      <span className="toast-text">{toast.text}</span>
      {toast.undo && (
        <button type="button" className="toast-btn strong" onClick={onUndo}>
          Undo
        </button>
      )}
      {!toast.busy && (
        <button type="button" className="toast-btn" aria-label="Dismiss" onClick={onDismiss}>
          ×
        </button>
      )}
    </div>
  );
}

export interface Command {
  id: string;
  label: string;
  hint: string;
  run: () => void;
}

/** ⌘K: jump to a page, a skill or an action by typing part of its name. */
export function CommandPalette({
  commands,
  onClose,
}: {
  commands: Command[];
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const list = useRef<HTMLUListElement>(null);

  const matches = useMemo(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    return commands
      .filter((c) => {
        const text = `${c.label} ${c.hint}`.toLowerCase();
        return words.every((w) => text.includes(w));
      })
      .slice(0, 50);
  }, [commands, query]);

  useEffect(() => setIndex(0), [query]);
  useEffect(() => {
    list.current?.children[index]?.scrollIntoView({ block: "nearest" });
  }, [index]);

  const run = (command: Command | undefined) => {
    if (!command) return;
    onClose();
    command.run();
  };

  return (
    <div className="scrim scrim-top" onMouseDown={onClose}>
      <div className="palette" role="dialog" aria-label="Command palette" onMouseDown={(e) => e.stopPropagation()}>
        <input
          autoFocus
          type="text"
          value={query}
          placeholder="Go to a page, skill or action…"
          aria-label="Search commands"
          spellCheck={false}
          autoCorrect="off"
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setIndex((i) => Math.min(i + 1, matches.length - 1));
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              setIndex((i) => Math.max(i - 1, 0));
            } else if (e.key === "Enter") {
              e.preventDefault();
              run(matches[index]);
            } else if (e.key === "Escape") {
              e.stopPropagation();
              onClose();
            }
          }}
        />
        <ul ref={list} role="listbox">
          {matches.map((c, i) => (
            <li
              key={c.id}
              role="option"
              aria-selected={i === index}
              className={i === index ? "on" : ""}
              onMouseMove={() => setIndex(i)}
              onClick={() => run(c)}
            >
              <span className="ellipsis">{c.label}</span>
              <span className="muted small">{c.hint}</span>
            </li>
          ))}
          {matches.length === 0 && <li className="muted">Nothing matches.</li>}
        </ul>
      </div>
    </div>
  );
}

/** Full-window first-launch screen of the sandboxed Mac App Store build: asks
 * for access to the home folder, where agents keep their skills. */
export function AccessScreen({ home, onGranted }: { home: string; onGranted: () => void }) {
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const grant = async () => {
    setBusy(true);
    try {
      await api.grantAccess();
      onGranted();
    } catch (e) {
      // An empty message means the user closed the folder picker.
      setError(message(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="access" data-tauri-drag-region>
      <div className="access-box">
        <Logo size={56} />
        <h1>Allow access to your skills</h1>
        <p className="body">
          Coding agents keep their skills in folders inside your home folder, like ~/.claude/skills
          and ~/.agents/skills. macOS needs you to choose your home folder once so SkillsMate can
          read and update them.
        </p>
        <p className="muted">
          SkillsMate only changes skill folders, and backs up before every change.
        </p>
        <Button variant="primary" busy={busy} onClick={grant}>
          Choose Home Folder…
        </Button>
        {home && <p className="muted small">Your home folder is {home}</p>}
        {error && <p className="error">{error}</p>}
      </div>
    </div>
  );
}
