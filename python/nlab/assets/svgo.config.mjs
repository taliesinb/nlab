// svgo config for inline docset diagrams. Ids must stay unique across
// the many SVGs inlined into one HTML page: prefix them with the
// content hash passed via SVGO_PREFIX (see bulk_pages.py).
export default {
  multipass: true,
  plugins: [
    'preset-default',
    {
      name: 'prefixIds',
      params: { prefix: process.env.SVGO_PREFIX || 'x', delim: '' },
    },
  ],
};
