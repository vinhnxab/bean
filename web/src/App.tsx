import { lazy } from "react";
import { BrowserRouter, Navigate, Route, Routes } from "react-router";
import { LoginPage } from "@/features/auth/LoginPage";
import { ProtectedRoute } from "@/features/auth/ProtectedRoute";
import { ChatLayout } from "@/features/chat/ChatLayout";
import { RealtimeProvider } from "@/features/chat/RealtimeProvider";

/**
 * Tách từng màn thành chunk riêng.
 *
 * Vì sao phải lazy
 *
 * Giao diện này có 11 màn, và một màn (chat) kéo theo `react-markdown` +
 * `highlight.js` — vài trăm ký tự mà **người vào thẳng `/status` không hề dùng**.
 * Gom tất cả vào một chunk làm mọi lượt tải đều phải tải cả phần thừa đó.
 *
 * Vì sao `LoginPage` và `ChatLayout` vẫn tĩnh
 *
 * `LoginPage` là thứ đầu tiên người dùng thấy khi chưa đăng nhập — nếu nó cũng
 * phải tải chunk thì có một khoảng trống trước khi form hiện ra. `ChatLayout`
 * (khung + sidebar) là thứ giữ nguyên khi chuyển màn, nên tải cùng lúc với trang
 * đầu tiên còn hơn là chờ rồi mới hiện. `Suspense` của các route lazy nằm ở
 * `ChatLayout` quanh `<Outlet />` — đặt ở đây thì lúc đang tải chunk, sidebar vẫn
 * đứng yên thay vì biến mất rồi nhảy lại.
 */
const HubPage = lazy(() => import("@/features/hub/HubPage").then((m) => ({ default: m.HubPage })));
const ChatIndexPage = lazy(() =>
  import("@/features/chat/ChatIndexPage").then((m) => ({ default: m.ChatIndexPage })),
);
const ChatPage = lazy(() => import("@/features/chat/ChatPage").then((m) => ({ default: m.ChatPage })));
const MemoryPage = lazy(() =>
  import("@/features/memory/MemoryPage").then((m) => ({ default: m.MemoryPage })),
);
const SkillsPage = lazy(() =>
  import("@/features/skills/SkillsPage").then((m) => ({ default: m.SkillsPage })),
);
const TasksPage = lazy(() => import("@/features/tasks/TasksPage").then((m) => ({ default: m.TasksPage })));
const ToolsPage = lazy(() => import("@/features/tools/ToolsPage").then((m) => ({ default: m.ToolsPage })));
const McpPage = lazy(() => import("@/features/mcp/McpPage").then((m) => ({ default: m.McpPage })));
const AuditPage = lazy(() => import("@/features/audit/AuditPage").then((m) => ({ default: m.AuditPage })));
const StatusPage = lazy(() =>
  import("@/features/status/StatusPage").then((m) => ({ default: m.StatusPage })),
);

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/login" element={<LoginPage />} />
        <Route element={<ProtectedRoute />}>
          <Route
            element={
              <RealtimeProvider>
                <ChatLayout />
              </RealtimeProvider>
            }
          >
            {/* HUB là trang chủ: `/` hiện trạng thái cả hệ agent, chat nằm ở
                `/sessions/:id` và `/chat` (giữ deep-link cũ còn dùng được). */}
            <Route index element={<HubPage />} />
            <Route path="chat" element={<ChatIndexPage />} />
            <Route path="sessions/:sessionId" element={<ChatPage />} />
            <Route path="memory" element={<MemoryPage />} />
            <Route path="skills" element={<SkillsPage />} />
            <Route path="skills/:name" element={<SkillsPage />} />
            <Route path="tasks" element={<TasksPage />} />
            <Route path="tools" element={<ToolsPage />} />
            <Route path="mcp" element={<McpPage />} />
            <Route path="audit" element={<AuditPage />} />
            <Route path="status" element={<StatusPage />} />
          </Route>
        </Route>
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </BrowserRouter>
  );
}
