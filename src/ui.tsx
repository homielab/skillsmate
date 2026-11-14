// Small building blocks shared by every page.

import {
  type ButtonHTMLAttributes,
  type ReactNode,
  type Ref,
  type RefObject,
  useEffect,
  useRef,
  useState,
} from "react";
import type { Bar, Kind, SkillCard } from "./api";
import { useApp } from "./store";

export function Logo({ size = 28 }: { size?: number }) {
  return (
    <svg
      className="logo"
      width={size}
      height={size}
      viewBox="0 0 1024 1024"
      role="img"
      aria-label="SkillsMate"
    >
      <path
        fill="currentColor"
        d="M356.982,150.46 L667.018,150.46 C687.377,150.46 703.882,166.965 703.882,187.324 C703.882,207.684 687.377,224.188 667.018,224.188 L356.982,224.188 C336.623,224.188 320.118,207.684 320.118,187.324 C320.118,166.965 336.623,150.46 356.982,150.46 Z M260.569,275.231 L763.431,275.231 C783.791,275.231 800.295,291.735 800.295,312.095 C800.295,332.454 783.791,348.959 763.431,348.959 L260.569,348.959 C240.209,348.959 223.705,332.454 223.705,312.095 C223.705,291.735 240.209,275.231 260.569,275.231 Z M246.39,400.946 L777.61,400.946 C842.864,400.946 895.764,453.846 895.764,519.1 L895.764,755.408 C895.764,820.663 842.864,873.562 777.61,873.562 L246.39,873.562 C181.136,873.562 128.236,820.663 128.236,755.408 L128.236,519.1 C128.236,453.846 181.136,400.946 246.39,400.946 Z M512,473.729 C494.702,587.157 461.997,619.956 348.475,637.254 C461.903,654.552 494.702,687.257 512,800.779 C529.298,687.351 562.003,654.552 675.525,637.254 C562.097,619.956 529.298,587.157 512,473.729 Z"
      />
    </svg>
  );
}

export function Spinner({ size = 16 }: { size?: number }) {
  return (
    <span
      className="spinner"
      style={{ width: size, height: size }}
      role="status"
      aria-label="Loading"
    />
  );
}

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "default" | "primary" | "danger" | "ghost";
  busy?: boolean;
  ref?: Ref<HTMLButtonElement>;
}

export function Button({ variant = "default", busy, children, disabled, ...rest }: ButtonProps) {
  return (
    <button
      type="button"
      className={`btn btn-${variant}`}
      disabled={disabled || busy}
      {...rest}
    >
      {busy && <Spinner size={12} />}
      {children}
    </button>
  );
}

const MARKS: Record<Kind, { label: string; glyph: ReactNode }> = {
  0: {
    label: "OK",
    glyph: <path d="M5.5 9.2 8 11.6l4.6-4.8" fill="none" stroke="currentColor" strokeWidth="1.6" />,
  },
  1: {
    label: "Warning",
    glyph: <path d="M9 4.6v5.2M9 12.4v1" fill="none" stroke="currentColor" strokeWidth="1.9" />,
  },
  2: {
    label: "Problem",
    glyph: (
      <path d="M6.2 6.2l5.6 5.6M11.8 6.2l-5.6 5.6" fill="none" stroke="currentColor" strokeWidth="1.8" />
    ),
  },
  3: {
    label: "Not applicable",
    glyph: <path d="M5.5 9h7" fill="none" stroke="currentColor" strokeWidth="1.6" />,
  },
  4: {
    label: "Tip",
    glyph: <path d="M9 8v5M9 5v1.2" fill="none" stroke="currentColor" strokeWidth="1.7" />,
  },
};

/** Round status marker. Only problems get colour; the text beside it always
 * carries the meaning. */
export function StatusMark({ kind }: { kind: Kind }) {
  const mark = MARKS[kind] ?? MARKS[3];
  return (
    <svg className={`mark mark-${kind}`} width="18" height="18" viewBox="0 0 18 18" role="img" aria-label={mark.label}>
      <circle cx="9" cy="9" r="8.25" />
      {mark.glyph}
    </svg>
  );
}

export function Pill({ children, tone }: { children: ReactNode; tone?: "accent" | "warn" }) {
  return <span className={`pill${tone ? ` pill-${tone}` : ""}`}>{children}</span>;
}

export function PageHeader({
  title,
  subtitle,
  busy,
  back,
  children,
}: {
  title: string;
  subtitle?: string;
  busy?: boolean;
  back?: { label: string; onClick: () => void };
  children?: ReactNode;
}) {
  return (
    <header className="page-header">
      <div className="page-title">
        {back && (
          <button type="button" className="back" onClick={back.onClick}>
            ‹ {back.label}
          </button>
        )}
        <h1>
          {title}
          {busy && <Spinner />}
        </h1>
        {subtitle && <p>{subtitle}</p>}
      </div>
      <div className="page-actions">{children}</div>
    </header>
  );
}

