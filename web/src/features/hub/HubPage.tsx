import { ErrorState } from "@/components/ui/Page";
import { ConfirmQueue } from "@/features/hub/ConfirmQueue";
import { useAgents } from "@/features/hub/queries";
import { Topology } from "@/features/hub/Topology";
import { useI18n } from "@/i18n";

/**
 * Trang chủ — Dashboard (chất liệu hologram).
 *
 * # Thứ tự bố cục: theo tần suất bạn cần HÀNH ĐỘNG, không theo thứ tự đẹp
 *
 * 1. sơ đồ hệ thống (trả lời "ai đang làm gì, nối với ai")
 * 2. hàng chờ duyệt (việc bạn phải làm ngay)
 *
 * Số liệu sống (token, audit, registry, runtime) nằm ở `DashboardSidebar` —
 * đảo kính phải kiểu Telegram Web, đọc ở cột bên thay vì chen giữa nội dung.
 * Provider/model không lặp lần ba: pill trạng thái ở sidebar trái đã hiển thị.
 *
 * # Chất liệu hologram và ranh giới của nó
 *
 * Hologram (lưới nền, panel viền phát sáng, vạch quét) là **khung hiển thị**
 * của số liệu — không phải màu ngữ nghĩa. Ba màu trạng thái (--live/--need/
 * --alert) vẫn giữ nguyên vai trò cũ ở mọi nơi; trang trí cyan không được mang
 * nghĩa nào (test "không mang nghĩa bằng chuyển động" khoá điều này).
 */
export function HubPage() {
  const { t } = useI18n();
  const agents = useAgents();

  return (
    // `pt-20/pb-24` cũ là phần đệm để né TopNav khi nav còn là overlay — nav giờ
    // nằm trong luồng flex của ChatLayout nên đệm đó là khoảng chết: cắt bớt để
    // sơ đồ nới spacing mà toàn trang vẫn gọn trong một màn hình.
    <div className="relative mx-auto w-full max-w-7xl flex-1 px-4 py-6 sm:px-6 md:pb-10">
      {/* Lưới hologram phía sau tiêu đề: trang trí thuần, mờ dần xuống dưới,
          pointer-events-none để không bao giờ chắn tương tác. */}
      <div aria-hidden="true" className="holo-grid-bg pointer-events-none absolute inset-x-0 top-0 h-80" />
      <div className="relative">
        {/* Tiêu đề trang chỉ còn cho trình đọc màn hình (mỗi trang một h1): nav
            trái và pill model đã định vị người dùng, logo + chữ mô tả bị bỏ —
            Dashboard nhường chỗ cho sơ đồ. Bean vẫn xuất hiện trong sơ đồ
            (node Manager) và ở trạng thái rỗng. */}
        <h1 className="sr-only">{t("hub.title")}</h1>

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
          </>
        )}
      </div>
    </div>
  );
}
