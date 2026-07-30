import { PHASE_DEVELOPMENT_SERVER } from "next/constants";
import type { NextConfig } from "next";

/**
 * The Mini App and the API answer on one origin.
 *
 * Not a convenience: a Telegram Mini App runs inside a WebView whose page is served from this
 * host, so calling a different origin would need CORS, would put the API's hostname in the
 * client bundle, and would break the moment a corporate network blocks the second host.
 *
 * In production there is nothing to arrange: `pustol-api` serves this build itself, so one origin
 * is a property of there being one listener rather than of a rule somebody has to keep writing.
 * Every route is prerendered and nothing is decided per request, so exporting loses the app
 * nothing — and it takes Node, and the whole of `node_modules`, out of production.
 *
 * `output: "export"` is set in every phase, including `next dev`, so that a change which quietly
 * needs a server — a route handler, middleware, a dynamic segment — fails in front of whoever
 * wrote it rather than in the deploy that cannot serve it.
 */
const config = (phase: string): NextConfig => ({
  output: "export",

  // Only `next dev`, which serves the app on :3000 while the API answers on :8080. An export has
  // no server to rewrite anything, and asking for it there earns a warning on every build.
  ...(phase === PHASE_DEVELOPMENT_SERVER
    ? {
        async rewrites() {
          const api = process.env.PUSTOL_API_URL ?? "http://127.0.0.1:8080";
          return [
            { source: "/api/:path*", destination: `${api}/api/:path*` },
            { source: "/health", destination: `${api}/health` },
          ];
        },
      }
    : {}),

  // The app is one screen deep and every asset is local; nothing here needs an image optimiser.
  images: { unoptimized: true },
  // Types are checked by `npm run typecheck` and in the build; nothing here silences either.
  typescript: { ignoreBuildErrors: false },
});

export default config;
