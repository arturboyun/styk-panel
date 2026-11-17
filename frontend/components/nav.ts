import {
  Bell,
  Boxes,
  DatabaseBackup,
  FolderTree,
  Globe,
  LayoutDashboard,
  ScrollText,
  Settings,
  ShieldCheck,
  type LucideIcon,
} from "lucide-react";

export interface NavItem {
  href: string;
  label: string;
  icon: LucideIcon;
  soon?: boolean;
}

export const NAV: { title?: string; items: NavItem[] }[] = [
  {
    items: [
      { href: "/", label: "Обзор", icon: LayoutDashboard },
      { href: "/projects/", label: "Проекты", icon: Boxes },
    ],
  },
  {
    title: "Инфраструктура",
    items: [
      { href: "/domains/", label: "Домены", icon: Globe },
      { href: "/backups/", label: "Бэкапы и БД", icon: DatabaseBackup },
      { href: "/files/", label: "Файлы", icon: FolderTree },
    ],
  },
  {
    title: "Контроль",
    items: [
      { href: "/alerts/", label: "Алерты", icon: Bell },
      { href: "/security/", label: "Безопасность", icon: ShieldCheck },
      { href: "/audit/", label: "Аудит", icon: ScrollText },
      { href: "/settings/", label: "Настройки", icon: Settings },
    ],
  },
];
