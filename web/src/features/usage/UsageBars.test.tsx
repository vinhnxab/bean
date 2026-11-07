import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { windowTotal } from "@/features/usage/queries";
import { UsageBars } from "@/features/usage/UsageBars";
import { renderManagement } from "@/test/management";
import { usageFixture } from "@/test/server";

const threeDays = usageFixture([
  { day: "2026-09-27", total: 0 },
  { day: "2026-09-28", total: 4000 },
  { day: "2026-09-29", total: 1200 },
]).days;

describe("UsageBars", () => {
  it("vẽ một cột cho mọi ngày, kể cả ngày bằng 0", () => {
    renderManagement(<UsageBars days={threeDays} budget={1_000_000} />);
    const bars = screen.getAllByTestId("usage-bar");
    expect(bars).toHaveLength(3);
    expect(bars[0]).toHaveAttribute("data-total", "0");
    expect(bars[1]).toHaveAttribute("data-total", "4000");
    // Ngày 0 vẫn chiếm chỗ (min-height) — cột trống tuyệt đối dễ đọc thành "Bean chết".
    expect(bars[0]?.className).toContain("min-h-[3px]");
  });

  it("tổng và ngày cao nhất đọc được bằng cả mắt lẫn màn hình", () => {
    renderManagement(<UsageBars days={threeDays} budget={1_000_000} />);
    expect(screen.getByText("5,200")).toBeInTheDocument();
    expect(screen.getByText("Peak day")).toBeInTheDocument();
    expect(screen.getByRole("img")).toHaveAccessibleName(/5,200/);
    expect(screen.getByRole("img")).toHaveAccessibleName(/4,000/);
  });

  it("đánh dấu cột cao nhất và ghi rõ hạn mức", () => {
    renderManagement(<UsageBars days={threeDays} budget={1_000_000} />);
    const bars = screen.getAllByTestId("usage-bar");
    expect(bars[1]?.className).toContain("bg-need");
    expect(bars[2]?.className).toContain("bg-live");
    expect(screen.getByText(/Budget 1,000,000\/day/)).toBeInTheDocument();
  });

  it("ngày cuối vượt hạn mức thì báo vượt, không vẽ cột bình thường", () => {
    renderManagement(
      <UsageBars days={usageFixture([{ day: "2026-09-29", total: 2000 }]).days} budget={1000} />,
    );
    expect(screen.getByText("Budget exceeded")).toBeInTheDocument();
    // Ghim đủ chuỗi hạn mức thay vì /Budget/: bản vượt hạn mức ("Budget exceeded")
    // cũng chứa "Budget" nên regex trần sẽ match nhầm.
    expect(screen.queryByText(/Budget 1,000,000\/day/)).not.toBeInTheDocument();
  });

  it("không có ngày nào thì nói chưa có dữ liệu thay vì biểu đồ rỗng", () => {
    renderManagement(<UsageBars days={[]} budget={1000} />);
    expect(screen.getByText("No daily token data yet.")).toBeInTheDocument();
    expect(screen.queryByTestId("usage-bar")).not.toBeInTheDocument();
  });

  it("windowTotal không đếm lệch so với cột đã vẽ", () => {
    expect(windowTotal(threeDays)).toBe(5200);
    expect(windowTotal([])).toBe(0);
  });
});
