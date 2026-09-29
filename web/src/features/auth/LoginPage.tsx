import { type FormEvent, useState } from "react";
import { useLocation, useNavigate } from "react-router";

import { ApiRequestError } from "@/api/client";
import { BeanAvatar } from "@/components/brand/BeanAvatar";
import { ThemeToggle } from "@/components/ThemeToggle";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { useLogin } from "@/features/auth/queries";
import { useI18n } from "@/i18n";

export function LoginPage() {
  const { t } = useI18n();
  const navigate = useNavigate();
  const location = useLocation();
  const login = useLogin();
  const [password, setPassword] = useState("");
  const next = readNext(location.search);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!password || login.isPending) return;
    try {
      await login.mutateAsync({ password });
      navigate(next, { replace: true });
    } catch (error) {
      // Backend chỉ trả thông đoạp an toàn; không hiển thị chi tiết lỗi nội bộ.
      if (!(error instanceof ApiRequestError)) setPassword("");
    }
  }

  const error = login.error instanceof ApiRequestError ? login.error : null;
  return (
    <main className="relative flex min-h-svh items-center justify-center bg-background p-4">
      {/* Nút chủ đề đặt ở góc màn hình: màn đăng nhập là nơi người dùng hay đổi
          chủ đề lần đầu, không nên bắt họ đăng nhập mới đổi được. */}
      <ThemeToggle className="absolute top-4 right-4" />
      <Card className="w-full max-w-sm gap-7 py-7">
        <CardHeader>
          <BeanAvatar size={56} className="mb-3" title={t("login.title")} />
          <CardTitle className="text-2xl tracking-tight">{t("login.title")}</CardTitle>
          <CardDescription>{t("login.subtitle")}</CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={submit} className="space-y-4">
            <label className="block text-sm font-medium" htmlFor="password">
              {t("login.password")}
            </label>
            <Input
              id="password"
              name="password"
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              className="h-11"
              required
              disabled={login.isPending}
            />
            {error ? (
              <p className="rounded-md bg-destructive/10 px-3 py-2 text-sm text-destructive" role="alert">
                {error.code === "rate_limited" ? t("login.rateLimited") : t("login.invalid")}
              </p>
            ) : null}
            <Button
              type="submit"
              disabled={login.isPending}
              className="h-11 w-full bg-brand text-brand-ink hover:bg-brand-strong"
            >
              {login.isPending ? t("common.loading") : t("login.submit")}
            </Button>
          </form>
        </CardContent>
      </Card>
    </main>
  );
}

function readNext(search: string): string {
  const value = new URLSearchParams(search).get("next");
  return value?.startsWith("/") && !value.startsWith("//") ? value : "/";
}
