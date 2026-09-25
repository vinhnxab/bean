import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { delay, HttpResponse, http } from "msw";
import { Route, Routes } from "react-router";
import { describe, expect, it } from "vitest";

import { SkillsPage } from "@/features/skills/SkillsPage";
import { renderManagement } from "@/test/management";
import { testServer } from "@/test/server";

describe("SkillsPage", () => {
  it("hiển thị trạng thái tải", () => {
    testServer.use(
      http.get("/api/skills", async () => {
        await delay(100);
        return HttpResponse.json({ skills: [] });
      }),
    );
    renderManagement(
      <Routes>
        <Route path="/skills" element={<SkillsPage />} />
        <Route path="/skills/:name" element={<SkillsPage />} />
      </Routes>,
      ["/skills"],
    );
    expect(screen.getAllByRole("status")[0]).toHaveTextContent("Đang tải");
  });

  it("tải danh sách và mở nội dung skill", async () => {
    testServer.use(
      http.get("/api/skills", () =>
        HttpResponse.json({ skills: [{ name: "daily-briefing", description: "Tóm tắt buổi sáng" }] }),
      ),
      http.get("/api/skills/daily-briefing", () =>
        HttpResponse.json({
          name: "daily-briefing",
          description: "Tóm tắt buổi sáng",
          content: "# Daily briefing",
          directory: "/skills/daily-briefing",
        }),
      ),
    );
    const user = userEvent.setup();
    renderManagement(
      <Routes>
        <Route path="/skills" element={<SkillsPage />} />
        <Route path="/skills/:name" element={<SkillsPage />} />
      </Routes>,
      ["/skills"],
    );
    await user.click(await screen.findByRole("button", { name: /daily-briefing/ }));
    expect(await screen.findByText(/Daily briefing/)).toBeInTheDocument();
    expect(screen.getByText("/skills/daily-briefing")).toBeInTheDocument();
  });

  it("hiển thị diff và duyệt skill nháp", async () => {
    let pending = true;
    testServer.use(
      http.get("/api/skills/drafts", () =>
        HttpResponse.json({
          drafts: pending
            ? [
                {
                  id: "draft_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                  name: "release-checklist",
                  kind: "new",
                  description: "Dùng trước khi phát hành.",
                  content: "---\nname: release-checklist\n---\n# Checklist",
                  reason: "Quy trình lặp lại nhiều lần.",
                  status: "pending",
                  created_at: "2026-09-25T10:00:00Z",
                },
              ]
            : [],
        }),
      ),
      http.post("/api/skills/drafts/:id/approve", () => {
        pending = false;
        return HttpResponse.json({
          id: "draft_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
          status: "approved",
        });
      }),
    );
    const user = userEvent.setup();
    renderManagement(
      <Routes>
        <Route path="/skills" element={<SkillsPage />} />
        <Route path="/skills/:name" element={<SkillsPage />} />
      </Routes>,
      ["/skills"],
    );

    expect(await screen.findByText(/Có đề xuất skill mới/)).toBeInTheDocument();
    expect(screen.getByText(/Quy trình lặp lại/)).toBeInTheDocument();
    expect(screen.getByText(/# Checklist/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Bỏ" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Duyệt" }));
    await waitFor(() => {
      expect(screen.queryByText(/Có đề xuất skill mới/)).not.toBeInTheDocument();
    });
  });

  it("hiển thị lỗi danh sách", async () => {
    testServer.use(
      http.get("/api/skills", () => HttpResponse.json({ code: "boom", message: "no" }, { status: 500 })),
    );
    renderManagement(
      <Routes>
        <Route path="/skills" element={<SkillsPage />} />
        <Route path="/skills/:name" element={<SkillsPage />} />
      </Routes>,
      ["/skills"],
    );
    expect(await screen.findByRole("alert")).toHaveTextContent("Đã xảy ra lỗi.");
  });
});
