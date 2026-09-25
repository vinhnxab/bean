import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { SafeMarkdown } from "@/components/markdown/SafeMarkdown";
import { I18nProvider } from "@/i18n";

describe("SafeMarkdown", () => {
  it("không render HTML thô, URL javascript hoặc ảnh remote", () => {
    render(
      <I18nProvider>
        <SafeMarkdown>{`<script>alert(1)</script>\n\n<img src="https://evil.example/track.png" onerror="alert(2)">\n\n[bad](javascript:alert(3))\n\n![remote](https://evil.example/image.png)\n\n[ok](https://example.com/docs)`}</SafeMarkdown>
      </I18nProvider>,
    );

    expect(document.querySelector("script")).toBeNull();
    expect(document.querySelector("img")).toBeNull();
    expect(screen.queryByRole("link", { name: /bad/ })).toBeNull();
    expect(screen.getByText(/Ảnh từ xa/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /ok/ })).toHaveAttribute("href", "https://example.com/docs");
    expect(screen.getByRole("link", { name: /ok/ })).toHaveAttribute("rel", "noopener noreferrer");
  });

  it("hiển thị code dạng text và không thêm thẽ script từ nội dung code", () => {
    render(
      <I18nProvider>
        <SafeMarkdown>{"```html\n<script>alert(1)</script>\n```"}</SafeMarkdown>
      </I18nProvider>,
    );

    expect(document.querySelector("pre")?.textContent).toContain("<script>alert(1)</script>");
    expect(document.querySelector("script")).toBeNull();
  });
});
