// Copy shared by every client (DOM-free, so mobile imports it as `@akatsuki/ui/copy` like `@akatsuki/ui/errors`).
// One string per action: the same button reads the same on web, mobile and desktop (review round 2, D-409).

/** Sign-in screens (web `auth/SignIn.tsx`, mobile `app/sign-in.tsx`). */
export const AUTH_COPY = {
  sendCode: "Email me a code",
  sendLink: "Send a sign-in link instead",
  verify: "Sign in",
  resendCode: "Send a new code",
  useCodeInstead: "Use a code instead",
  otherEmail: "Use a different email",
  checkEmail: "Check your email",
} as const;
