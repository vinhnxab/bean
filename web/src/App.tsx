import { BrowserRouter, Navigate, Route, Routes } from "react-router";

import { LoginPage } from "@/features/auth/LoginPage";
import { ProtectedRoute } from "@/features/auth/ProtectedRoute";
import { ChatIndexPage } from "@/features/chat/ChatIndexPage";
import { ChatLayout } from "@/features/chat/ChatLayout";
import { ChatPage } from "@/features/chat/ChatPage";
import { RealtimeProvider } from "@/features/chat/RealtimeProvider";

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
            <Route index element={<ChatIndexPage />} />
            <Route path="sessions/:sessionId" element={<ChatPage />} />
          </Route>
        </Route>
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </BrowserRouter>
  );
}
