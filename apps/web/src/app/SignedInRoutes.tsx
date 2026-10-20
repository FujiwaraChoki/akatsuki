// Everything behind the auth gate: onboarding, the mail shell and the part-2 routes (settings, compose). Loaded
// as its own chunk by App.tsx, which also registers the part-2 features (compose, AI, reminders, senders,
// settings) before anything here renders.
import { Button, Heading } from "@akatsuki/ui";
import type { MeView } from "../backend/types.ts";
import { ErrorBoundary } from "../components/ErrorState.tsx";
import { registerFeatures } from "../features/index.ts";
import { MailApp } from "../mail/MailApp.tsx";
import { Onboarding } from "../onboarding/Onboarding.tsx";
import { FatalError } from "./App.tsx";
import { routeSlot } from "./extensions.ts";
import { navigate } from "./router.tsx";
import type { Route } from "./routes.ts";

registerFeatures();

export default function SignedInRoutes({ route, me }: { route: Route; me: MeView }) {
  if (route.name === "onboarding") return <Onboarding step={route.step} state={me.profile.onboarding} />;
  return <Routes route={route} me={me} />;
}

function Routes({ route, me }: { route: Route; me: MeView }) {
  switch (route.name) {
    case "mail":
    case "search":
      return (
        <ErrorBoundary
          resetKey={route.name}
          fallback={(e, reset) => <FatalError error={e} onRetry={reset} />}
        >
          <MailApp route={route} me={me} />
        </ErrorBoundary>
      );
    case "settings": {
      const Settings = routeSlot("settings");
      return Settings ? <Settings section={route.section} /> : <NotYet what="Settings" />;
    }
    case "compose": {
      const Compose = routeSlot("compose");
      return Compose ? <Compose /> : <NotYet what="Compose" />;
    }
    default:
      return <NotFound />;
  }
}

/** Part-2 routes before their module is registered (src/app/extensions.ts). Says so plainly. */
function NotYet({ what }: { what: string }) {
  return (
    <main className="ak-fatal">
      <Heading level={1} variant="heading">
        {what} is not available in this build yet.
      </Heading>
      <Button onClick={() => navigate("/inbox")}>Back to Inbox</Button>
    </main>
  );
}

function NotFound() {
  return (
    <main className="ak-fatal">
      <Heading level={1} variant="heading">
        Nothing lives at this address.
      </Heading>
      <Button onClick={() => navigate("/inbox")}>Go to Inbox</Button>
    </main>
  );
}
