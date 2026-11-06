"use client";

import { useState } from "react";

import type { SettingsTestResponse } from "@/api/bindings";
import { Button } from "@/components/ui/button";
import { useTestConnection } from "@/features/settings/queries";
import { useI18n } from "@/i18n";

export type ConnectionTarget = "llm" | "voice" | "telegram" | "search";

/**
 * Nút "Kiểm tra kết nối" cho một nhóm cài đặt — gọi `POST /api/settings/test/:target`,
 * cùng probe dùng chung với `bean doctor --online` (M29 mục 8.1).
 *
 * Kết quả server đã redact secret, nên hiển thị nguyên văn là an toàn.
 */
export function ConnectionTestButton({
  target,
  disabled = false,
}: {
  target: ConnectionTarget;
  disabled?: boolean;
}) {
  const { t } = useI18n();
  const test = useTestConnection();
  const [result, setResult] = useState<SettingsTestResponse | null>(null);

  const handleClick = async () => {
    setResult(null);
    try {
      setResult(await test.mutateAsync(target));
    } catch {
      // Lỗi mạng/4xx ngoài hợp đồng API — message đã được client lọc (invalid_response
      // hoặc http_error), không lộ secret.
      setResult(null);
    }
  };

  return (
    <div className="flex flex-col sm:flex-row gap-2 items-start sm:items-center">
      <Button type="button" disabled={disabled || test.isPending} onClick={handleClick}>
        {test.isPending ? t("testConnection.testing") : t("testConnection.button")}
      </Button>
      {result ? (
        <div className="flex items-center gap-2 text-sm">
          {result.level === "ok" ? (
            <span className="font-medium text-live dark:text-live-strong">{t("testConnection.ok")}</span>
          ) : result.level === "warn" ? (
            <span className="font-medium text-need dark:text-need-strong">{t("testConnection.warn")}</span>
          ) : (
            <span className="font-medium text-alert dark:text-alert-strong">{t("testConnection.error")}</span>
          )}
          <span className="text-muted-foreground">{result.message}</span>
          {result.hint ? (
            <span className="ml-1 text-xs text-muted-foreground">
              {t("testConnection.hint", { hint: result.hint })}
            </span>
          ) : null}
        </div>
      ) : null}
      {!result && test.isError ? (
        <span className="text-xs font-medium text-destructive">{t("testConnection.failed")}</span>
      ) : null}
    </div>
  );
}
