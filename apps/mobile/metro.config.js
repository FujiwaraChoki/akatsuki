// Metro config for the pnpm monorepo. Expo's default config already watches the workspace root and resolves
// symlinked workspace packages (`@akatsuki/*`), which ship TypeScript sources with `.ts` import extensions.
const { getDefaultConfig } = require("expo/metro-config");

const config = getDefaultConfig(__dirname);

module.exports = config;
