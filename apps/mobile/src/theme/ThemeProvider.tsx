import { createContext, type ReactNode, useContext, useMemo } from "react";
import { useColorScheme } from "react-native";
import { makeTheme, resolveMode, type Theme } from "./theme";

const LIGHT = makeTheme("light");
const DARK = makeTheme("dark");
const Ctx = createContext<Theme>(LIGHT);

/** Light/dark from `prefs.theme` ("system" follows the OS). */
export function ThemeProvider({
  pref,
  children,
}: {
  pref?: "system" | "light" | "dark";
  children: ReactNode;
}) {
  const system = useColorScheme();
  const mode = resolveMode(pref, system === "dark" ? "dark" : system === "light" ? "light" : null);
  const theme = useMemo(() => (mode === "dark" ? DARK : LIGHT), [mode]);
  return <Ctx.Provider value={theme}>{children}</Ctx.Provider>;
}

export function useTheme(): Theme {
  return useContext(Ctx);
}
