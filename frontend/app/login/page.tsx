"use client";

import { Suspense, useEffect, useRef, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import QRCode from "qrcode";
import { Check, Copy, KeyRound, Loader2, ShieldCheck } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { InputOTP, InputOTPGroup, InputOTPSlot } from "@/components/ui/input-otp";
import { Label } from "@/components/ui/label";
import { LogoMark } from "@/components/logo";
import { ApiError, api, post } from "@/lib/api";

type Step = "password" | "totp" | "totp_setup" | "recovery";

const TG_FIELDS = ["id", "first_name", "last_name", "username", "photo_url", "auth_date", "hash"];

function errorText(e: unknown) {
  if (e instanceof ApiError) {
    if (e.code === "invalid_credentials") return "Неверный логин или пароль";
    if (e.code === "invalid_code") return "Неверный код";
    if (e.code === "rate_limited") return "Слишком много попыток, подождите 15 минут";
    if (e.code === "telegram_denied") return "Этот Telegram-аккаунт не привязан к панели";
    if (e.status === 401) return "Сессия входа истекла, начните заново";
    return e.message;
  }
  return "Сервер недоступен";
}

function TelegramButton({ bot }: { bot: string }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const s = document.createElement("script");
    s.src = "https://telegram.org/js/telegram-widget.js?22";
    s.async = true;
    s.dataset.telegramLogin = bot;
    s.dataset.size = "large";
    s.dataset.radius = "10";
    s.dataset.authUrl = `${location.origin}/login/`;
    el.appendChild(s);
    return () => {
      el.innerHTML = "";
    };
  }, [bot]);
  return <div ref={ref} className="flex min-h-10 justify-center" />;
}

function CodeInput({ onComplete, disabled }: { onComplete: (code: string) => void; disabled?: boolean }) {
  const [value, setValue] = useState("");
  return (
    <InputOTP
      maxLength={6}
      value={value}
      autoFocus
      disabled={disabled}
      inputMode="numeric"
      pattern="^[0-9]*$"
      onChange={(v) => {
        setValue(v);
        if (v.length === 6) {
          onComplete(v);
          setValue("");
        }
      }}
      containerClassName="justify-center"
    >
      <InputOTPGroup>
        {Array.from({ length: 6 }, (_, i) => (
          <InputOTPSlot key={i} index={i} className="size-11 text-lg" />
        ))}
      </InputOTPGroup>
    </InputOTP>
  );
}

