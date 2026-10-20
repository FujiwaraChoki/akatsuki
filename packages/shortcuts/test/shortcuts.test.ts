import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  comboFromEvent,
  createMatcher,
  findConflicts,
  formatKeys,
  getShortcut,
  type KeyEventLike,
  keysFor,
  paletteCommands,
  parseCombo,
  parseSequence,
  type Scope,
  SHORTCUTS,
  SHORTCUTS_VERSION,
  sequenceToString,
} from "../src/index.ts";
import { rustConstName, toGpuiKeystrokes, toRust, validateShortcuts } from "../src/rust.ts";

const ev = (key: string, mods: Partial<KeyEventLike> = {}): KeyEventLike => ({
  key,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...mods,
});

describe("shortcuts.json", () => {
  it("is valid and conflict-free on every platform and keymap", () => {
    expect(() => validateShortcuts(SHORTCUTS)).not.toThrow();
    expect(findConflicts()).toEqual([]);
  });

  it("follows the Superhuman conventions the brief asks for", () => {
    const expected: Record<string, { mac: string; linux: string }> = {
      "thread.archive": { mac: "e", linux: "e" },
      "thread.snooze": { mac: "h", linux: "h" },
      "thread.reply": { mac: "r", linux: "r" },
      "thread.replyAll": { mac: "a", linux: "a" },
      "thread.forward": { mac: "f", linux: "f" },
      "compose.new": { mac: "c", linux: "c" },
      "search.open": { mac: "/", linux: "/" },
      "palette.open": { mac: "cmd+k", linux: "ctrl+k" },
      "nav.next": { mac: "j", linux: "j" },
      "nav.prev": { mac: "k", linux: "k" },
      "nav.open": { mac: "enter", linux: "enter" },
      "nav.back": { mac: "escape", linux: "escape" },
      "compose.send": { mac: "cmd+enter", linux: "ctrl+enter" },
      undo: { mac: "z", linux: "z" },
      "thread.trash": { mac: "#", linux: "#" },
      "thread.spam": { mac: "!", linux: "!" },
      "thread.toggleRead": { mac: "u", linux: "u" },
      "thread.star": { mac: "s", linux: "s" },
      "thread.label": { mac: "l", linux: "l" },
      "go.inbox": { mac: "g i", linux: "g i" },
      "split.next": { mac: "tab", linux: "tab" },
    };
    for (const [id, keys] of Object.entries(expected)) {
      expect(keysFor(id, "mac")[0], id).toBe(keys.mac);
      expect(keysFor(id, "linux")[0], id).toBe(keys.linux);
      expect(keysFor(id, "windows")[0], id).toBe(keys.linux);
    }
  });

  it("applies the gmail keymap overrides", () => {
    expect(keysFor("nav.back", "linux", "gmail")).toEqual(["escape", "u"]);
    expect(keysFor("thread.toggleRead", "linux", "gmail")).not.toContain("u");
    expect(keysFor("thread.archive", "linux", "gmail")).toEqual(["e"]);
  });

  it("has a version", () => {
    expect(SHORTCUTS_VERSION).toBe(1);
    expect(() => getShortcut("nope")).toThrow();
  });
});

describe("parsing and formatting", () => {
  it("parses combos and sequences", () => {
    expect(parseCombo("cmd+shift+L")).toEqual({ key: "l", cmd: true, ctrl: false, alt: false, shift: true });
    expect(parseCombo("#")).toEqual({ key: "#", cmd: false, ctrl: false, alt: false, shift: false });
    expect(sequenceToString(parseSequence("g  i"))).toBe("g i");
    expect(() => parseCombo("hyper+k")).toThrow(/modifier/);
    expect(() => parseCombo("ctrl+pagedown")).toThrow(/unknown key/);
  });
  it("formats per platform", () => {
    expect(formatKeys("cmd+k", "mac")).toBe("⌘K");
    expect(formatKeys("ctrl+k", "linux")).toBe("Ctrl+K");
    expect(formatKeys("cmd+shift+l", "mac")).toBe("⇧⌘L");
    expect(formatKeys("g i", "linux")).toBe("G then I");
    expect(formatKeys("escape", "windows")).toBe("Esc");
  });
  it("lists palette commands with hints", () => {
    const cmds = paletteCommands("mac");
    const archive = cmds.find((c) => c.id === "thread.archive");
    expect(archive?.hint).toBe("E");
    expect(cmds.find((c) => c.id === "palette.open")).toBeUndefined();
    expect(cmds.find((c) => c.id === "thread.openOriginal")?.hint).toBe("⇧O");
    expect(cmds.find((c) => c.id === "labels.remove")?.hint).toBeNull();
  });
});

