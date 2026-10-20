// Small building blocks shared by the settings sections: a section (a sentence-case group header over a
// hairline-bordered group of rows), labelled rows, an ink switch, a segmented radio group. Token-only styles in
// src/styles/features.css. The page title (Instrument Serif, once per page) is SettingsApp's; a section that
// repeats it shows only its description, as the page's lede.
import { cx } from "@akatsuki/ui";
import { createContext, type ReactNode, useContext, useId } from "react";

/** The settings page's own title, so a section with the same name does not repeat it. */
export const SettingsPageTitle = createContext<string | null>(null);

export function Section({
  title,
  description,
  children,
  actions,
}: {
  title: string;
  description?: ReactNode;
  children: ReactNode;
  actions?: ReactNode;
}) {
  const pageTitle = useContext(SettingsPageTitle);
  const isPage = pageTitle !== null && pageTitle === title;
  return (
    <section className="ak-settings__section" aria-label={title} data-page={isPage || undefined}>
      {(!isPage || actions) && (
        <header className="ak-settings__section-head">
          {!isPage && <h2 className="ak-settings__group-head">{title}</h2>}
          {actions && <div className="ak-settings__actions">{actions}</div>}
        </header>
      )}
      {description && <p className="ak-settings__lede">{description}</p>}
      {children}
    </section>
  );
}

export function Row({
  label,
  hint,
  children,
  htmlFor,
}: {
  label: ReactNode;
  hint?: ReactNode;
  children: ReactNode;
  htmlFor?: string;
}) {
  return (
    <div className="ak-settings__row">
      <div className="ak-settings__row-text">
        {htmlFor ? (
          <label htmlFor={htmlFor} className="ak-settings__row-label">
            {label}
          </label>
        ) : (
          <span className="ak-settings__row-label">{label}</span>
        )}
        {hint && <p className="ak-settings__hint">{hint}</p>}
      </div>
      <div className="ak-settings__control">{children}</div>
    </div>
  );
}

export function Switch({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      className={cx("ak-switch", checked && "ak-switch--on")}
      onClick={() => onChange(!checked)}
    >
      <span className="ak-switch__thumb" aria-hidden="true" />
    </button>
  );
}

export function Segmented<T extends string | number>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: readonly { value: T; label: string }[];
  onChange: (v: T) => void;
  label: string;
}) {
  const name = useId();
  return (
    <div className="ak-segmented" role="radiogroup" aria-label={label}>
      {options.map((o) => (
        <label
          key={String(o.value)}
          className={cx("ak-segmented__option", o.value === value && "ak-segmented__option--on")}
        >
          <input
            type="radio"
            name={name}
            className="ak-visually-hidden"
            checked={o.value === value}
            onChange={() => onChange(o.value)}
          />
          {o.label}
        </label>
      ))}
    </div>
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return <p className="ak-settings__empty">{children}</p>;
}
