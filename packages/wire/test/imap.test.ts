// IMAP autodetect helpers (§6.7 step 5, D-396, D-407): worker answer first, then the local guess.
import { describe, expect, it } from "vitest";
import { discoverImapSettings, emailDomain, guessImap, parseImapDiscovery, tlsForPort } from "../src/imap.ts";

const found = {
  imap: { host: "imap.fastmail.com", port: 993, tls: "implicit" },
  smtp: { host: "smtp.fastmail.com", port: 465, tls: "implicit" },
  username: "",
  source: "well_known",
  appPasswordRequired: true,
};

describe("emailDomain", () => {
  it("accepts public domains and refuses IP literals and local names", () => {
    expect(emailDomain(" Ana@Example.COM. ")).toBe("example.com");
    expect(emailDomain("ana@sub.example.co.uk")).toBe("sub.example.co.uk");
    for (const bad of ["ana", "@example.com", "ana@localhost", "ana@1.2.3.4", "ana@[::1]", "ana@box.local"])
      expect(emailDomain(bad)).toBeNull();
    expect(emailDomain("ana@-bad.com")).toBeNull();
    expect(emailDomain("ana@nodot")).toBeNull();
  });
});

describe("discovery", () => {
  it("validates the worker's answer", () => {
    expect(parseImapDiscovery(found)).toEqual(found);
    expect(parseImapDiscovery({ ...found, source: "other" })?.source).toBe("well_known");
    expect(parseImapDiscovery({ ...found, imap: { host: "x", port: 0, tls: "implicit" } })).toBeNull();
    expect(parseImapDiscovery({ ...found, smtp: { host: "x", port: 25, tls: "none" } })).toBeNull();
    expect(parseImapDiscovery(null)).toBeNull();
  });
  it("uses the worker, then falls back to the guess", async () => {
    expect(await discoverImapSettings("ana@fastmail.com", async () => found)).toEqual({
      ...found,
      username: "ana@fastmail.com",
    });
    const guess = await discoverImapSettings("ana@example.org", async () => {
      throw new Error("404");
    });
    expect(guess).toEqual(guessImap("ana@example.org"));
    expect(guess).toMatchObject({
      imap: { host: "imap.example.org", port: 993, tls: "implicit" },
      smtp: { host: "smtp.example.org", port: 587, tls: "starttls" },
      source: "guess",
    });
    expect(await discoverImapSettings("ana@example.org", null)).toMatchObject({ source: "guess" });
    let asked = false;
    expect(
      await discoverImapSettings("ana@127.0.0.1", async () => {
        asked = true;
        return found;
      }),
    ).toBeNull();
    expect(asked).toBe(false);
  });
  it("maps typed ports to TLS modes", () => {
    expect(tlsForPort(993, "imap")).toBe("implicit");
    expect(tlsForPort(143, "imap")).toBe("starttls");
    expect(tlsForPort(465, "smtp")).toBe("implicit");
    expect(tlsForPort(587, "smtp")).toBe("starttls");
    expect(tlsForPort(2525, "smtp")).toBe("starttls");
  });
});