export function Section({
  title,
  note,
  actions,
  children,
  className = "",
}: {
  title?: string;
  note?: string;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={`card section ${className}`}>
      {(title || note || actions) && (
        <div className="section-head">
          {title && <h2>{title}</h2>}
          {note && <span className="muted small">{note}</span>}
          {actions}
        </div>
      )}
      {children}
    </section>
  );
}

export function StatTile({
  label,
  value,
  note,
  onClick,
}: {
  label: string;
  value: string | number;
  note?: string;
  onClick?: () => void;
}) {
  const body = (
    <>
      <span className="tile-label">{label}</span>
      <span className="tile-value">{value}</span>
      <span className="tile-note">{note || " "}</span>
    </>
  );
  return onClick ? (
    <button type="button" className="card tile tile-link" onClick={onClick}>
      {body}
    </button>
  ) : (
    <div className="card tile">{body}</div>
  );
}

/** Ranked horizontal bars: one hue, largest first, values beside the label. */
export function BarList({ rows, empty }: { rows: Bar[]; empty: string }) {
  if (rows.length === 0) return <p className="muted">{empty}</p>;
  return (
    <ul className="bars">
      {rows.map((row) => (
        <li key={row.label}>
          <div className="bar-text">
            <span className="ellipsis">{row.label}</span>
            <span className="muted">{row.value}</span>
          </div>
          <div className="bar-track">
            <div
              className="bar-fill"
              style={{ width: `${Math.max(2, Math.min(1, row.fraction) * 100)}%` }}
            />
          </div>
        </li>
      ))}
    </ul>
  );
}

export function EmptyState({
  title,
  message,
  children,
}: {
  title: string;
  message: string;
  children?: ReactNode;
}) {
  return (
    <div className="empty">
      <h3>{title}</h3>
      <p>{message}</p>
      {children}
    </div>
  );
}

/** Grey placeholder rows shown while a list loads. */
export function Skeleton({ rows = 5 }: { rows?: number }) {
  return (
    <div className="skeleton" aria-hidden="true">
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="skeleton-row" />
      ))}
    </div>
  );
}

export function SearchField({
  value,
  onChange,
  placeholder,
  autoFocus,
  inputRef,
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  autoFocus?: boolean;
  inputRef?: RefObject<HTMLInputElement | null>;
}) {
  const own = useRef<HTMLInputElement>(null);
  const ref = inputRef ?? own;
  return (
    <div className="search">
      <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
        <circle cx="6" cy="6" r="4.5" fill="none" stroke="currentColor" strokeWidth="1.5" />
        <path d="M9.4 9.4l3.4 3.4" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
      </svg>
      <input
        ref={ref}
        type="text"
        value={value}
        placeholder={placeholder}
        aria-label={placeholder}
        autoFocus={autoFocus}
        spellCheck={false}
        autoCorrect="off"
        autoCapitalize="off"
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape" && value) {
            e.stopPropagation();
            onChange("");
          }
        }}
      />
      {value && (
        <button
          type="button"
          className="search-clear"
          aria-label="Clear"
          onClick={() => {
            onChange("");
            ref.current?.focus();
          }}
        >
          ×
        </button>
      )}
    </div>
  );
}

/** Filter chips: one choice at a time, each with its count. */
export function Chips<T extends string>({
  value,
  onChange,
  options,
}: {
  value: T;
  onChange: (value: T) => void;
  options: { id: T; label: string; count?: number }[];
}) {
  return (
    <div className="chips" role="tablist">
      {options.map((o) => (
        <button
          key={o.id}
          type="button"
          role="tab"
          aria-selected={value === o.id}
          className={`chip${value === o.id ? " chip-on" : ""}`}
          onClick={() => onChange(o.id)}
        >
          {o.label}
          {o.count !== undefined && <span className="chip-count">{o.count}</span>}
        </button>
      ))}
    </div>
  );
}

/** A skill on skills.sh, with View and Install. */
export function SkillCardRow({ card }: { card: SkillCard }) {
  const { install, installing, openUrl } = useApp();
  const id = `${card.source}/${card.slug}`;
  return (
    <li className="row">
      <div className="row-main">
        <div className="row-title">
          <span className="strong ellipsis">{card.name}</span>
          {card.installed && <Pill>Installed</Pill>}
        </div>
        {card.description && <p className="body clamp-2">{card.description}</p>}
        <span className="muted small ellipsis">
          {card.source}
          {card.installs && ` · ${card.installs}`}
        </span>
      </div>
      <div className="row-actions">
        <Button variant="ghost" onClick={() => openUrl(card.url)}>
          View
        </Button>
        {!card.installed && card.installable && (
          <Button
            variant="primary"
            busy={installing === id}
            disabled={installing !== null}
            onClick={() => install(card.source, card.slug)}
          >
            Install
          </Button>
        )}
      </div>
    </li>
  );
}

/** `value`, once it has stopped changing for `delay` ms. */
export function useDebouncedValue<T>(value: T, delay: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setSettled(value), delay);
    return () => clearTimeout(timer);
  }, [value, delay]);
  return settled;
}
