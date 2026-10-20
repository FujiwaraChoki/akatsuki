import type { ReactNode } from "react";
import { Footer } from "./Footer";
import { Header } from "./Header";

export function Shell({
  current,
  children,
  footer = true,
}: {
  current: string;
  children: ReactNode;
  footer?: boolean;
}) {
  return (
    <>
      <Header current={current} />
      <main id="main">{children}</main>
      {footer && <Footer />}
    </>
  );
}
