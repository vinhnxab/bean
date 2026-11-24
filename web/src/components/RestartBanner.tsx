import { useMutation } from "@tanstack/react-query";
import { RotateCcwIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { ApiRequestError, api } from "@/api/client";
import { Button } from "@/components/ui/button";
import { useStatus } from "@/features/status/queries";
import { useI18n } from "@/i18n";

/**
 * Banner "Cần khởi động lại Bean" (M31) — nút restart kiểu Ubuntu.
 *
 * Hiện khi `/api/status` báo `restart.required` (khoá cấu hình tier B đã ghi
 * đĩa chưa hiệu lực, hoặc bản mới đã cài qua `bean update`); mỗi lý do được
 * server định dạng sẵn nên banner chỉ liệt kê. "Khởi động lại ngay" gửi
 * `POST /api/system/restart` (admin-only; server từ chối 409 khi còn run) rồi
 * chuyển sang trạng thái "đang khởi động lại" — WS tự đứt và nối lại với
 * backoff (cơ chế sẵn có), banner tự biến mất khi `/api/status` mới không còn
 * `required`. "Để sau" chỉ ẩn banner trong phiên hiện tại (sessionStorage),
 * không đổi gì phía server.
 */
export function RestartBanner({ className = "" }: { className?: string }) {
  const { t } = useI18n();
  const status = useStatus();
  const [dismissed, setDismissed] = useState(() => sessionStorage.getItem(DISMISSED_KEY) === "1");
  const [error, setError] = useState<string | null>(null);
  // "202 đã nhận, đang chờ binary mới" — tách khỏi mutation vì `isSuccess` của
  // mutation persist mãi trong mount; dựa vào nó thì lần chờ-restart KẾ TIẾP
  // sẽ hiện sai "Đang khởi động lại…" với nút bị khoá oan.
  const [awaitingReboot, setAwaitingReboot] = useState(false);

  // `restart?.` — ui_dir phục vụ UI từ đĩa nên UI có thể mới hơn binary đang
  // chạy (binary cũ trước M31 chưa trả field `restart`); thiếu `?.` ở đây làm
  // crash cả cây React (TypeError reading 'required' — lỗi thật ngày 2026-11-24).
  const required = status.data?.restart?.required ?? false;
  const reasons = status.data?.restart?.reasons ?? [];

  const {
    mutate: requestRestart,
    reset: resetRestart,
    isPending: requestPending,
  } = useMutation({
    mutationFn: () => api.systemRestart(),
    onSuccess: () => {
      setError(null);
      setAwaitingReboot(true);
      // 202 = tiến trình đã nhận: spinner giữ tới khi `/api/status` mới (sau
      // exec) trả `required = false` — effect ở dưới reset. Refetch được bấm
      // NGAY bởi RealtimeProvider khi WS nối lại (reconnect → invalidate
      // status), nên banner tắt realtime thay vì chờ chu kỳ 30 giây.
    },
    onError: (err) => {
      if (err instanceof ApiRequestError && err.code === "restart_pending") {
        // Một restart đã được yêu cầu (tab khác) và chưa hoàn tất — coi như
        // đã gửi, spinner đợi binary mới lên.
        setError(null);
        setAwaitingReboot(true);
        return;
      }
      setAwaitingReboot(false);
      setError(
        err instanceof ApiRequestError && err.code === "run_active"
          ? t("restart.runActive")
          : t("restart.failed"),
      );
    },
  });

  // Backend hết chờ restart (binary mới đã lên) → reset sạch mọi trạng thái
  // để lần chờ-restart kế tiếp hiện lại đúng. RealtimeProvider bấm refetch
  // status ngay khi WS nối lại, nên nhánh này chạy trong cùng giây đó.
  useEffect(() => {
    if (!required) {
      setError(null);
      setAwaitingReboot(false);
      resetRestart();
      if (dismissed) {
        setDismissed(false);
        sessionStorage.removeItem(DISMISSED_KEY);
      }
    }
  }, [required, dismissed, resetRestart]);

  if (!required || dismissed) {
    return null;
  }

  const restarting = awaitingReboot || requestPending;

  return (
    <section
      role="status"
      aria-label={t("restart.aria")}
      data-slot="restart-banner"
      data-testid="restart-banner"
      data-state={restarting ? "restarting" : error ? "error" : "idle"}
      className={`glass flex shrink-0 flex-col gap-2 rounded-2xl border border-need/30 bg-need/10 p-3 text-sm sm:flex-row sm:items-center sm:gap-3 ${className}`}
    >
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <p className="flex items-center gap-2 font-semibold text-need">
          <RotateCcwIcon className="size-4 shrink-0" aria-hidden="true" />
          {t("restart.title")}
        </p>
        {reasons.length > 0 ? (
          <ul className="min-w-0 list-inside list-disc text-xs text-muted-foreground">
            {reasons.map((reason) => (
              <li key={reason} className="truncate">
                {reason}
              </li>
            ))}
          </ul>
        ) : null}
        {error ? (
          <p className="text-xs text-alert" role="alert">
            {t("restart.reasonPrefix")} {error}
          </p>
        ) : null}
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Button
          type="button"
          size="sm"
          disabled={restarting}
          onClick={() => requestRestart()}
          data-slot="restart-now"
          aria-label={t("restart.restartNow")}
          title={t("restart.restartNow")}
        >
          {restarting ? t("restart.restarting") : t("restart.restartNow")}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          disabled={restarting}
          onClick={() => {
            setDismissed(true);
            sessionStorage.setItem(DISMISSED_KEY, "1");
          }}
          data-slot="restart-later"
          aria-label={t("restart.later")}
          title={t("restart.later")}
        >
          {t("restart.later")}
        </Button>
      </div>
    </section>
  );
}

const DISMISSED_KEY = "bean.restart-banner-dismissed";
