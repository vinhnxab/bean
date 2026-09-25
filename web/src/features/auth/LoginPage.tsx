import { type FormEvent, useState } from "react";
import { useLocation, useNavigate } from "react-router";

import { ApiRequestError } from "@/api/client";
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
    <main className="flex min-h-svh items-center justify-center bg-slate-50 p-4 dark:bg-slate-950">
      <section
        className="w-full max-w-sm rounded-2xl border border-slate-200 bg-white p-7 shadow-sm dark:border-slate-800 dark:bg-slate-900"
        aria-labelledby="login-title"
      >
        <div className="mb-7">
          <div className="mb-3 inline-flex rounded-xl bg-emerald-100 px-3 py-1 text-sm font-bold text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300">
            B
          </div>
          <h1 id="login-title" className="text-2xl font-semibold tracking-tight">
            {t("login.title")}
          </h1>
          <p className="mt-2 text-sm text-slate-500 dark:text-slate-400">{t("login.subtitle")}</p>
        </div>
        <form onSubmit={submit} className="space-y-4">
          <label className="block text-sm font-medium" htmlFor="password">
            {t("login.password")}
          </label>
          <input
            id="password"
            name="password"
            type="password"
            autoComplete="current-password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            className="w-full rounded-xl border border-slate-300 bg-white px-3 py-3 outline-none transition focus:border-emerald-500 focus:ring-2 focus:ring-emerald-500/20 dark:border-slate-700 dark:bg-slate-950"
            required
            disabled={login.isPending}
          />
          {error ? (
            <p
              className="rounded-lg bg-rose-50 px-3 py-2 text-sm text-rose-700 dark:bg-rose-950 dark:text-rose-300"
              role="alert"
            >
              {error.code === "rate_limited" ? t("login.rateLimited") : t("login.invalid")}
            </p>
          ) : null}
          <button
            type="submit"
            disabled={login.isPending}
            className="w-full rounded-xl bg-emerald-600 px-4 py-3 font-semibold text-white transition hover:bg-emerald-700 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {login.isPending ? t("common.loading") : t("login.submit")}
          </button>
        </form>
      </section>
    </main>
  );
}

function readNext(search: string): string {
  const value = new URLSearchParams(search).get("next");
  return value?.startsWith("/") && !value.startsWith("//") ? value : "/";
}
