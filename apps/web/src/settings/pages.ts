// The settings sections (ids, titles, palette keywords). Kept apart from the pages themselves so the palette
// and the shortcuts can list them without loading the settings chunk.
export const SETTINGS_PAGES = [
  { id: "accounts", label: "Accounts", keywords: ["mailbox", "gmail", "outlook", "imap"] },
  {
    id: "identities",
    label: "Identities and signatures",
    keywords: ["signature", "alias", "send as", "from"],
  },
  { id: "mail", label: "Mail", keywords: ["undo send", "images", "read receipts", "cache"] },
  { id: "appearance", label: "Appearance", keywords: ["theme", "dark", "density"] },
  { id: "shortcuts", label: "Shortcuts", keywords: ["keymap", "gmail keys", "keyboard"] },
  { id: "notifications", label: "Notifications", keywords: ["push"] },
  { id: "splits", label: "Split inbox", keywords: ["tabs", "vip"] },
  { id: "rules", label: "Rules", keywords: ["filters", "automation"] },
  {
    id: "labels",
    label: "Labels",
    keywords: ["categories", "keywords", "rename label", "delete label", "folders"],
  },
  { id: "snippets", label: "Snippets", keywords: ["templates"] },
  { id: "senders", label: "Senders", keywords: ["block", "vip", "images"] },
  { id: "ai", label: "AI", keywords: ["provider", "model", "ollama", "key", "openrouter", "anthropic"] },
  { id: "billing", label: "Billing", keywords: ["plan", "pro", "trial", "subscription"] },
  { id: "devices", label: "Devices and sessions", keywords: ["sign out", "browser"] },
  { id: "data", label: "Your data", keywords: ["export", "delete account"] },
] as const;

export type SettingsPageId = (typeof SETTINGS_PAGES)[number]["id"];
