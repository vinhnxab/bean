import { BeanAvatar } from "@/components/brand/BeanAvatar";
import { ErrorState } from "@/components/ui/Page";
import { ActivityFeed } from "@/features/hub/ActivityFeed";
import { ConfirmQueue } from "@/features/hub/ConfirmQueue";
import { useAgents } from "@/features/hub/queries";
import { Topology } from "@/features/hub/Topology";
import { useI18n } from "@/i18n";

/**
 * Trang chủ — HUB.
 *
 * # Thứ tự bố cục: theo tần suất bạn cần HÀNH ĐỘNG, không theo thứ tự đẹp
 *
 * 1. sơ đồ hệ thống (trả lời "ai đang làm gì, nối với ai")
 * 2. hàng chờ duyệt (việc bạn phải làm ngay)
 * 3. hoạt động + trạng thái (ngữ cảnh, không cạnh tranh sự chú ý)
 *
 * # Vì sao chỉ sơ đồ được "bỏ mựa"
 *
 * Nguyên tắc "spend boldness in one place": sơ đồ là nơi duy nhất có chiều sâu,
 * có lưới kẻ, có nét đứt. Hàng chờ duyệt có **trọng số** nhờ vị trí + viền màu,
 * không nhờ trang trí. Không vùng nào thứ ba được phép cạnh tranh.
 */
export function HubPage() {
  const { t } = useI18n();
  const agents = useAgents();

  return (
    <div className="mx-auto w-full max-w-7xl flex-1 px-4 py-6 pb-24 pt-20 sm:px-6 md:pb-10 md:pt-8">
      <header className="mb-5 flex flex-wrap items-center gap-3">
        {/* Mascot ở góc trên-trái: thứ ấm duy nhất trên màn hình. */}
        <BeanAvatar size={44} title={t("hub.manager.name")} />
        <div>
          <h1 className="text-2xl font-bold tracking-tight sm:text-3xl">{t("hub.title")}</h1>
          <p className="mt-1 max-w-2xl text-sm text-ink-muted">{t("hub.description")}</p>
        </div>
      </header>

      {agents.isError ? (
        <ErrorState onRetry={() => void agents.refetch()} />
      ) : (
        <>
          <Topology agents={agents.data?.agents ?? []} />
          {/* Hàng chờ duyệt nằm ngay dưới sơ đồ: trọng số thị giác cao, không
              chôn trong tab phụ. */}
          <div className="mt-4">
            <ConfirmQueue />
          </div>
          <div className="mt-6">
            <ActivityFeed />
          </div>
        </>
      )}
    </div>
  );
}
