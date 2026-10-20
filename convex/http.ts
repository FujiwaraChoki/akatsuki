// HTTP routes (ARCHITECTURE §4.5): Better Auth, mounted at /api/auth/*, and the mobile sign-in handoff
// (/mobile/auth/*, D-408). No mail or money traffic touches Convex HTTP.
import { httpRouter } from "convex/server";
import { authComponent, createAuth } from "./auth";
import { handoff, redeem } from "./mobileAuth";

const http = httpRouter();
authComponent.registerRoutes(http, createAuth, { cors: true });
http.route({ path: "/mobile/auth/handoff", method: "GET", handler: handoff });
http.route({ path: "/mobile/auth/redeem", method: "POST", handler: redeem });

export default http;
