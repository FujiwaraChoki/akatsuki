// Transactional email (ARCHITECTURE §14.6): pluggable sender, SES v2 implementation, templates.
// Only Convex sends email (D-047). Values come from the Convex deployment env and are never logged.
import { AwsClient } from "aws4fetch";

export interface OutgoingEmail {
  to: string;
  subject: string;
  text: string;
  html: string;
}

export interface EmailSender {
  readonly name: string;
  send(email: OutgoingEmail): Promise<void>;
}

export class SesEmailSender implements EmailSender {
  readonly name = "ses";
  private readonly client: AwsClient;
  constructor(
    private readonly cfg: { region: string; accessKeyId: string; secretAccessKey: string; from: string },
  ) {
    this.client = new AwsClient({
      accessKeyId: cfg.accessKeyId,
      secretAccessKey: cfg.secretAccessKey,
      region: cfg.region,
      service: "ses",
    });
  }

  async send(email: OutgoingEmail): Promise<void> {
    const res = await this.client.fetch(
      `https://email.${this.cfg.region}.amazonaws.com/v2/email/outbound-emails`,
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          FromEmailAddress: this.cfg.from,
          Destination: { ToAddresses: [email.to] },
          Content: {
            Simple: {
              Subject: { Data: email.subject, Charset: "UTF-8" },
              Body: {
                Text: { Data: email.text, Charset: "UTF-8" },
                Html: { Data: email.html, Charset: "UTF-8" },
              },
            },
          },
        }),
      },
    );
    if (!res.ok) {
      // Status only: SES error bodies can echo the recipient address.
      throw new Error(`SES SendEmail failed with HTTP ${res.status}`);
    }
  }
}

/**
 * True for loopback hosts: `localhost`, a full `127.0.0.0/8` literal or `::1`. `*.localhost` names are not
 * accepted, the same rule as the AI baseUrl check (D-129).
 */
export function isLoopbackHost(hostname: string): boolean {
  const h = hostname.toLowerCase().replace(/^\[|\]$/g, "");
  return h === "localhost" || h === "::1" || /^127(?:\.(?:25[0-5]|2[0-4]\d|1?\d?\d)){3}$/.test(h);
}

/**
 * Where the console transport may run (D-350):
 * - `"reveal"`: the local harness, recognised by a loopback `CONVEX_SITE_URL` (the self-hosted backend
 *   in docs/DEV.md serves HTTP actions on http://127.0.0.1:3411). Address, link and code are logged so
 *   sign-in works offline.
 * - `"redact"`: any other deployment with the explicit `AKATSUKI_DEV_HARNESS=1` flag. The email is logged
 *   without its address, link or code, because remote logs are retained and visible to every member.
 * - `"refuse"`: everything else. `EMAIL_TRANSPORT=console` is a configuration error there.
 */
export function consoleTransportMode(
  env: Record<string, string | undefined>,
): "reveal" | "redact" | "refuse" {
  const site = env.CONVEX_SITE_URL;
  if (site) {
    try {
      if (isLoopbackHost(new URL(site).hostname)) return "reveal";
    } catch {
      // not a URL: fall through
    }
  }
  return env.AKATSUKI_DEV_HARNESS === "1" ? "redact" : "refuse";
}

const LINK_RE = /\b[a-z][a-z0-9+.-]*:\/\/\S+/gi;
const CODE_RE = /\b\d{4,}\b/g;

/** Replaces every link and every sign-in code (4+ digits) in a console line. */
export function redactForLog(text: string): string {
  return text.replace(LINK_RE, "<link redacted>").replace(CODE_RE, "<code redacted>");
}

/**
 * Logs an email instead of sending it. Only `emailSenderFromEnv` decides whether the address, link and code
 * may appear (local harness); the default is redacted.
 */
export class ConsoleEmailSender implements EmailSender {
  readonly name = "console";
  constructor(private readonly opts: { reveal: boolean } = { reveal: false }) {}
  get reveals(): boolean {
    return this.opts.reveal;
  }
  async send(email: OutgoingEmail): Promise<void> {
    if (this.opts.reveal) {
      console.info(`[email:console] to=${email.to} subject=${JSON.stringify(email.subject)}\n${email.text}`);
      return;
    }
    console.info(
      `[email:console] to=<redacted> subject=${JSON.stringify(redactForLog(email.subject))} (body withheld: links and codes are logged only on the local harness)`,
    );
  }
}

/** Drops every email (tests). */
export class NullEmailSender implements EmailSender {
  readonly name = "none";
  readonly sent: OutgoingEmail[] = [];
  async send(email: OutgoingEmail): Promise<void> {
    this.sent.push(email);
  }
}

/**
 * EMAIL_TRANSPORT selects the sender: "ses" (default), "console" (local harness only, see
 * `consoleTransportMode`) or "none". SES reads AWS_SES_REGION, AWS_SES_ACCESS_KEY_ID,
 * AWS_SES_SECRET_ACCESS_KEY and SES_FROM_ADDRESS.
 */
