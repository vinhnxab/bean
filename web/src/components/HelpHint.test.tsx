import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { HelpHint } from "@/components/HelpHint";
import { renderManagement } from "@/test/management";

describe("HelpHint", () => {
  it("mặc định chỉ hiện dấu hỏi, không hiện nội dung giải thích", () => {
    renderManagement(<HelpHint title="Host mode" text="Giải thích dài về host mode." />);
    expect(screen.getByTestId("help-hint")).toBeInTheDocument();
    expect(screen.queryByText("Giải thích dài về host mode.")).not.toBeInTheDocument();
  });

  it("bấm dấu hỏi thì popover hiện kế bên nút, kèm tiêu đề + nội dung", async () => {
    const user = userEvent.setup();
    renderManagement(<HelpHint title="Host mode" text="Giải thích dài về host mode." />);
    await user.click(screen.getByTestId("help-hint"));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByText("Host mode")).toBeInTheDocument();
    expect(screen.getByText("Giải thích dài về host mode.")).toBeInTheDocument();
  });

  it("bấm lần nữa (hoặc Escape) thì đóng, không nhân bản nội dung", async () => {
    const user = userEvent.setup();
    renderManagement(<HelpHint title="Host mode" text="Giải thích dài về host mode." />);
    await user.click(screen.getByTestId("help-hint"));
    expect(screen.getAllByText("Giải thích dài về host mode.")).toHaveLength(1);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("nhiều HelpHint trên một trang: id tiêu đề không trùng nhau", async () => {
    const user = userEvent.setup();
    renderManagement(
      <div>
        <HelpHint title="A" text="Giải thích A." />
        <HelpHint title="B" text="Giải thích B." />
      </div>,
    );
    const firstHint = screen.getAllByTestId("help-hint").at(0);
    if (firstHint === undefined) throw new Error("không tìm thấy help-hint nào");
    await user.click(firstHint);
    expect(screen.getByText("Giải thích A.")).toBeInTheDocument();
    expect(screen.queryByText("Giải thích B.")).not.toBeInTheDocument();
  });
});
