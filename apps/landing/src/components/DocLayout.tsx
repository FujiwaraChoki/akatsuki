// §7: the document layout for secondary pages. Shared page header (breadcrumb, serif title with a
// period, meta), a sticky numbered table of contents, 66ch prose and optional "In short" sidenotes.
import type { ReactNode } from "react";
import { Hy } from "./Nb";
import { PageHeader } from "./PageHeader";
import { Shell } from "./Shell";

const bare = (t: string) => t.replace(/\.$/, "");
const withPeriod = bare;

export interface DocPart {
  id: string;
  num: string;
  title: string;
  /** Either HTML (from markdown) or React content. */
  html?: string;
  body?: ReactNode;
  note?: ReactNode;
}

export function DocLayout({
  current,
  crumb,
  title,
  meta,
  lead,
  intro,
  parts,
  toc = true,
}: {
  current: string;
  crumb: string;
  title: string;
  meta: string;
  lead?: ReactNode;
  intro?: ReactNode;
  parts: DocPart[];
  toc?: boolean;
}) {
  const showToc = toc && parts.length > 1;
  return (
    <Shell current={current}>
      <div className="container doc">
        <PageHeader crumb={crumb} title={title} lead={lead} meta={meta} />
        <div className={`grid doc-body${showToc ? "" : " no-toc"}`}>
          {showToc && (
            <>
              <nav className="doc-toc" aria-label="On this page" data-toc>
                <p className="toc-h">Contents</p>
                <ol>
                  {parts.map((p) => (
                    <li key={p.id}>
                      <a href={`#${p.id}`}>
                        <span className="toc-n">{p.num}</span>
                        {bare(p.title)}
                      </a>
                    </li>
                  ))}
                </ol>
              </nav>
              <details className="doc-toc-m">
                <summary className="toc-h">Contents</summary>
                <ol>
                  {parts.map((p) => (
                    <li key={p.id}>
                      <a href={`#${p.id}`}>
                        <span className="toc-n">{p.num}</span>
                        {bare(p.title)}
                      </a>
                    </li>
                  ))}
                </ol>
              </details>
            </>
          )}
          <div className="doc-main">
            {intro && (
              <div className="doc-sec">
                <div className="prose">{intro}</div>
              </div>
            )}
            {parts.map((p) => (
              <section key={p.id} className="doc-sec" aria-labelledby={p.id}>
                {p.html ? (
                  // biome-ignore lint/security/noDangerouslySetInnerHtml: our own checked-in documentation, rendered at build time
                  <div className="prose" dangerouslySetInnerHTML={{ __html: p.html }} />
                ) : (
                  <div className="prose">
                    <h2 id={p.id}>
                      <span className="hn">{p.num}</span>
                      <Hy>{withPeriod(p.title)}</Hy>
                    </h2>
                    {p.body}
                  </div>
                )}
                {p.note && (
                  <div className="side">
                    <p className="side-h">In short</p>
                    <p>{p.note}</p>
                  </div>
                )}
              </section>
            ))}
          </div>
        </div>
      </div>
    </Shell>
  );
}