export function emailSenderFromEnv(env: Record<string, string | undefined> = process.env): EmailSender {
  const transport = env.EMAIL_TRANSPORT || "ses";
  if (transport === "console") {
    const mode = consoleTransportMode(env);
    if (mode === "refuse") {
      throw new Error(
        "EMAIL_TRANSPORT=console is only allowed on the local harness (loopback CONVEX_SITE_URL) or with AKATSUKI_DEV_HARNESS=1",
      );
    }
    return new ConsoleEmailSender({ reveal: mode === "reveal" });
  }
  if (transport === "none") return new NullEmailSender();
  if (transport !== "ses") throw new Error("EMAIL_TRANSPORT must be ses, console or none");
  const region = env.AWS_SES_REGION;
  const accessKeyId = env.AWS_SES_ACCESS_KEY_ID;
  const secretAccessKey = env.AWS_SES_SECRET_ACCESS_KEY;
  const from = env.SES_FROM_ADDRESS ?? "Akatsuki <no-reply@akatsuki.vc>";
  if (!region || !accessKeyId || !secretAccessKey) {
    throw new Error(
      "SES is not configured: set AWS_SES_REGION, AWS_SES_ACCESS_KEY_ID, AWS_SES_SECRET_ACCESS_KEY",
    );
  }
  return new SesEmailSender({ region, accessKeyId, secretAccessKey, from });
}

// ───────────── templates ─────────────

export type EmailTemplate = "magic_link" | "otp" | "trial_will_end" | "reauth_needed" | "purge_warning";

const escapeHtml = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

function layout(title: string, paragraphs: string[], action?: { label: string; url: string }): string {
  const button = action
    ? `<p style="margin:32px 0"><a href="${escapeHtml(action.url)}" style="background:#C1272D;color:#ffffff;padding:12px 28px;border-radius:9999px;text-decoration:none;font-family:Georgia,serif">${escapeHtml(action.label)}</a></p>`
    : "";
  return `<!doctype html><html><body style="margin:0;background:#ffffff;color:#171717;font-family:Georgia,'Times New Roman',serif">
<div style="max-width:520px;margin:0 auto;padding:40px 24px">
<p style="text-transform:uppercase;letter-spacing:0.3em;font-size:12px;color:#C1272D;margin:0 0 24px">Akatsuki</p>
<h1 style="font-weight:400;font-size:28px;margin:0 0 16px">${escapeHtml(title)}</h1>
${paragraphs.map((p) => `<p style="font-size:16px;line-height:1.75;color:rgba(23,23,23,0.8)">${p}</p>`).join("\n")}
${button}
<p style="font-size:13px;color:rgba(23,23,23,0.6)">If you did not ask for this, you can ignore this email.</p>
</div></body></html>`;
}

export function renderEmail(
  template: EmailTemplate,
  to: string,
  vars: Record<string, string>,
): OutgoingEmail {
  switch (template) {
    case "magic_link": {
      const url = vars.url ?? "";
      return {
        to,
        subject: "Your Akatsuki sign-in link",
        text: `Sign in to Akatsuki:\n\n${url}\n\nThe link works once and expires in 5 minutes.`,
        html: layout("Sign in to Akatsuki", ["The link works once and expires in 5 minutes."], {
          label: "Sign in",
          url,
        }),
      };
    }
    case "otp": {
      const otp = vars.otp ?? "";
      return {
        to,
        subject: `Your Akatsuki code: ${otp}`,
        text: `Your Akatsuki sign-in code is ${otp}. It expires in 5 minutes.`,
        html: layout("Your sign-in code", [
          `<span style="font-family:ui-monospace,monospace;font-size:28px;letter-spacing:0.3em">${escapeHtml(otp)}</span>`,
          "It expires in 5 minutes.",
        ]),
      };
    }
    case "trial_will_end": {
      const url = vars.url ?? "https://app.akatsuki.vc/billing";
      return {
        to,
        subject: "Your Akatsuki Pro trial ends soon",
        text: `Your 14-day Akatsuki Pro trial ends ${vars.endsOn ?? "soon"}. Akatsuki Pro is $20 a month. Manage billing: ${url}`,
        html: layout(
          "Your trial ends soon",
          [
            `Your 14-day Akatsuki Pro trial ends ${escapeHtml(vars.endsOn ?? "soon")}. Akatsuki Pro is $20 a month.`,
          ],
          { label: "Manage billing", url },
        ),
      };
    }
    case "reauth_needed": {
      const url = vars.url ?? "https://app.akatsuki.vc/settings/accounts";
      return {
        to,
        subject: "Reconnect your mailbox in Akatsuki",
        text: `Akatsuki can no longer reach ${vars.account ?? "one of your mailboxes"}. Reconnect it: ${url}`,
        html: layout(
          "Reconnect your mailbox",
          [
            `Akatsuki can no longer reach ${escapeHtml(vars.account ?? "one of your mailboxes")}. Your actions are kept until you reconnect.`,
          ],
          { label: "Reconnect", url },
        ),
      };
    }
    case "purge_warning": {
      const url = vars.url ?? "https://app.akatsuki.vc/billing";
      return {
        to,
        subject: "Your Akatsuki data will be deleted in 7 days",
        text: `Your Akatsuki Pro subscription ended. In 7 days we delete the synced mail metadata for your account. Your mail at your provider is not affected. Resubscribe: ${url}`,
        html: layout(
          "Your synced data will be deleted in 7 days",
          [
            "Your Akatsuki Pro subscription ended. In 7 days we delete the synced mail metadata for your account.",
            "Your mail at your provider is not affected.",
          ],
          { label: "Resubscribe", url },
        ),
      };
    }
  }
}
