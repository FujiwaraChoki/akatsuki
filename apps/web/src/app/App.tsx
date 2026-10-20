// The app root: backend and UI providers, the auth gate, onboarding, and the routes.
import type { Keymap } from "@akatsuki/shortcuts";
import { AkatsukiUiProvider, Spinner } from "@akatsuki/ui";
import { createContext, lazy, Suspense, useContext, useEffect, useRef, useState } from "react";
import { SignIn } from "../auth/SignIn.tsx";
import { BackendProvider, useBackend } from "../backend/context.tsx";
import type { Backend } from "../backend/types.ts";
import { ErrorBoundary, ErrorState } from "../components/ErrorState.tsx";
import type { BodyCache } from "../mail/bodyCache.ts";
import { navigate, useRoute } from "./router.tsx";
import type { Route } from "./routes.ts";

const loadSignedIn = () => import("./SignedInRoutes.tsx");
const SignedInRoutes = lazy(loadSignedIn);

const KeymapCtx = createContext<(k: Keymap) => void>(() => {});
const KEYMAP_KEY = "ak:keymap";

function storedKeymap(): Keymap {
  try {
    return localStorage.getItem(KEYMAP_KEY) === "gmail" ? "gmail" : "superhuman";
  } catch {
    return "superhuman";
  }
}

export function App({
  backend,
  syncOrigin,
  bodyCache,
}: {
  backend: Backend;
  syncOrigin: string | null;
  bodyCache?: BodyCache;
}) {
  const [keymap, setKeymap] = useState<Keymap>(storedKeymap);
  const update = (k: Keymap) => {
    setKeymap(k);
    try {
      localStorage.setItem(KEYMAP_KEY, k);
    } catch {
      /* ignore */
    }
  };
  return (
    <AkatsukiUiProvider keymap={keymap}>
      <KeymapCtx.Provider value={update}>
        <BackendProvider backend={backend} syncOrigin={syncOrigin} bodyCache={bodyCache}>
          <ErrorBoundary fallback={(e, reset) => <FatalError error={e} onRetry={reset} />}>
            <Gate />
          </ErrorBoundary>
        </BackendProvider>
      </KeymapCtx.Provider>
    </AkatsukiUiProvider>
  );
}

function Splash({ label = "Loading Akatsuki" }: { label?: string }) {
  return (
    <div className="ak-splash" aria-busy="true">
      <span className="ak-splash__mark" aria-hidden="true" />
      <Spinner size="sm" label={label} />
    </div>
  );
}

export function FatalError({ error, onRetry }: { error: unknown; onRetry: () => void }) {
  const backend = useBackend();
  return (
    <main className="ak-fatal">
      <ErrorState
        error={error}
        onRetry={onRetry}
        onSignIn={() => void backend.auth.signOut().finally(() => navigate("/signin", { replace: true }))}
        onUpgrade={() => navigate("/settings/billing")}
      />
    </main>
  );
}

function Gate() {
  const backend = useBackend();
  const auth = backend.useAuthState();
  const route = useRoute();
  // A magic-link or social sign-in comes back with `?ott=`; ConvexBetterAuthProvider exchanges it. Do not
  // redirect (and drop the token) while that is pending.
  const pendingOtt = new URLSearchParams(window.location.search).has("ott");

  useEffect(() => {
    if (pendingOtt) return;
    if (auth.status === "signed_out" && route.name !== "signin") navigate("/signin", { replace: true });
    if (auth.status === "signed_in" && route.name === "signin") navigate("/", { replace: true });
  }, [auth.status, route.name, pendingOtt]);

  // A returning user (session loading or present) starts fetching the mail chunk at once, in parallel with the
  // session and `me.get`; only the sign-in page goes without it.
  useEffect(() => {
    if (auth.status !== "signed_out") void loadSignedIn();
  }, [auth.status]);

  if (auth.status === "loading" || (pendingOtt && auth.status === "signed_out")) return <Splash />;
  if (auth.status === "signed_out") return <SignIn />;
  if (route.name === "signin") return <Splash />;
  return <SignedIn route={route} />;
}

function browserTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

function SignedIn({ route }: { route: Route }) {
  const backend = useBackend();
  const me = backend.useMe();
  const m = backend.useMutations();
  const setKeymap = useContext(KeymapCtx);
  const bootstrapped = useRef(false);

  // The rows normally come from the Better Auth user.onCreate trigger; me.bootstrap is the idempotent fallback.
  useEffect(() => {
    if (me === null && !bootstrapped.current) {
      bootstrapped.current = true;
      void m.bootstrap({ timezone: browserTimezone() }).catch(() => undefined);
    }
  }, [me, m]);

  // Profiles created by the trigger start in UTC; adopt this browser's zone once (dates resolve in it, §1.3).
  const tzSynced = useRef(false);
  useEffect(() => {
    if (!me || tzSynced.current) return;
    tzSynced.current = true;
    const tz = browserTimezone();
    if (me.profile.timezone === "UTC" && tz !== "UTC")
      void m.updateProfile({ timezone: tz }).catch(() => undefined);
  }, [me, m]);

  useEffect(() => {
    if (me) setKeymap(me.profile.prefs.keymap);
  }, [me, setKeymap]);

  const onboarding = me?.profile.onboarding;
  useEffect(() => {
    if (onboarding && onboarding !== "done" && route.name !== "onboarding")
      navigate("/onboarding", { replace: true });
  }, [onboarding, route.name]);

  if (me === undefined) return <Splash />;
  if (me === null) return <Splash label="Setting up your account" />;
  if (route.name !== "onboarding" && me.profile.onboarding !== "done") return <Splash />;
  // Everything behind sign-in (onboarding, the mail shell, settings, compose, AI) is one lazily loaded chunk,
  // so the sign-in page downloads only what it shows (review round 2, D-409).
  return (
    <Suspense fallback={<Splash />}>
      <SignedInRoutes route={route} me={me} />
    </Suspense>
  );
}
