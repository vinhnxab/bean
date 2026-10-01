import type { UsageDayDto } from "@/api/bindings";
import { Panel } from "@/components/ui/Page";
import { windowTotal } from "@/features/usage/queries";
import { useI18n } from "@/i18n";
import { formatDay } from "@/lib/format";

/**
 * Biểu đồ token theo ngày — "hệ thống đang tiêu bao nhiêu, và hôm nay so với
 * các ngày trước ra sao".
 *
 * # Vì sao là cột, không phải đường
 *
 * Mỗi cột là một **bản ghi tổng kết của một ngày** (khoá UTC), không phải một mẫu
 * liên tục. Đường nối gợi "có dữ liệu giữa hai ngày" — thứ mà API không có.
 *
 * # Vì sao ngày bằng 0 vẫn có cột
 *
 * Cột trống tuyệt đối làm người dùng đọc là "Bean ngừng chạy" trong khi sự thật là "không ai
 * gọi". Nên cột rỗng tồn tại và cao đúng một mép, và server cũng trả `0` chứ không
 * bỏ vắng ngày (test Rust `usage_endpoint_..._never_fabricates` chốt điều đó).
 *
 * # Tiếp cận
 *
 * Toàn bộ biểu đồ là `role="img"` kèm nhãn tóm tắt (tổng, ngày cao nhất). Người
 * đọc màn hình nhận đúng thông tin cần quyết định thay vì 14 con số vô nghĩa;
 * number thật vẫn hiện bằng chữ ở dòng trên, không chỉ trong cột.
 */
export function UsageBars({ days, budget }: { days: UsageDayDto[]; budget: number }) {
  const { t, lang } = useI18n();

  if (days.length === 0) {
    return (
      <Panel>
        <h2 className="font-semibold">{t("usage.title")}</h2>
        <p className="mt-2 text-sm text-muted-foreground">{t("usage.empty")}</p>
      </Panel>
    );
  }

  const total = windowTotal(days);
  const peak = days.reduce((best, day) => (day.total_tokens > best.total_tokens ? day : best), days[0]);
  const scale = Math.max(1, peak.total_tokens);
  const first = days[0];
  const last = days[days.length - 1];
  const overBudget = budget > 0 && last.total_tokens > budget;

  return (
    <Panel>
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h2 className="font-semibold">{t("usage.title")}</h2>
          <p className="hub-num mt-1 text-2xl font-bold">{total.toLocaleString()}</p>
          <p className="mt-1 text-xs text-muted-foreground">
            {t("usage.range", { from: formatDay(first.day, lang), to: formatDay(last.day, lang) })}
          </p>
        </div>
        <div className="text-right">
          <p className="text-xs text-muted-foreground">{t("usage.peak")}</p>
          <p className="hub-num text-sm font-semibold">
            {peak.total_tokens.toLocaleString()} · {formatDay(peak.day, lang)}
          </p>
          {budget > 0 ? (
            <p
              className={
                overBudget ? "mt-1 text-xs font-semibold text-alert" : "mt-1 text-xs text-muted-foreground"
              }
            >
              {overBudget
                ? t("status.budgetExceeded")
                : t("usage.budget", { budget: budget.toLocaleString() })}
            </p>
          ) : null}
        </div>
      </div>

      <div
        role="img"
        aria-label={t("usage.summary", {
          days: days.length,
          total: total.toLocaleString(),
          peak: peak.total_tokens.toLocaleString(),
          peakDay: formatDay(peak.day, lang),
        })}
        className="mt-4 flex h-24 items-end gap-1"
      >
        {days.map((day) => {
          const percent = Math.round((day.total_tokens / scale) * 100);
          const isPeak = day.day === peak.day && peak.total_tokens > 0;
          return (
            <div
              key={day.day}
              data-testid="usage-bar"
              data-day={day.day}
              data-total={day.total_tokens}
              title={`${formatDay(day.day, lang)} · ${day.total_tokens.toLocaleString()}`}
              className={
                isPeak ? "min-h-[3px] flex-1 rounded-sm bg-need" : "min-h-[3px] flex-1 rounded-sm bg-live/60"
              }
              style={{ height: `${percent}%` }}
            />
          );
        })}
      </div>
    </Panel>
  );
}
