// Billing (ARCHITECTURE §13): Akatsuki Pro, $20 a month, a 14-day trial with the card taken up front, one trial
// per person, no free hosted tier. Checkout and the customer portal are Stripe pages created by apps/api
// (`POST /v1/billing/checkout`, `/portal`); the entitlement updates live from Convex once Stripe's webhook lands.
// Self-hosted servers run without billing. The onboarding step for new hosted users is `CheckoutStep`.
import { Button, Eyebrow, errorMessage, Heading, Orbs, useToast } from "@akatsuki/ui";
import { useEffect, useState } from "react";
import { useBackend } from "../../backend/context.tsx";
import type { BillingInfo, MeView } from "../../backend/types.ts";
import { codeOf } from "../../components/ErrorState.tsx";
import { fullDate } from "../../lib/format.ts";
import { Row, Section } from "../ui.tsx";

export const PRICE_COPY = "Akatsuki Pro is $20 a month after a 14-day free trial.";
const CHECKOUT_FLAG = "ak:checkout-started";
const money = (micros: number) => `$${(micros / 1_000_000).toFixed(2)}`;

/** Starts Stripe Checkout (or the portal) and leaves the app; resolves only on failure. */
function useBillingActions() {
  const backend = useBackend();
  const { toast } = useToast();
  const [busy, setBusy] = useState<null | "checkout" | "portal">(null);
  const go = async (kind: "checkout" | "portal") => {
    setBusy(kind);
    try {
      const { url } = kind === "checkout" ? await backend.api.checkout() : await backend.api.portal();
      if (kind === "checkout")
        try {
          sessionStorage.setItem(CHECKOUT_FLAG, String(Date.now()));
        } catch {
          /* ignore */
        }
      window.location.assign(url);
    } catch (e) {
      const code = codeOf(e);
      toast({
        title:
          code === "already_subscribed"
            ? "You already have a subscription. Use Manage billing."
            : `Couldn't open billing: ${errorMessage(e)}`,
        tone: "error",
      });
    } finally {
      setBusy(null);
    }
  };
  return { go, busy, configured: backend.api.configured };
}

export function describePlan(info: BillingInfo, tz: string): { title: string; detail: string } {
  const { entitlement: e, billing: b } = info;
  switch (e.state) {
    case "selfhost":
      return { title: "Self-hosted", detail: "This server runs without billing. Everything is available." };
    case "trialing":
      return {
        title: "Pro trial",
        detail: b.trialEnd
          ? `Your trial ends ${fullDate(b.trialEnd, tz)}. ${b.cancelAtPeriodEnd ? "It will not renew." : "Then $20 a month."}`
          : "Your trial is running.",
      };
    case "active":
      return {
        title: "Pro",
        detail: b.currentPeriodEnd
          ? b.cancelAtPeriodEnd
            ? `Ends ${fullDate(b.currentPeriodEnd, tz)}. It will not renew.`
            : `Renews ${fullDate(b.currentPeriodEnd, tz)} at $20.`
          : "$20 a month.",
      };
    case "grace":
      return {
        title: "Payment failed",
        detail: "Update your card in Manage billing. Akatsuki keeps working for 7 days.",
      };
    case "lapsed":
      return {
        title: "Subscription ended",
        detail:
          "Syncing is paused. Your mail stays with your provider; Akatsuki deletes its copy of your metadata 30 days after the end.",
      };
    default:
      return { title: "No plan", detail: PRICE_COPY };
  }
}

