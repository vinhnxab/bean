import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { PlanCard } from "@/components/chat/PlanCard";
import { I18nProvider } from "@/i18n";

function renderCard(ui: React.ReactNode) {
  return render(<I18nProvider>{ui}</I18nProvider>);
}

describe("PlanCard (D25)", () => {
  it("hiển thị các bước kèm tiến độ, tiêu đề bước xong gạch ngang", () => {
    renderCard(
      <PlanCard
        steps={[
          { title: "Đọc bug report", status: "done" },
          { title: "Sửa code", status: "in_progress" },
          { title: "Chạy test", status: "pending" },
        ]}
      />,
    );
    expect(screen.getByRole("region", { name: "Plan" })).toBeInTheDocument();
    expect(screen.getByText("1/3 done")).toBeInTheDocument();
    expect(screen.getByText("Đọc bug report")).toHaveClass("line-through");
    expect(screen.getByText("Sửa code")).toBeInTheDocument();
    expect(screen.getByText("Chạy test")).toBeInTheDocument();
  });

  it("danh sách rỗng thì không render gì cả", () => {
    const { container } = renderCard(<PlanCard steps={[]} />);
    expect(container).toBeEmptyDOMElement();
  });
});
