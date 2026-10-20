// Buttons: flat pills (First Light, D-483). `primary` is ink (the everyday primary: Send, Save, Turn on);
// `brand` is the accent fill, used in exactly three places (sign-in, onboarding, the billing upgrade). A `shortcut`
// renders its key hint and sets `aria-keyshortcuts`.
import type { ShortcutId } from "@akatsuki/shortcuts";
import type { ButtonHTMLAttributes, ReactNode, Ref } from "react";
import { cx, useAriaKeyShortcuts } from "./config.tsx";
import { Kbd } from "./Kbd.tsx";
import { Spinner } from "./Spinner.tsx";
import { Tooltip } from "./Tooltip.tsx";

export type ButtonVariant = "primary" | "ink" | "brand" | "secondary" | "ghost" | "danger";
export type ButtonSize = "sm" | "md" | "lg";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** Shows a spinner, sets `aria-busy` and disables the button. */
  loading?: boolean;
  /** Shows this shortcut's key inside the button. */
  shortcut?: ShortcutId;
  /** An icon before the label. */
  icon?: ReactNode;
  ref?: Ref<HTMLButtonElement>;
}

export function Button({
  variant = "secondary",
  size = "md",
  loading = false,
  shortcut,
  icon,
  className,
  children,
  disabled,
  type = "button",
  ref,
  ...rest
}: ButtonProps) {
  const aria = useAriaKeyShortcuts(shortcut);
  return (
    <button
      ref={ref}
      type={type}
      className={cx("ak-button", `ak-button--${variant}`, `ak-button--${size}`, className)}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      aria-keyshortcuts={aria}
      {...rest}
    >
      {loading ? (
        <Spinner size="sm" label="Working" />
      ) : (
        icon && <span className="ak-button__icon">{icon}</span>
      )}
      {children !== undefined && <span className="ak-button__label">{children}</span>}
      {shortcut && <Kbd shortcut={shortcut} className="ak-button__kbd" />}
    </button>
  );
}

export interface IconButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> {
  /** Required: the accessible name, also shown as the tooltip. */
  label: string;
  icon: ReactNode;
  shortcut?: ShortcutId;
  variant?: Exclude<ButtonVariant, "primary" | "ink" | "brand">;
  size?: ButtonSize;
  /** Set false to drop the tooltip (e.g. inside a toolbar that shows labels). */
  tooltip?: boolean;
  pressed?: boolean;
  ref?: Ref<HTMLButtonElement>;
}

/** An icon-only button. Always labelled; the tooltip shows the label and the shortcut. */
export function IconButton({
  label,
  icon,
  shortcut,
  variant = "ghost",
  size = "md",
  tooltip = true,
  pressed,
  className,
  type = "button",
  ref,
  ...rest
}: IconButtonProps) {
  const aria = useAriaKeyShortcuts(shortcut);
  const button = (
    <button
      ref={ref}
      type={type}
      aria-label={label}
      aria-pressed={pressed}
      aria-keyshortcuts={aria}
      className={cx("ak-icon-button", `ak-button--${variant}`, `ak-icon-button--${size}`, className)}
      {...rest}
    >
      <span className="ak-icon-button__icon" aria-hidden="true">
        {icon}
      </span>
    </button>
  );
  return tooltip ? (
    <Tooltip content={label} shortcut={shortcut}>
      {button}
    </Tooltip>
  ) : (
    button
  );
}
