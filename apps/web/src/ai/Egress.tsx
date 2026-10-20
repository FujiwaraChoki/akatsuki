// The egress label under every AI action (§12.2): where the data goes for this feature with the user's
// current route. "Stays on this device" for loopback endpoints, "Stays on your local network (host)",
// "Sent to Akatsuki for processing, not stored", or "Leaves this device: <what>, sent to <host>".
import { Link } from "../app/router.tsx";
import { type AiFeatureId, type AiStatus, STATUS_COPY, type UseAi } from "./useAi.ts";

export function EgressNote({
  ai,
  feature,
  className,
}: {
  ai: UseAi;
  feature: AiFeatureId | "dictation";
  className?: string;
}) {
  const status: AiStatus = ai.status(feature);
  if (status === "loading") return <p className={`ak-egress ${className ?? ""}`} aria-hidden="true" />;
  if (status !== "ready")
    return (
      <p className={`ak-egress ak-egress--off ${className ?? ""}`}>
        {STATUS_COPY[status]} <Link href="/settings/ai">Open AI settings</Link>
      </p>
    );
  const text = ai.egress(feature);
  return (
    <p className={`ak-egress ${className ?? ""}`} data-egress={feature}>
      {text}
    </p>
  );
}

export { TRIAGE_LABEL, TriageChip } from "./TriageChip.tsx";
