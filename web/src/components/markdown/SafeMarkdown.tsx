import hljs from "highlight.js/lib/common";

import { isValidElement, type ReactNode, useState } from "react";
import ReactMarkdown, { type Components, type UrlTransform } from "react-markdown";
import remarkGfm from "remark-gfm";
import { useI18n } from "@/i18n";

function safeUrl(value: string): string | null {
  try {
    const url = new URL(value);
    return ["http:", "https:", "mailto:"].includes(url.protocol) ? url.href : null;
  } catch {
    return null;
  }
}

const safeTransform: UrlTransform = (value) => safeUrl(value) ?? "";

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (isValidElement<{ children?: ReactNode }>(node)) return textOf(node.props.children);
  return "";
}

function highlightedNodes(code: string, language: string | undefined): ReactNode {
  if (!language || typeof DOMParser === "undefined" || !hljs.getLanguage(language)) return code;
  const result = hljs.highlight(code, { language, ignoreIllegals: true });
  const document = new DOMParser().parseFromString(`<body>${result.value}</body>`, "text/html");
  const renderNode = (node: globalThis.Node, key: number): ReactNode => {
    if (node.nodeType === globalThis.Node.TEXT_NODE) return node.textContent;
    if (node.nodeType !== globalThis.Node.ELEMENT_NODE) return null;
    const element = node as HTMLElement;
    const children = [...element.childNodes].map((child, index) => renderNode(child, index));
    const className = typeof element.className === "string" ? element.className : "";
    return (
      <span className={className} key={`${element.tagName}-${key}`}>
        {children}
      </span>
    );
  };
  return [...document.body.childNodes].map((node, index) => renderNode(node, index));
}

function CodeBlock({ children }: { children: ReactNode }) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);
  const code = textOf(children).replace(/\n$/, "");
  const language = isValidElement<{ className?: string }>(children)
    ? children.props.className?.match(/language-([\w-]+)/)?.[1]
    : undefined;

  async function copy() {
    try {
      await navigator.clipboard?.writeText(code);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1400);
    } catch {
      setCopied(false);
    }
  }

  return (
    <div className="group relative my-3 overflow-hidden rounded-xl border border-slate-200 bg-slate-950 dark:border-slate-800">
      <button
        type="button"
        onClick={copy}
        className="absolute right-2 top-2 z-10 rounded-md bg-slate-800 px-2 py-1 text-xs text-slate-200 hover:bg-slate-700"
        aria-label={t("markdown.copyCode")}
      >
        {copied ? t("common.copied") : t("common.copy")}
      </button>
      <pre className="overflow-x-auto p-4 pr-20 text-sm leading-6 text-slate-100">
        <code className={language ? `language-${language}` : undefined}>
          {highlightedNodes(code, language)}
        </code>
      </pre>
    </div>
  );
}

function linkDomain(href: string): string {
  try {
    const url = new URL(href);
    return url.protocol === "mailto:" ? "email" : url.hostname;
  } catch {
    return "";
  }
}

export function SafeMarkdown({ children }: { children: string }) {
  const { t } = useI18n();
  const components: Components = {
    a: ({ href, children: content }) => {
      const safeHref = safeUrl(href ?? "");
      if (!safeHref) return <span className="underline decoration-dotted underline-offset-2">{content}</span>;
      return (
        <a href={safeHref} target="_blank" rel="noopener noreferrer">
          {content}
          <span className="ml-1 text-xs opacity-60">({linkDomain(safeHref)})</span>
        </a>
      );
    },
    img: ({ src, alt }) => {
      const safeHref = safeUrl(src ?? "");
      return (
        <span className="my-2 block rounded-lg border border-amber-300 bg-amber-50 px-3 py-2 text-sm text-amber-900 dark:border-amber-800 dark:bg-amber-950 dark:text-amber-100">
          <strong>{t("markdown.remoteImage")}: </strong>
          {safeHref ? (
            <a href={safeHref} target="_blank" rel="noopener noreferrer">
              {alt || src}
            </a>
          ) : (
            <span>{alt || src}</span>
          )}{" "}
          — {t("markdown.remoteImageWarning")}
        </span>
      );
    },
    pre: ({ children: content }) => <CodeBlock>{content}</CodeBlock>,
    code: ({ className, children: content, ...props }) => (
      <code className={className} {...props}>
        {content}
      </code>
    ),
  };

  return (
    <div className="max-w-none break-words">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        skipHtml
        urlTransform={safeTransform}
        components={components}
      >
        {children}
      </ReactMarkdown>
    </div>
  );
}
