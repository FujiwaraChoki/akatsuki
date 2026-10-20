import { describe, expect, it } from "vitest";
import { mailtoAddress, mailtoConfirmation } from "../src/vm/unsubscribe";

describe("mailto unsubscribe confirmation", () => {
  it("names the address and warns when it is not the sender's domain", () => {
    expect(mailtoAddress("mailto:Leave@Lists.Shop.example?subject=x")).toBe("leave@lists.shop.example");
    expect(mailtoAddress("https://x.example")).toBeNull();
    expect(mailtoConfirmation("mailto:leave@lists.shop.example", "news@shop.example")).toMatchObject({
      to: "leave@lists.shop.example",
      foreign: false,
    });
    const c = mailtoConfirmation("mailto:ceo@victim.example?subject=I%20quit", "promo@spoofed.example");
    expect(c).toMatchObject({ to: "ceo@victim.example", foreign: true });
    expect(c?.message).not.toContain("I quit");
  });
});