export function BillingSection({ me }: { me: MeView }) {
  const backend = useBackend();
  const info = backend.useBilling();
  const { go, busy, configured } = useBillingActions();
  const [returned] = useState(() => new URLSearchParams(window.location.search).has("session_id"));
  if (!info) return null;
  const plan = describePlan(info, me.profile.timezone);
  const subscribed =
    info.entitlement.state === "trialing" ||
    info.entitlement.state === "active" ||
    info.entitlement.state === "grace";
  const selfhost = info.entitlement.state === "selfhost";
  return (
    <Section
      title="Billing"
      description={selfhost ? undefined : `${PRICE_COPY} Card required for the trial; cancel any time.`}
    >
      {returned && !subscribed && (
        <p className="ak-banner" role="status">
          Thanks. Stripe is confirming your subscription; this page updates by itself.
        </p>
      )}
      <div className="ak-settings__card ak-plan">
        <Eyebrow>Plan</Eyebrow>
        <Heading level={3} variant="heading">
          {plan.title}
        </Heading>
        <p className="ak-settings__hint">{plan.detail}</p>
        {!selfhost && (
          <div className="ak-settings__actions">
            {subscribed ? (
              <Button loading={busy === "portal"} disabled={!configured} onClick={() => void go("portal")}>
                Manage billing
              </Button>
            ) : (
              <Button
                variant="primary"
                loading={busy === "checkout"}
                disabled={!configured}
                onClick={() => void go("checkout")}
              >
                {info.billing.trialUsedAt ? "Subscribe for $20 a month" : "Start 14-day free trial"}
              </Button>
            )}
            {!configured && (
              <span className="ak-settings__hint">Billing is not configured in this build.</span>
            )}
          </div>
        )}
      </div>
      {info.entitlement.managedAi && info.aiUsage.limitMicros > 0 && (
        <Row
          label="Akatsuki AI this month"
          hint={
            info.entitlement.state === "trialing"
              ? "The trial includes $1 of managed AI."
              : "Pro includes $8 of managed AI each month."
          }
        >
          <meter
            className="ak-meter"
            min={0}
            max={info.aiUsage.limitMicros}
            value={info.aiUsage.usedMicros}
            aria-label="Managed AI used this month"
          />
          <span className="ak-settings__hint">
            {money(info.aiUsage.usedMicros)} of {money(info.aiUsage.limitMicros)}
          </span>
        </Row>
      )}
    </Section>
  );
}

/** Onboarding while `profile.onboarding === "checkout"` (§13.1): new hosted users start their trial first. */
export function CheckoutStep() {
  const backend = useBackend();
  const me = backend.useMe();
  const { go, busy, configured } = useBillingActions();
  const [waiting] = useState(() => {
    try {
      const at = Number(sessionStorage.getItem(CHECKOUT_FLAG));
      return Number.isFinite(at) && Date.now() - at < 30 * 60_000;
    } catch {
      return false;
    }
  });
  useEffect(() => {
    if (me && me.profile.onboarding !== "checkout")
      try {
        sessionStorage.removeItem(CHECKOUT_FLAG);
      } catch {
        /* ignore */
      }
  }, [me]);
  return (
    <main className="ak-onboarding">
      <Orbs variant="hero" />
      <div className="ak-onboarding__inner">
        <Eyebrow>Akatsuki Pro</Eyebrow>
        <Heading level={1} variant="display" className="ak-onboarding__title">
          {waiting ? "Almost there." : "Start your trial."}
        </Heading>
        <p className="ak-onboarding__lede">
          {waiting
            ? "Stripe is confirming your trial. This page moves on by itself in a few seconds."
            : `${PRICE_COPY} Your card is needed up front and you can cancel any time before the trial ends.`}
        </p>
        <ul className="ak-plan__features">
          <li>Gmail, Outlook and IMAP in one keyboard-first inbox</li>
          <li>Split inbox, snooze, reminders, undo send and send later</li>
          <li>Akatsuki AI included, or bring your own key or local model</li>
        </ul>
        <div className="ak-auth__actions">
          <Button
            variant="primary"
            size="lg"
            loading={busy === "checkout"}
            disabled={!configured}
            onClick={() => void go("checkout")}
          >
            {waiting ? "Open checkout again" : "Start 14-day free trial"}
          </Button>
          <Button variant="ghost" onClick={() => void backend.auth.signOut()}>
            Sign out
          </Button>
        </div>
        {!configured && <p className="ak-onboarding__error">Billing is not configured in this build.</p>}
      </div>
    </main>
  );
}
