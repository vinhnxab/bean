import type { ReactNode } from "react";

import { ApiRequestError } from "@/api/client";
import { Button } from "@/components/ui/button";
import { useCheckUpdate } from "@/features/settings/queries";
import { useI18n } from "@/i18n";

/**
 * Render chuỗi i18n có inline-code bọc trong cặp backtick thành `<code>` thuần
 * văn bản (React tự escape) — không bao giờ nội suy chuỗi dịch vào HTML. Key là
 * nội dung đoạn (không phải index): chuỗi i18n nằm trong repo, mỗi cặp backtick
 * bọc một lệnh khác nhau nên không thể trùng key.
 */
function renderInlineCode(text: string): ReactNode {
  return text.split("`").map((segment, index) =>
    index % 2 === 1 ? (
      <code key={segment} className="rounded bg-muted px-1 font-mono">
        {segment}
      </code>
    ) : (
      segment
    ),
  );
}

/**
 * Thẻ **Phiên bản & cập nhật** (M30) trong mục Nâng cao: hỏi GitHub Releases có
 * bản mới hay không.
 *
 * # Ranh giới bảo mật
 *
 * Card chỉ **HỎI** — lần check duy nhất xảy ra khi người dùng bấm nút
 * (mutation, KHÔNG fetch lúc mount, không gọi định kỳ — D23.6: Bean không tự
 * gọi ra ngoài khi không được yêu cầu). Bean không tự tải/cài gì: self-update
 * chỉ qua CLI `bean update`, UI chỉ dẫn tới trang release. Khi RBAC bật mà user
 * không phải admin, server trả 403 `admin_required` — hiện thông báo nguyên văn
 * từ server. `notes` là nội dung không tin cậy từ GitHub (nguồn prompt
 * injection): render thuần văn bản qua React, tuyệt đối không markdown/HTML.
 */
export function UpdateCard() {
  const { t } = useI18n();
  const check = useCheckUpdate();
  const data = check.data;

  return (
    <div className="mt-4 border border-border rounded-lg p-4">
      <h3 className="text-sm font-medium">{t("settings.update.title")}</h3>
      <p className="mt-1 text-xs text-muted-foreground">{t("settings.update.description")}</p>

      <div className="mt-3">
        <Button type="button" size="sm" disabled={check.isPending} onClick={() => check.mutate({})}>
          {check.isPending ? t("common.loading") : t("settings.update.checkNow")}
        </Button>
      </div>

      {data ? (
        <>
          {data.update_available ? (
            <>
              <p
                className="mt-2 rounded-lg border border-live/50 bg-live/10 px-3 py-2 text-xs text-live"
                data-testid="update-status"
              >
                {t("settings.update.available", {
                  current: data.current_version,
                  latest: data.latest_version,
                })}
              </p>
              <p className="mt-2 text-xs text-muted-foreground">
                {t("settings.update.viewRelease")}:{" "}
                <a
                  href={data.release_url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="break-all text-live underline underline-offset-2"
                >
                  {data.release_url}
                </a>
              </p>
              <p className="mt-2 text-xs text-muted-foreground">
                {renderInlineCode(t("settings.update.installHint"))}
              </p>
            </>
          ) : (
            <p className="mt-3 text-xs text-muted-foreground" data-testid="update-status">
              {t("settings.update.upToDate", { version: data.current_version })}
            </p>
          )}

          {data.artifact === null ? (
            <p className="text-need text-xs mt-2">{t("settings.update.noArtifact")}</p>
          ) : null}

          {data.notes !== "" ? (
            <p className="mt-2 text-xs text-muted-foreground whitespace-pre-line">{data.notes}</p>
          ) : null}
        </>
      ) : null}

      {check.isError ? (
        <p className="text-xs text-destructive font-medium mt-3" role="alert">
          {check.error instanceof ApiRequestError ? check.error.message : t("settings.update.checkFailed")}
        </p>
      ) : null}
    </div>
  );
}
