import { useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { Copy, Check } from "lucide-react";

import "highlight.js/styles/github-dark.css";

function CopyButton({ text, className = "" }: { text: string; className?: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      onClick={(e) => {
        e.stopPropagation();
        navigator.clipboard.writeText(text);
        setCopied(true);
        setTimeout(() => setCopied(false), 1200);
      }}
      className={`text-phantom-muted hover:text-phantom-text transition flex items-center gap-1 text-[11px] ${className}`}
      aria-label="複製"
    >
      {copied ? <Check size={13} /> : <Copy size={13} />}
      {copied ? "已複製" : "複製"}
    </button>
  );
}

interface MarkdownProps {
  children: string;
  className?: string;
}

export default function Markdown({ children, className = "" }: MarkdownProps) {
  return (
    <div className={`md-body ${className}`}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeHighlight]}
        components={{
          // ── Code blocks ────────────────────────────────────────────────
          pre({ children, ...props }) {
            // Extract code text for copy button
            // children is a <code> element from rehype
            const codeNode = (children as any)?.props?.children ?? children;
            const codeText = typeof codeNode === "string" ? codeNode :
              Array.isArray(codeNode) ? codeNode.join("") : String(codeNode || "");

            // Try to detect language from className
            const lang = (children as any)?.props?.className?.match(/language-([\w+-]+)/)?.[1];

            return (
              <div className="relative group my-2.5 rounded-lg overflow-hidden border border-phantom-border bg-[#0d1117]">
                <div className="flex items-center justify-between px-3 py-1.5 bg-phantom-card border-b border-phantom-border text-[10px]">
                  <span className="text-phantom-muted font-mono uppercase tracking-wide">
                    {lang || "code"}
                  </span>
                  <CopyButton text={codeText} />
                </div>
                <pre {...props} className="text-[12.5px] leading-relaxed p-3 overflow-x-auto">
                  {children}
                </pre>
              </div>
            );
          },

          // ── Inline code ────────────────────────────────────────────────
          code({ className, children, ...props }) {
            // If parent is <pre> rehype-highlight handles it; only style true inline
            const isInline = !className?.includes("language-");
            if (isInline) {
              return (
                <code
                  {...props}
                  className="px-1.5 py-0.5 rounded bg-phantom-card border border-phantom-border text-[0.875em] text-phantom-primary font-mono"
                >
                  {children}
                </code>
              );
            }
            return <code className={className} {...props}>{children}</code>;
          },

          // ── Headings ───────────────────────────────────────────────────
          h1: ({ children }) => <h1 className="text-lg font-bold mt-3 mb-1.5">{children}</h1>,
          h2: ({ children }) => <h2 className="text-base font-bold mt-3 mb-1.5">{children}</h2>,
          h3: ({ children }) => <h3 className="text-sm font-bold mt-2 mb-1">{children}</h3>,

          // ── Paragraph ──────────────────────────────────────────────────
          p: ({ children }) => <p className="my-1.5 leading-relaxed">{children}</p>,

          // ── Lists ──────────────────────────────────────────────────────
          ul: ({ children }) => <ul className="list-disc pl-5 my-1.5 space-y-0.5">{children}</ul>,
          ol: ({ children }) => <ol className="list-decimal pl-5 my-1.5 space-y-0.5">{children}</ol>,
          li: ({ children }) => <li className="leading-relaxed">{children}</li>,

          // ── Block quote ────────────────────────────────────────────────
          blockquote: ({ children }) => (
            <blockquote className="border-l-2 border-phantom-primary/60 pl-3 my-2 text-phantom-muted italic">
              {children}
            </blockquote>
          ),

          // ── Links ──────────────────────────────────────────────────────
          a: ({ href, children }) => (
            <a
              href={href}
              target="_blank"
              rel="noopener noreferrer"
              className="text-phantom-primary underline underline-offset-2 break-all"
            >
              {children}
            </a>
          ),

          // ── Tables ─────────────────────────────────────────────────────
          table: ({ children }) => (
            <div className="overflow-x-auto my-2">
              <table className="border-collapse text-[12px]">{children}</table>
            </div>
          ),
          th: ({ children }) => (
            <th className="border border-phantom-border bg-phantom-card px-2 py-1 text-left font-medium">
              {children}
            </th>
          ),
          td: ({ children }) => (
            <td className="border border-phantom-border px-2 py-1">{children}</td>
          ),

          // ── HR ─────────────────────────────────────────────────────────
          hr: () => <hr className="border-phantom-border my-3" />,
        }}
      >
        {children}
      </ReactMarkdown>
    </div>
  );
}

export { CopyButton };
