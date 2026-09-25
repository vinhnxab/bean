import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { delay, HttpResponse, http } from "msw";
import { describe, expect, it } from "vitest";

import { AuditPage } from "@/features/audit/AuditPage";
import { renderManagement } from "@/test/management";
import { testServer } from "@/test/server";

function entry(index: number) {
  return {
    ts: `2026-09-26T00:00:${String(index).padStart(2, "0")}Z`,
    session: index,
    channel: "web",
    tool: "read_file",
    args: { path: `file-${index}` },
    ok: true,
    decision: "allow",
    decided_by: "web:admin",
    error: null,
  };
}

describe("AuditPage", () => {
  it("hiển thị lỗi và tải trang audit kế tiếp", async () => {
    const firstPage = Array.from({ length: 25 }, (_, index) => entry(index));
    testServer.use(
      http.get("/api/audit", ({ request }) => {
        const before = new URL(request.url).searchParams.get("before");
        return HttpResponse.json({ entries: before ? [entry(25)] : firstPage });
      }),
    );
    const user = userEvent.setup();
    renderManagement(<AuditPage />);
    expect(await screen.findByText(/file-0/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Tải thêm" }));
    await waitFor(() => expect(screen.getByText(/file-25/)).toBeInTheDocument());
  });

  it("hiển thị lỗi tải audit", async () => {
    testServer.use(
      http.get("/api/audit", () => HttpResponse.json({ code: "boom", message: "no" }, { status: 500 })),
    );
    renderManagement(<AuditPage />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Đã xảy ra lỗi.");
  });
  it("hiển thị trạng thái tải", () => {
    testServer.use(
      http.get("/api/audit", async () => {
        await delay(100);
        return HttpResponse.json({ entries: [] });
      }),
    );
    renderManagement(<AuditPage />);
    expect(screen.getByText("Đang tải…")).toBeInTheDocument();
  });
});
