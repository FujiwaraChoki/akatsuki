import { Redirect } from "expo-router";
import { View } from "react-native";
import { Spinner } from "../src/components/ui";
import { authClient } from "../src/lib/auth";
import { useTheme } from "../src/theme/ThemeProvider";

export default function Index() {
  const t = useTheme();
  const session = authClient!.useSession();
  if (session.isPending)
    return (
      <View
        style={{
          flex: 1,
          alignItems: "center",
          justifyContent: "center",
          backgroundColor: t.colors.background,
        }}
      >
        <Spinner />
      </View>
    );
  return <Redirect href={session.data ? "/inbox" : "/sign-in"} />;
}
