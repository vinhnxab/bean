import { BrowserRouter, Navigate, Route, Routes } from "react-router";
import { AuditPage } from "@/features/audit/AuditPage";
import { LoginPage } from "@/features/auth/LoginPage";
import { ProtectedRoute } from "@/features/auth/ProtectedRoute";
import { ChatIndexPage } from "@/features/chat/ChatIndexPage";
import { ChatLayout } from "@/features/chat/ChatLayout";
import { ChatPage } from "@/features/chat/ChatPage";
import { RealtimeProvider } from "@/features/chat/RealtimeProvider";
import { HubPage } from "@/features/hub/HubPage";
import { MemoryPage } from "@/features/memory/MemoryPage";
import { SkillsPage } from "@/features/skills/SkillsPage";
import { StatusPage } from "@/features/status/StatusPage";
import { TasksPage } from "@/features/tasks/TasksPage";

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
            <Route path="audit" element={<AuditPage />} />
            <Route path="status" element={<StatusPage />} />
          </Route>
        </Route>
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </BrowserRouter>
  );
}
