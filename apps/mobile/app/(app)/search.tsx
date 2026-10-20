// Search (§11): the shared grammar parsed on the device for chips and errors, `search.threads` on Convex for
// headers and snippets, and "Search the full mailbox" through the worker's provider search for the rest.
// "Search with AI" (§11 NL search, feature `nl_search`) turns a sentence into the same grammar, shown as an
// editable query with the model's explanation and where the sentence went; it never runs anything else.

import { AiFeatureError, nlSearchToQuery } from "@akatsuki/ai";
import type { Id } from "@akatsuki/convex";
import { api } from "@akatsuki/convex";
import { usePaginatedQuery, useQuery } from "convex/react";
import { router } from "expo-router";
import { type ComponentRef, useEffect, useMemo, useRef, useState } from "react";
import { ScrollView, TextInput, View } from "react-native";
import { EgressLine } from "../../src/components/Egress";
import { EmptyState } from "../../src/components/EmptyState";
import { HeaderButton } from "../../src/components/Header";
import { openThread, ThreadList } from "../../src/components/ThreadList";
import { Button, Chip, NO_OUTLINE, Row, Screen, Spinner, Txt } from "../../src/components/ui";
import { aiStatusText, useAiEnv } from "../../src/lib/ai";
import { worker } from "../../src/lib/clients";
import { errorText } from "../../src/lib/errors";
import { useShortcuts } from "../../src/state/keyboard";
import { useMail } from "../../src/state/mail";
import { useTheme } from "../../src/theme/ThemeProvider";
import { displayName } from "../../src/vm/format";
import {
  insertOperator,
  OPERATOR_HINTS,
  SEARCH_DEBOUNCE_MS,
  searchModel,
  searchNow,
  unsupportedNotice,
} from "../../src/vm/search";
import type { ThreadRowLike } from "../../src/vm/threadList";

function ProviderHit({ threadId }: { threadId: string }) {
  const r = useQuery(api.mail.getThread, { threadId: threadId as Id<"threads"> });
  if (!r) return null;
  return (
    <Row
      title={r.thread.subject || "(no subject)"}
      detail={displayName(r.thread.lastFrom)}
      onPress={() => openThread(threadId)}
    />
  );
}

