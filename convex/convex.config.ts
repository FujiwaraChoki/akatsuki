// Components (ARCHITECTURE §4.5): Better Auth (default component, no Local Install, D-034) and the rate limiter.
import betterAuth from "@convex-dev/better-auth/convex.config";
import rateLimiter from "@convex-dev/rate-limiter/convex.config";
import { defineApp } from "convex/server";

const app = defineApp();
app.use(betterAuth);
app.use(rateLimiter);

export default app;