describe("matching", () => {
  const list = new Set<Scope>(["global", "list"]);
  const compose = new Set<Scope>(["compose"]);

  it("normalizes events", () => {
    expect(comboFromEvent(ev("#", { shiftKey: true })).shift).toBe(false);
    expect(comboFromEvent(ev("J", { shiftKey: true }))).toMatchObject({ key: "j", shift: true });
    expect(comboFromEvent(ev("<", { ctrlKey: true, shiftKey: true, code: "Comma" }))).toMatchObject({
      key: ",",
      ctrl: true,
      shift: true,
    });
    expect(comboFromEvent(ev("ArrowDown")).key).toBe("down");
  });

  it("matches single keys in scope", () => {
    const m = createMatcher({ platform: "linux" });
    expect(m.handle(ev("e"), list)).toEqual({ kind: "match", id: "thread.archive" });
    expect(m.handle(ev("#", { shiftKey: true }), list)).toEqual({ kind: "match", id: "thread.trash" });
    expect(m.handle(ev("k", { ctrlKey: true }), list)).toEqual({ kind: "match", id: "palette.open" });
  });

  it("does not fire single-key shortcuts while typing", () => {
    const m = createMatcher({ platform: "linux" });
    expect(m.handle(ev("e"), compose)).toEqual({ kind: "none" });
    expect(m.handle(ev("Enter", { ctrlKey: true }), compose)).toEqual({ kind: "match", id: "compose.send" });
    expect(m.handle(ev("k", { ctrlKey: true }), compose)).toEqual({ kind: "match", id: "palette.open" });
  });

  it("matches sequences within the timeout", () => {
    let t = 0;
    const m = createMatcher({ platform: "mac", now: () => t });
    expect(m.handle(ev("g"), list)).toEqual({ kind: "pending" });
    t = 500;
    expect(m.handle(ev("i"), list)).toEqual({ kind: "match", id: "go.inbox" });
    expect(m.handle(ev("g"), list)).toEqual({ kind: "pending" });
    t = 5000;
    expect(m.handle(ev("i"), list)).toEqual({ kind: "none" });
  });

  it("uses cmd on mac and ctrl elsewhere", () => {
    const mac = createMatcher({ platform: "mac" });
    expect(mac.handle(ev("Enter", { metaKey: true }), compose)).toEqual({
      kind: "match",
      id: "compose.send",
    });
    expect(mac.handle(ev("Enter", { ctrlKey: true }), compose)).toEqual({ kind: "none" });
  });
});

describe("rust generator", () => {
  const rs = toRust(SHORTCUTS, SHORTCUTS_VERSION);
  it("emits constants", () => {
    expect(rustConstName("thread.replyAll")).toBe("THREAD_REPLY_ALL");
    expect(toGpuiKeystrokes("cmd+shift+l")).toBe("shift-cmd-l");
    expect(toGpuiKeystrokes("g i")).toBe("g i");
    expect(rs).toContain('pub const THREAD_ARCHIVE: &str = "thread.archive";');
    expect(rs).toContain('mac: &["cmd+k"]');
    expect(rs).toContain('gpui_linux: &["ctrl-k"]');
    expect(rs).toContain("scope: Scope::Compose");
  });
  it("rejects duplicates", () => {
    expect(() => validateShortcuts([...SHORTCUTS, SHORTCUTS[0]!])).toThrow(/duplicate/);
  });
  const rustc = spawnSync("rustc", ["--version"], { encoding: "utf8" });
  it.skipIf(rustc.status !== 0)("compiles with rustc", () => {
    const dir = mkdtempSync(join(tmpdir(), "ak-shortcuts-rs-"));
    try {
      const file = join(dir, "generated.rs");
      writeFileSync(file, rs);
      execFileSync(
        "rustc",
        [
          "--edition",
          "2024",
          "--crate-type",
          "lib",
          "--crate-name",
          "ak_shortcuts",
          "-o",
          join(dir, "out.rlib"),
          file,
        ],
        { stdio: "pipe" },
      );
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
