// Tooltips (Radix) with an optional shortcut hint: every icon-only control names itself and its key.
import type { ShortcutId } from "@akatsuki/shortcuts";
import * as RadixTooltip from "@radix-ui/react-tooltip";
import type { ReactElement, ReactNode } from "react";
import { Kbd } from "./Kbd.tsx";

export interface TooltipProps {
  /** The tooltip text. */
  content: ReactNode;
  /** Shows this shortcut's key next to the text. */
  shortcut?: ShortcutId;
  /** One focusable element (rendered with `asChild`). */
  children: ReactElement;
  side?: "top" | "right" | "bottom" | "left";
  /** Hover delay; keyboard focus shows it at once. Default 400 ms. */
  delayMs?: number;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}

export function Tooltip({
  content,
  shortcut,
  children,
  side = "bottom",
  delayMs = 400,
  open,
  onOpenChange,
}: TooltipProps) {
  return (
    <RadixTooltip.Provider delayDuration={delayMs} skipDelayDuration={300}>
      <RadixTooltip.Root open={open} onOpenChange={onOpenChange}>
        <RadixTooltip.Trigger asChild>{children}</RadixTooltip.Trigger>
        <RadixTooltip.Portal>
          <RadixTooltip.Content className="ak-tooltip" side={side} sideOffset={6} collisionPadding={8}>
            <span className="ak-tooltip__text">{content}</span>
            {shortcut && <Kbd shortcut={shortcut} className="ak-tooltip__kbd" />}
          </RadixTooltip.Content>
        </RadixTooltip.Portal>
      </RadixTooltip.Root>
    </RadixTooltip.Provider>
  );
}