function LoginFlow() {
  const router = useRouter();
  const params = useSearchParams();
  const qc = useQueryClient();
  const [step, setStep] = useState<Step>("password");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [useRecovery, setUseRecovery] = useState(false);
  const [setup, setSetup] = useState<{ secret: string; qr: string } | null>(null);
  const [codes, setCodes] = useState<string[]>([]);
  const [copied, setCopied] = useState(false);

  const { data: cfg } = useQuery({ queryKey: ["auth-config"], queryFn: () => api<{ telegram_bot_username: string | null }>("/auth/config") });

  const done = () => {
    qc.invalidateQueries({ queryKey: ["me"] });
    router.replace("/");
  };

  // Already signed in → straight to the panel.
  useEffect(() => {
    api("/auth/me").then(done, () => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Telegram redirect mode lands here with the signed fields in the query string.
  const tgHandled = useRef(false);
  useEffect(() => {
    if (tgHandled.current || !params.get("hash") || !params.get("id")) return;
    tgHandled.current = true;
    const data = Object.fromEntries(TG_FIELDS.filter((k) => params.has(k)).map((k) => [k, params.get(k)!]));
    window.history.replaceState(null, "", "/login/");
    // Make sure the CSRF cookie exists before the POST.
    api("/auth/config")
      .then(() => post("/auth/telegram", data))
      .then(done)
      .catch((e) => setError(errorText(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [params]);

  useEffect(() => {
    if (step !== "totp_setup") return;
    api<{ secret: string; otpauth_url: string }>("/auth/totp/setup")
      .then(async (r) => setSetup({ secret: r.secret, qr: await QRCode.toDataURL(r.otpauth_url, { margin: 1, width: 220 }) }))
      .catch((e) => {
        setError(errorText(e));
        setStep("password");
      });
  }, [step]);

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(errorText(e));
      if (e instanceof ApiError && e.status === 401) setStep("password");
    } finally {
      setBusy(false);
    }
  };

  const submitPassword = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const f = new FormData(e.currentTarget);
    run(async () => {
      const r = await post<{ next: "totp" | "totp_setup" }>("/auth/login", { username: f.get("username"), password: f.get("password") });
      setStep(r.next);
    });
  };

  const submitCode = (code: string) =>
    run(async () => {
      const r = await post<{ recovery_codes: string[] | null }>("/auth/totp", { code });
      if (r.recovery_codes) {
        setCodes(r.recovery_codes);
        setStep("recovery");
      } else done();
    });

  return (
    <div className="relative flex min-h-screen items-center justify-center overflow-hidden px-4">
      <div className="pointer-events-none absolute inset-0 -z-10 bg-[radial-gradient(60rem_30rem_at_50%_-10%,color-mix(in_oklch,var(--primary)_22%,transparent),transparent)]" />
      <div className="w-full max-w-sm">
        <div className="mb-8 flex flex-col items-center gap-3 text-center">
          <div className="grid size-12 place-items-center rounded-2xl bg-gradient-to-br from-primary to-chart-5 text-primary-foreground shadow-lg shadow-primary/20">
            <LogoMark className="size-7" />
          </div>
          <div>
            <div className="text-xl font-semibold tracking-tight">Styk Panel</div>
            <div className="text-sm text-muted-foreground">Панель управления сервером</div>
          </div>
        </div>

        <Card className="shadow-xl shadow-black/5">
          {step === "password" && (
            <>
              <CardHeader>
                <CardTitle>Вход</CardTitle>
                <CardDescription>Логин, пароль и код из приложения-аутентификатора</CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <form onSubmit={submitPassword} className="space-y-3">
                  <div className="space-y-1.5">
                    <Label htmlFor="username">Логин</Label>
                    <Input id="username" name="username" autoComplete="username" required autoFocus />
                  </div>
                  <div className="space-y-1.5">
                    <Label htmlFor="password">Пароль</Label>
                    <Input id="password" name="password" type="password" autoComplete="current-password" required />
                  </div>
                  {error && <p className="text-sm text-destructive">{error}</p>}
                  <Button type="submit" className="h-9 w-full" disabled={busy}>
                    {busy && <Loader2 className="animate-spin" />} Продолжить
                  </Button>
                </form>
                {cfg?.telegram_bot_username && (
                  <>
                    <div className="flex items-center gap-3 text-xs text-muted-foreground">
                      <div className="h-px flex-1 bg-border" /> или <div className="h-px flex-1 bg-border" />
                    </div>
                    <TelegramButton bot={cfg.telegram_bot_username} />
                  </>
                )}
              </CardContent>
            </>
          )}

          {step === "totp" && (
            <>
              <CardHeader>
                <CardTitle className="flex items-center gap-2">
                  <ShieldCheck className="size-5 text-primary" /> Двухфакторная проверка
                </CardTitle>
                <CardDescription>{useRecovery ? "Введите один из резервных кодов" : "Код из приложения-аутентификатора"}</CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                {useRecovery ? (
                  <form
                    onSubmit={(e) => {
                      e.preventDefault();
                      submitCode(String(new FormData(e.currentTarget).get("code")));
                    }}
                    className="space-y-3"
                  >
                    <Input name="code" placeholder="xxxxx-xxxxx" autoFocus autoComplete="off" className="font-mono" />
                    <Button type="submit" className="w-full" disabled={busy}>
                      Войти
                    </Button>
                  </form>
                ) : (
                  <CodeInput onComplete={submitCode} disabled={busy} />
                )}
                {error && <p className="text-center text-sm text-destructive">{error}</p>}
                <button className="w-full text-center text-xs text-muted-foreground hover:text-foreground" onClick={() => setUseRecovery((v) => !v)}>
                  {useRecovery ? "Ввести код из приложения" : "Нет доступа к приложению? Резервный код"}
                </button>
              </CardContent>
            </>
          )}

          {step === "totp_setup" && (
            <>
              <CardHeader>
                <CardTitle className="flex items-center gap-2">
                  <KeyRound className="size-5 text-primary" /> Настройка 2FA
                </CardTitle>
                <CardDescription>Отсканируйте QR-код в Google Authenticator, 1Password, Aegis и т.п., затем введите код.</CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="flex justify-center">
                  {setup ? (
                    // eslint-disable-next-line @next/next/no-img-element
                    <img src={setup.qr} alt="QR-код TOTP" className="size-[220px] rounded-lg bg-white p-1" />
                  ) : (
                    <div className="grid size-[220px] place-items-center rounded-lg bg-muted">
                      <Loader2 className="animate-spin text-muted-foreground" />
                    </div>
                  )}
                </div>
                {setup && (
                  <button
                    className="w-full rounded-md bg-muted px-2 py-1.5 text-center font-mono text-xs break-all text-muted-foreground hover:text-foreground"
                    onClick={() => navigator.clipboard.writeText(setup.secret).then(() => toast.success("Секрет скопирован"))}
                  >
                    {setup.secret}
                  </button>
                )}
                <CodeInput onComplete={submitCode} disabled={busy || !setup} />
                {error && <p className="text-center text-sm text-destructive">{error}</p>}
              </CardContent>
            </>
          )}

          {step === "recovery" && (
            <>
              <CardHeader>
                <CardTitle>Резервные коды</CardTitle>
                <CardDescription>Сохраните их в менеджере паролей. Каждый код одноразовый, больше они показаны не будут.</CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="grid grid-cols-2 gap-1.5 rounded-lg bg-muted p-3 font-mono text-sm">
                  {codes.map((c) => (
                    <div key={c}>{c}</div>
                  ))}
                </div>
                <Button
                  variant="outline"
                  className="w-full"
                  onClick={() => navigator.clipboard.writeText(codes.join("\n")).then(() => setCopied(true))}
                >
                  {copied ? <Check /> : <Copy />} {copied ? "Скопировано" : "Скопировать"}
                </Button>
                <Button className="w-full" onClick={done}>
                  Я сохранил коды
                </Button>
              </CardContent>
            </>
          )}
        </Card>
      </div>
    </div>
  );
}

export default function LoginPage() {
  return (
    <Suspense>
      <LoginFlow />
    </Suspense>
  );
}
