import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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

describe("SafeMarkdown — khối code", () => {
  it("hiện tên ngôn ngữ trên header và nút sao chép", () => {
    render(
      <I18nProvider>
        <SafeMarkdown>{"```python\nprint('xin chào')\n```"}</SafeMarkdown>
      </I18nProvider>,
    );
    // Tên ngôn ngữ phải đọc được TRƯỚC khi đọc code — đó là lý do có header.
    expect(screen.getByText("python")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sao chép mã" })).toBeInTheDocument();
  });

  it("gắn nhãn 'văn bản thuần' khi khối không có ngôn ngữ", () => {
    render(
      <I18nProvider>
        <SafeMarkdown>{"```\nplain\n```"}</SafeMarkdown>
      </I18nProvider>,
    );
    expect(screen.getByText("văn bản thuần")).toBeInTheDocument();
  });

  it("không hiện nút thu gọn với khối ngắn, nhưng thu gọn khối dài và báo số dòng ẩn", async () => {
    const user = userEvent.setup();
    const short = "```\na\nb\n```";
    const { rerender } = render(
      <I18nProvider>
        <SafeMarkdown>{short}</SafeMarkdown>
      </I18nProvider>,
    );
    expect(screen.queryByRole("button", { name: "Mở rộng khối mã" })).not.toBeInTheDocument();

    // 30 dòng: dài hơn ngưỡng 12 nên phải thu gọn và nói rõ còn bao nhiêu dòng.
    const long = `\`\`\`rust\n${Array.from({ length: 30 }, (_, i) => `let x = ${i};`).join("\n")}\n\`\`\``;
    rerender(
      <I18nProvider>
        <SafeMarkdown>{long}</SafeMarkdown>
      </I18nProvider>,
    );
    expect(screen.getByText("18 dòng ẩn")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Mở rộng khối mã" }));
    expect(screen.queryByText("18 dòng ẩn")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Thu gọn khối mã" })).toHaveAttribute("aria-expanded", "true");
  });

  it("dòng cuối bị cắt vẫn hiện đủ, không mất ký tự UTF-8", () => {
    // Biên cắt theo DÒNG (`slice` trên mảng string), nên tiếng Việt có dấu và
    // emoji ở ranh giới vẫn nguyên vẹn — cùng lý do mục 6.9 của spec.
    const lines = Array.from({ length: 14 }, (_, i) => `// dòng ${i} 🎉 có dấu`);
    render(
      <I18nProvider>
        <SafeMarkdown>{`\`\`\`\n${lines.join("\n")}\n\`\`\``}</SafeMarkdown>
      </I18nProvider>,
    );
    const text = document.querySelector("pre")?.textContent ?? "";
    expect(text).toContain("dòng 11 🎉 có dấu");
  });
});
