// Modal dialogs (Radix): focus trap, Escape closes, focus returns to the trigger. Level 1 elevation on a light
// scrim (no blur). The title is Geist 15/500; pass `hideTitle` to keep it for screen readers only.
// `anchor` turns the dialog into an anchored popover (pickers: snooze, labels, send later): no scrim, placed
// under a rectangle and flipped above it near the bottom edge (SPEC §3.7).
import * as RadixDialog from "@radix-ui/react-dialog";
import { VisuallyHidden } from "@radix-ui/react-visually-hidden";
import type { CSSProperties, ReactNode } from "react";
import { cx } from "./config.tsx";

export const DialogTrigger = (props: RadixDialog.DialogTriggerProps) => (
  <RadixDialog.Trigger asChild {...props} />
);
export const DialogClose = (props: RadixDialog.DialogCloseProps) => <RadixDialog.Close asChild {...props} />;

export interface DialogProps {
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** Required for accessibility; shown as the heading unless `hideTitle`. */
  title: ReactNode;
  hideTitle?: boolean;
  description?: ReactNode;
  /** Buttons, right-aligned. */
  footer?: ReactNode;
  size?: "sm" | "md" | "lg";
  /** Rendered outside the content box (e.g. a `<DialogTrigger>`). */
  trigger?: ReactNode;
  children?: ReactNode;
  className?: string;
  /** Called when the dialog opens, to move focus somewhere other than the first focusable element. */
  onOpenAutoFocus?: (e: Event) => void;
  /** A visible Close button pinned to the top right, for long dialogs read by scrolling. */
  closeButton?: boolean;
  /** Anchors the dialog as a popover under this rectangle (viewport coordinates), with no scrim. */
  anchor?: AnchorRect | null;
  /** Popover width in px (with `anchor`). */
  width?: number;
}

export interface AnchorRect {
  top: number;
  bottom: number;
  left: number;
  right: number;
}

/** Where an anchored popover goes: under the rectangle, or above it when the space below is short. */
export function popoverPosition(
  anchor: AnchorRect,
  width: number,
  viewport: { width: number; height: number },
  minBelow = 340,
): CSSProperties {
  const left = Math.max(8, Math.min(anchor.left, viewport.width - width - 8));
  const below = viewport.height - anchor.bottom;
  return below >= minBelow || below >= anchor.top
    ? { top: Math.round(anchor.bottom + 4), bottom: "auto", left: Math.round(left), width }
    : { top: "auto", bottom: Math.round(viewport.height - anchor.top + 4), left: Math.round(left), width };
}

export function Dialog({
  open,
  defaultOpen,
  onOpenChange,
  title,
  hideTitle,
  description,
  footer,
  size = "md",
  trigger,
  children,
  className,
  onOpenAutoFocus,
  closeButton,
  anchor,
  width = 280,
}: DialogProps) {
  const heading = <RadixDialog.Title className="ak-dialog__title">{title}</RadixDialog.Title>;
  const pop = anchor
    ? popoverPosition(anchor, width, {
        width: typeof window === "undefined" ? 1440 : window.innerWidth,
        height: typeof window === "undefined" ? 900 : window.innerHeight,
      })
    : undefined;
  return (
    <RadixDialog.Root open={open} defaultOpen={defaultOpen} onOpenChange={onOpenChange}>
      {trigger}
      <RadixDialog.Portal>
        {!pop && <RadixDialog.Overlay className="ak-dialog__overlay" />}
        <RadixDialog.Content
          className={cx("ak-dialog", pop ? "ak-dialog--pop" : `ak-dialog--${size}`, className)}
          style={pop}
          onOpenAutoFocus={onOpenAutoFocus}
          // Without a description, tell Radix there is none (it warns otherwise).
          {...(description ? {} : { "aria-describedby": undefined })}
        >
          {closeButton && (
            <RadixDialog.Close className="ak-dialog__close" aria-label="Close">
              <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" focusable="false">
                <path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
              </svg>
            </RadixDialog.Close>
          )}
          {hideTitle ? <VisuallyHidden>{heading}</VisuallyHidden> : heading}
          {description ? (
            <RadixDialog.Description className="ak-dialog__description">
              {description}
            </RadixDialog.Description>
          ) : null}
          {children !== undefined && <div className="ak-dialog__body">{children}</div>}
          {footer && <div className="ak-dialog__footer">{footer}</div>}
        </RadixDialog.Content>
      </RadixDialog.Portal>
    </RadixDialog.Root>
  );
}
