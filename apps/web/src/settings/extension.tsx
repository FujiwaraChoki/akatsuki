// Registers settings with the shell: the `/settings/<section>` route and the onboarding checkout step, both
// loaded on demand (their own chunk), Ctrl/⌘ , and a palette entry per section.
import { Spinner } from "@akatsuki/ui";
import { lazy, Suspense } from "react";
import { registerExtension } from "../app/extensions.ts";
import { navigate } from "../app/router.tsx";
import { SETTINGS_PAGES } from "./pages.ts";

const SettingsApp = lazy(() => import("./SettingsApp.tsx"));
const Checkout = lazy(() => import("./SettingsApp.tsx").then((m) => ({ default: m.CheckoutStep })));

function Loading() {
  return (
    <div className="ak-splash" aria-busy="true">
      <Spinner size="sm" label="Loading settings" />
    </div>
  );
}

function SettingsRoute({ section }: { section?: string }) {
  return (
    <Suspense fallback={<Loading />}>
      <SettingsApp section={section} />
    </Suspense>
  );
}

function CheckoutRoute() {
  return (
    <Suspense fallback={<Loading />}>
      <Checkout />
    </Suspense>
  );
}

export function registerSettings(): void {
  registerExtension({
    id: "settings",
    routes: { settings: SettingsRoute, checkout: CheckoutRoute },
    commands: () => ({ "settings.open": () => navigate("/settings") }),
    paletteItems: () =>
      SETTINGS_PAGES.map((p) => ({
        id: `settings:${p.id}`,
        label: `Settings: ${p.label}`,
        section: "Settings",
        keywords: [...p.keywords],
        run: () => navigate(`/settings/${p.id}`),
      })),
  });
}
