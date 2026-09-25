import { useEffect } from "react";
import { Navigate, Outlet, useLocation, useNavigate } from "react-router";

import { setUnauthorizedHandler } from "@/api/client";
import { useAuth } from "@/features/auth/queries";
import { useI18n } from "@/i18n";

export function ProtectedRoute() {
  const { isPending, isError, data } = useAuth();
  const { t } = useI18n();
  const navigate = useNavigate();
  const location = useLocation();
  const next = `${location.pathname}${location.search}`;

  useEffect(() => {
    setUnauthorizedHandler(() => {
      navigate(`/login?next=${encodeURIComponent(next)}`, { replace: true });
    });
    return () => setUnauthorizedHandler(null);
  }, [navigate, next]);

  if (isPending) return <div className="p-6 text-sm text-slate-500">{t("auth.checking")}</div>;
  if (isError || !data) return <Navigate to={`/login?next=${encodeURIComponent(next)}`} replace />;
  return <Outlet />;
}
