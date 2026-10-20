// Part 2 of the web app (D-280..D-299): compose, reminders, AI, sender actions and settings plug into the
// shell through src/app/extensions.ts. main.tsx (and the tests) call `registerFeatures()` once.
import { registerAi } from "../ai/extension.tsx";
import { registerCompose } from "../compose/extension.tsx";
import { registerReminders } from "../reminders/extension.tsx";
import { registerSenders } from "../senders/extension.tsx";
import { registerSettings } from "../settings/extension.tsx";

let done = false;

export function registerFeatures(): void {
  if (done) return;
  done = true;
  registerCompose();
  registerReminders();
  registerAi();
  registerSenders();
  registerSettings();
}
