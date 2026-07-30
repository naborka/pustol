import type { Metadata, Viewport } from "next";
import Script from "next/script";

import { ThemeProvider } from "@/components/ThemeProvider";

import "./globals.css";

export const metadata: Metadata = {
  title: "Столик",
  description: "Бронирование столов в баре",
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  // A Mini App is a fixed panel, not a document: letting it zoom moves the layout out from under
  // whoever is tapping it.
  maximumScale: 1,
  userScalable: false,
  viewportFit: "cover",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="ru">
      <body>
        {/*
          Telegram's own script, loaded before the app paints. It is the only source of the signed
          payload and of the user's theme, so a page that rendered first would flash in the wrong
          colours and then have to authenticate.
        */}
        <Script src="https://telegram.org/js/telegram-web-app.js" strategy="beforeInteractive" />
        <ThemeProvider>{children}</ThemeProvider>
      </body>
    </html>
  );
}
