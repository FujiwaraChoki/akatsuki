// One header for every secondary page: breadcrumb, serif title with its period, an optional lead
// and meta line, then a hairline. The breadcrumb always sits at the same height.
import type { ReactNode } from "react";
import { Hy } from "./Nb";

export function PageHeader({
  crumb,
  title,
  lead,
  meta,
}: {
  crumb: string;
  title: ReactNode;
  lead?: ReactNode;
  meta?: ReactNode;
}) {
  return (
    <header className="ph">
      <p className="ph-crumb">
        <a href="/">Akatsuki</a>
        <span aria-hidden="true"> / </span>
        <span>{crumb}</span>
      </p>
      <h1 className="ph-h1">{typeof title === "string" ? <Hy>{title.replace(/\.$/, "")}</Hy> : title}</h1>
      {lead && <div className="ph-lead">{lead}</div>}
      {meta && <p className="ph-meta">{meta}</p>}
    </header>
  );
}
