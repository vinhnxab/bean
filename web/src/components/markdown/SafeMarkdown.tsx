import hljs from "highlight.js/lib/common";
import { CheckIcon, ChevronDownIcon, ChevronUpIcon, CopyIcon } from "lucide-react";

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

const COLLAPSE_LINES = 12;

/**
 * Khối code có header riêng: tên ngôn ngữ, nút sao chép, nút thu gọn.
 *
 * # Vì sao có header thay vì nút sao chép trôi ở góc
 *
 * Ở bản cũ nút sao chép nằm `absolute` trong khối code, phủ lên dòng code đầu
 * tiên và không có nhãn ngôn ngữ. Hai hậu quả: (1) người đọc không biết đoạn
 * này là Python hay Rust trước khi đọc; (2) ở khối dài, nút trôi theo khi
 * cuộn nên chỉ dùng được khi đang ở đúng đoạn đó. Header cố định ở trên giải
 * quyết cả hai, và là chỗ tự nhiên để thêm nút thu gọn.
 *
 * # Vì sao thu gọn mặc định
 *
 * Log dài 400 dòng chiếm hết màn hình và đẩy lời cuội xuống dưới tầm nhìn.
 * Trên `COLLAPSE_LINES` dòng thì gập lại, hiện số dòng bị ẩn để người đọc
 * biết còn gì bên trong thay vì tưởng hết.
 */
function CodeBlock({ children }: { children: ReactNode }) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const code = textOf(children).replace(/\n$/, "");
  const language = isValidElement<{ className?: string }>(children)
    ? children.props.className?.match(/language-([\w-]+)/)?.[1]
    : undefined;
  const lineCount = code.split("\n").length;
  const collapsible = lineCount > COLLAPSE_LINES;
  const collapsed = collapsible && !expanded;

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
    <div className="code-block group/code my-3 overflow-hidden rounded-lg border border-rule bg-surface-raised">
      <div className="flex items-center justify-between gap-2 border-b border-rule bg-surface-hover/60 px-3 py-1.5">
        {/* Tên ngôn ngữ: chữ nhỏ, chữ đơn, `--ink-muted` — thông tin phụ, không
            được phải giành chỗ với nội dung code. */}
        <span className="truncate font-mono text-xs uppercase tracking-wide text-muted-foreground">
          {language ?? t("markdown.plainText")}
        </span>
        <div className="flex shrink-0 items-center gap-1">
          {collapsible ? (
            <button
              type="button"
              onClick={() => setExpanded((value) => !value)}
              aria-expanded={expanded}
              aria-label={expanded ? t("markdown.collapseCode") : t("markdown.expandCode")}
              title={expanded ? t("markdown.collapseCode") : t("markdown.expandCode")}
              className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-ink"
            >
              {expanded ? <ChevronUpIcon className="size-3.5" /> : <ChevronDownIcon className="size-3.5" />}
              <span className="hub-num">{lineCount}</span>
            </button>
          ) : null}
          <button
            type="button"
            onClick={copy}
            className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-ink"
            aria-label={t("markdown.copyCode")}
            title={copied ? t("common.copied") : t("markdown.copyCode")}
          >
            {copied ? <CheckIcon className="size-3.5 text-live" /> : <CopyIcon className="size-3.5" />}
            <span>{copied ? t("common.copied") : t("common.copy")}</span>
          </button>
        </div>
      </div>
      <pre className="overflow-x-auto p-4 text-sm leading-6 text-ink">
        <code className={language ? `language-${language}` : undefined}>
          {collapsed
            ? `${code.split("\n").slice(0, COLLAPSE_LINES).join("\n")}\n…`
            : highlightedNodes(code, language)}
        </code>
      </pre>
      {collapsed ? (
        <p className="border-t border-rule px-4 py-2 text-xs italic text-muted-foreground">
          {t("markdown.hiddenLines", { count: lineCount - COLLAPSE_LINES })}
        </p>
      ) : null}
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
        <span className="my-2 block rounded-lg border border-need bg-need/10 px-3 py-2 text-sm text-need">
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
