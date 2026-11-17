import Link from "next/link";

export default function NotFound() {
  return (
    <div className="grid min-h-screen place-items-center px-4 text-center">
      <div>
        <div className="text-6xl font-semibold tracking-tight text-muted-foreground/40">404</div>
        <p className="mt-2 text-muted-foreground">Такой страницы нет</p>
        <Link href="/" className="mt-4 inline-block text-sm text-primary hover:underline">
          На главную
        </Link>
      </div>
    </div>
  );
}
