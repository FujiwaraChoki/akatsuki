// Modal dialogs (Radix): focus trap, Escape closes, focus returns to the trigger. The title is set in
// Instrument Serif; pass `hideTitle` to keep it for screen readers only.
import * as RadixDialog from "@radix-ui/react-dialog";
import { VisuallyHidden } from "@radix-ui/react-visually-hidden";
import type { ReactNode } from "react";
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
}: DialogProps) {
  const heading = <RadixDialog.Title className="ak-dialog__title">{title}</RadixDialog.Title>;
  return (
    <RadixDialog.Root open={open} defaultOpen={defaultOpen} onOpenChange={onOpenChange}>
      {trigger}
      <RadixDialog.Portal>
        <RadixDialog.Overlay className="ak-dialog__overlay" />
        <RadixDialog.Content
          className={cx("ak-dialog", `ak-dialog--${size}`, className)}
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
