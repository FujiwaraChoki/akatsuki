// Dropdown menus (Radix): roving focus, typeahead, Escape to close. Items show their shortcut, so the menu
// teaches the keys.
import type { ShortcutId } from "@akatsuki/shortcuts";
import * as RadixMenu from "@radix-ui/react-dropdown-menu";
import type { ReactNode } from "react";
import { cx } from "./config.tsx";
import { Kbd } from "./Kbd.tsx";

export const Menu = RadixMenu.Root;
export const MenuTrigger = (props: RadixMenu.DropdownMenuTriggerProps) => (
  <RadixMenu.Trigger asChild {...props} />
);
export const MenuGroup = RadixMenu.Group;

export interface MenuContentProps extends RadixMenu.DropdownMenuContentProps {
  children: ReactNode;
}

export function MenuContent({
  className,
  sideOffset = 6,
  align = "start",
  children,
  ...rest
}: MenuContentProps) {
  return (
    <RadixMenu.Portal>
      <RadixMenu.Content
        className={cx("ak-menu", className)}
        sideOffset={sideOffset}
        align={align}
        collisionPadding={8}
        loop
        {...rest}
      >
        {children}
      </RadixMenu.Content>
    </RadixMenu.Portal>
  );
}

export interface MenuItemProps extends Omit<RadixMenu.DropdownMenuItemProps, "children"> {
  children: ReactNode;
  shortcut?: ShortcutId;
  icon?: ReactNode;
  /** Destructive actions (delete forever, discard) use the accent colour. */
  destructive?: boolean;
}

export function MenuItem({ children, shortcut, icon, destructive, className, ...rest }: MenuItemProps) {
  return (
    <RadixMenu.Item
      className={cx("ak-menu__item", destructive && "ak-menu__item--destructive", className)}
      {...rest}
    >
      {icon && (
        <span className="ak-menu__icon" aria-hidden="true">
          {icon}
        </span>
      )}
      <span className="ak-menu__label">{children}</span>
      {shortcut && <Kbd shortcut={shortcut} className="ak-menu__kbd" />}
    </RadixMenu.Item>
  );
}

export interface MenuCheckboxItemProps extends Omit<RadixMenu.DropdownMenuCheckboxItemProps, "children"> {
  children: ReactNode;
}

export function MenuCheckboxItem({ children, className, ...rest }: MenuCheckboxItemProps) {
  return (
    <RadixMenu.CheckboxItem className={cx("ak-menu__item", "ak-menu__item--check", className)} {...rest}>
      <span className="ak-menu__check" aria-hidden="true">
        <RadixMenu.ItemIndicator>✓</RadixMenu.ItemIndicator>
      </span>
      <span className="ak-menu__label">{children}</span>
    </RadixMenu.CheckboxItem>
  );
}

/** A sentence-case section label. */
export function MenuLabel({ className, ...rest }: RadixMenu.DropdownMenuLabelProps) {
  return <RadixMenu.Label className={cx("ak-menu__heading", className)} {...rest} />;
}

export function MenuSeparator({ className, ...rest }: RadixMenu.DropdownMenuSeparatorProps) {
  return <RadixMenu.Separator className={cx("ak-menu__separator", className)} {...rest} />;
}
