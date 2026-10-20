// The local harness runs EMAIL_TRANSPORT=console, so sign-in codes are read from the deployment's function logs
// through the documented `convex logs --env-file ../.env.local` (the CLI reads the file; these tests never do).
// Only the line for the test's own address is parsed; nothing from the log is printed.
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const CONVEX_DIR = fileURLToPath(new URL("../../../../convex/", import.meta.url));

/** Waits for the console-transport email to `email` in the function logs and returns its 6-digit code. */
export function readOtp(email: string, timeoutMs = 30_000): Promise<string> {
  return new Promise((resolve, reject) => {
    const child = spawn(
      "pnpm",
      ["exec", "convex", "logs", "--history", "50", "--env-file", "../.env.local"],
      {
        cwd: CONVEX_DIR,
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    let out = "";
    const timer = setTimeout(() => {
      child.kill();
      reject(new Error("no sign-in code in the function logs"));
    }, timeoutMs);
    child.stdout.on("data", (d: Buffer) => {
      out += d.toString();
      const at = out.lastIndexOf(`to=${email}`);
      if (at === -1) return;
      const m = /\b(\d{6})\b/.exec(out.slice(at + email.length + 3));
      if (!m) return;
      clearTimeout(timer);
      child.kill();
      resolve(m[1] as string);
    });
  });
}