export default function Search() {
  const t = useTheme();
  const mail = useMail();
  const input = useRef<ComponentRef<typeof TextInput>>(null);
  const [text, setText] = useState("");
  const [query, setQuery] = useState("");
  const [nowAt, setNowAt] = useState(() => searchNow(Date.now()));
  const [provider, setProvider] = useState<{
    state: "idle" | "busy" | "done" | "error";
    ids: string[];
    error?: string;
  }>({ state: "idle", ids: [] });

  const ai = useAiEnv();
  const splits = useQuery(api.splits.list, mail.entitled ? {} : "skip");
  const [nl, setNl] = useState<{ state: "idle" | "busy" | "done" | "error"; text?: string; from?: string }>({
    state: "idle",
  });
  const askAi = async () => {
    const sentence = text.trim();
    if (!sentence) return;
    let status: Awaited<ReturnType<typeof ai.check>>;
    try {
      status = await ai.check("nl_search");
    } catch (e) {
      setNl({ state: "error", text: errorText(e) });
      return;
    }
    if (status !== "ready") {
      setNl({ state: "error", text: aiStatusText(status) });
      return;
    }
    setNl({ state: "busy" });
    try {
      const out = await nlSearchToQuery(
        {
          query: sentence,
          timezone: mail.timeZone,
          labels: [...new Set(mail.labels.filter((l) => l.labelKey.startsWith("l:")).map((l) => l.name))],
          splits: (splits ?? []).map((sp) => sp.name),
        },
        await ai.context(),
      );
      setText(out.query);
      setNl({ state: "done", text: out.explanation, from: sentence });
    } catch (e) {
      setNl({ state: "error", text: e instanceof AiFeatureError ? e.message : errorText(e) });
    }
  };

  useEffect(() => {
    const id = setTimeout(() => {
      setQuery(text);
      setNowAt(searchNow(Date.now()));
      setProvider({ state: "idle", ids: [] });
    }, SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(id);
  }, [text]);

  const model = useMemo(() => searchModel(query, nowAt, mail.timeZone), [query, nowAt, mail.timeZone]);
  const results = usePaginatedQuery(
    api.search.threads,
    mail.entitled && !model.empty && !model.tooLong ? { q: model.query, nowAt } : "skip",
    { initialNumItems: 30 },
  );

  // `unsupported` and the AST come with every page; read them from the latest page result.
  const unsupported =
    useQuery(
      api.search.threads,
      mail.entitled && !model.empty && !model.tooLong
        ? { q: model.query, nowAt, paginationOpts: { numItems: 1, cursor: null } }
        : "skip",
    )?.unsupported ?? [];

  const searchFullMailbox = async () => {
    if (!worker) return;
    setProvider({ state: "busy", ids: [] });
    try {
      const accounts = mail.accounts.filter((a) => a.caps.serverSearch);
      const all = await Promise.all(
        accounts.map((a) => worker!.search({ accountId: a._id as Id<"mailAccounts">, q: model.query })),
      );
      const known = new Set(results.results.map((r) => r._id as string));
      setProvider({
        state: "done",
        ids: [...new Set(all.flatMap((r) => r.threadIds as string[]))]
          .filter((id) => !known.has(id))
          .slice(0, 30),
      });
    } catch (err) {
      setProvider({ state: "error", ids: [], error: errorText(err) });
    }
  };

  useShortcuts("search", {
    "search.ai": () => void askAi(),
    "search.fullMailbox": () => void searchFullMailbox(),
    "search.close": () => router.back(),
    "search.submit": () => {
      setQuery(text);
      input.current?.blur();
    },
  });

  const notice = unsupportedNotice(unsupported);
  const nlEgress = ai.egress("nl_search");
  const aiRow =
    !model.empty && (nl.state !== "idle" || /\s/.test(text.trim())) ? (
      <View style={{ paddingHorizontal: t.space[2], gap: t.space["0_5"] }}>
        {nl.state === "done" ? (
          <Txt variant="meta" tone="secondary">
            {`From “${nl.from}”: ${nl.text}`}
          </Txt>
        ) : nl.state === "error" ? (
          <Txt variant="meta" tone="error">
            {nl.text}
          </Txt>
        ) : (
          <Button
            title="Search with AI"
            variant="ghost"
            compact
            busy={nl.state === "busy"}
            accessibilityLabel="Turn this sentence into a search with AI"
            onPress={() => void askAi()}
          />
        )}
        {nlEgress && nl.state !== "error" ? <EgressLine text={nlEgress} /> : null}
      </View>
    ) : null;
  const header = (
    <View style={{ gap: t.space[1], paddingBottom: t.space[1] }}>
      {aiRow}
      {model.chips.length > 0 ? (
        <ScrollView
          horizontal
          showsHorizontalScrollIndicator={false}
          contentContainerStyle={{ gap: t.space["0_5"], paddingHorizontal: t.space[2] }}
        >
          {[...new Map(model.chips.map((c) => [`${c.negated ? "-" : ""}${c.label}`, c])).entries()].map(
            ([k, c]) => (
              <Chip key={k} label={c.negated ? `not ${c.label}` : c.label} />
            ),
          )}
        </ScrollView>
      ) : null}
      {model.errors.map((e) => (
        <Txt key={e} variant="meta" tone="error" style={{ paddingHorizontal: t.space[2] }}>
          {e}
        </Txt>
      ))}
      {model.tooLong ? (
        <Txt variant="meta" tone="error" style={{ paddingHorizontal: t.space[2] }}>
          Searches are limited to 500 characters.
        </Txt>
      ) : null}
      {notice ? (
        <Txt variant="meta" tone="muted" style={{ paddingHorizontal: t.space[2] }}>
          {notice}
        </Txt>
      ) : null}
      {!model.empty && mail.accounts.some((a) => a.caps.serverSearch) ? (
        <View style={{ paddingHorizontal: t.space[2] }}>
          <Button
            title={
              provider.state === "done"
                ? `${provider.ids.length} more from the full mailbox`
                : "Search the full mailbox"
            }
            variant="ghost"
            compact
            busy={provider.state === "busy"}
            disabled={provider.state === "done"}
            onPress={searchFullMailbox}
          />
          {provider.error ? (
            <Txt variant="meta" tone="error">
              {provider.error}
            </Txt>
          ) : null}
        </View>
      ) : null}
      {provider.ids.map((id) => (
        <ProviderHit key={id} threadId={id} />
      ))}
    </View>
  );

  return (
    <Screen>
      <View
        style={{
          flexDirection: "row",
          alignItems: "center",
          gap: t.space[1],
          paddingHorizontal: t.space[2],
          paddingVertical: t.space[1],
        }}
      >
        <TextInput
          ref={input}
          accessibilityLabel="Search mail"
          value={text}
          onChangeText={(v) => {
            setText(v);
            if (nl.state !== "busy") setNl({ state: "idle" });
          }}
          onSubmitEditing={() => setQuery(text)}
          placeholder="Search: from:ana is:unread budget"
          placeholderTextColor={t.colors.muted}
          selectionColor={t.colors.dawn}
          autoFocus
          autoCapitalize="none"
          autoCorrect={false}
          returnKeyType="search"
          clearButtonMode="while-editing"
          maxLength={600}
          style={[
            t.text.reading,
            {
              flex: 1,
              minHeight: 44,
              color: t.colors.text,
              paddingHorizontal: t.space["1_5"],
              borderRadius: t.radius.md,
              backgroundColor: t.colors.surface,
            },
            NO_OUTLINE,
          ]}
        />
        <HeaderButton label="Cancel" onPress={() => router.back()} />
      </View>
      {model.empty ? (
        <View
          style={{
            flexDirection: "row",
            flexWrap: "wrap",
            gap: t.space["0_5"],
            paddingHorizontal: t.space[2],
          }}
        >
          {OPERATOR_HINTS.map((h) => (
            <Chip
              key={h.label}
              mono
              label={h.label}
              onPress={() => setText((q) => insertOperator(q, h.insert))}
            />
          ))}
        </View>
      ) : (
        <View style={{ flex: 1 }}>
          <ThreadList
            rows={results.results as ThreadRowLike[]}
            viewLabelKey="all"
            header={header}
            onEndReached={() => {
              if (results.status === "CanLoadMore") results.loadMore(30);
            }}
            empty={
              results.status === "LoadingFirstPage" ? (
                <View style={{ padding: t.space[4], alignItems: "center" }}>
                  <Spinner />
                </View>
              ) : (
                <EmptyState title="No results" body="Try fewer words, or search the full mailbox." />
              )
            }
          />
        </View>
      )}
    </Screen>
  );
}
