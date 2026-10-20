// Error copy for the mobile app: the shared table in @akatsuki/ui/errors (§15), except that the app never points
// at buying anything (§6.4, §13.1): `not_entitled` and `ai_budget_exhausted` get neutral wording here.
import { errorCopy } from "@akatsuki/ui/errors";
import { toErrorPayload } from "@akatsuki/wire/errors";

const MOBILE_OVERRIDES: Record<string, string> = {
  not_entitled: "This account has no active Akatsuki Pro plan.",
  ai_budget_exhausted: "This month's Akatsuki AI allowance is used up. It resets at the start of next month.",
  unreachable: "No connection. It will be retried when you are back online.",
};

export function errorCode(err: unknown): string {
  return toErrorPayload(err).code;
}

export function errorText(err: unknown): string {
  const code = errorCode(err);
  return MOBILE_OVERRIDES[code] ?? errorCopy(code).title;
}

export { describeOpFailure } from "@akatsuki/ui/errors";
