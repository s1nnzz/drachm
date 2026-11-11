// Static export. Set NEXT_BASE_PATH (e.g. "/drachm") when serving from a
// sub-path such as GitHub Pages.
const basePath = process.env.NEXT_BASE_PATH || '';

/** @type {import('next').NextConfig} */
export default {
  output: 'export',
  basePath,
  trailingSlash: true,
  images: { unoptimized: true },
  reactStrictMode: true,
};
