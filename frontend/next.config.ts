import type { NextConfig } from "next";

const isDev = process.env.NODE_ENV !== "production";

// Production is a static export served by styk-panel itself; in `next dev`
// the API is proxied to a locally running backend.
const nextConfig: NextConfig = {
  output: isDev ? undefined : "export",
  trailingSlash: true,
  images: { unoptimized: true },
  turbopack: {
    rules: {
      "*.css": { loaders: ["@tailwindcss/turbopack"], as: "*.css" },
    },
  },
  ...(isDev && {
    async rewrites() {
      const api = process.env.STYK_API ?? "http://127.0.0.1:8088";
      return [{ source: "/api/:path*", destination: `${api}/api/:path*` }];
    },
  }),
};

export default nextConfig;
