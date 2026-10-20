// Each variable is read with a literal `process.env.EXPO_PUBLIC_*` expression: Expo inlines exactly those at
// build time (a dynamic lookup would be undefined in the bundle).
import { readEnv } from "./vm/env.ts";

export const ENV = readEnv({
  EXPO_PUBLIC_CONVEX_URL: process.env.EXPO_PUBLIC_CONVEX_URL,
  EXPO_PUBLIC_CONVEX_SITE_URL: process.env.EXPO_PUBLIC_CONVEX_SITE_URL,
  EXPO_PUBLIC_SYNC_URL: process.env.EXPO_PUBLIC_SYNC_URL,
  EXPO_PUBLIC_API_URL: process.env.EXPO_PUBLIC_API_URL,
  EXPO_PUBLIC_LOGIN_PROVIDERS: process.env.EXPO_PUBLIC_LOGIN_PROVIDERS,
});
