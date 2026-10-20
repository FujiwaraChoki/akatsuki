// internal.email.send (ARCHITECTURE §4.5, §14.6): the only path for transactional email.
import { v } from "convex/values";
import { internalAction } from "./_generated/server";
import { emailHash } from "./ee/service";
import { emailSenderFromEnv, renderEmail } from "./lib/email";
import { rateLimiter } from "./lib/rateLimits";

export const vEmailTemplate = v.union(
  v.literal("magic_link"),
  v.literal("otp"),
  v.literal("trial_will_end"),
  v.literal("reauth_needed"),
  v.literal("purge_warning"),
);

/** Sign-in emails anyone can trigger for any address; they are limited per recipient (D-394). */
const SIGN_IN_TEMPLATES = new Set(["magic_link", "otp"]);

export const send = internalAction({
  args: { template: vEmailTemplate, to: v.string(), vars: v.record(v.string(), v.string()) },
  returns: v.null(),
  handler: async (ctx, { template, to, vars }) => {
    if (SIGN_IN_TEMPLATES.has(template)) {
      // 5 an hour and 20 a day per address, whatever the caller's IP. Over the limit the email is dropped
      // quietly: the caller already got the same answer as for any address, so nothing is revealed.
      const key = await emailHash(to);
      for (const name of ["signInEmailHour", "signInEmailDay"] as const) {
        const r = await rateLimiter.limit(ctx, name, { key });
        if (!r.ok) {
          console.warn(JSON.stringify({ msg: "sign-in email dropped", reason: name }));
          return null;
        }
      }
    }
    const sender = emailSenderFromEnv();
    await sender.send(renderEmail(template, to, vars));
    return null;
  },
});
