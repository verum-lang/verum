import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import sitemap from '@astrojs/sitemap';
import mdx from '@astrojs/mdx';

export default defineConfig({
  site: 'https://verum-lang.org',
  integrations: [
    starlight({
      title: 'Verum',
      description: 'Memory safety without runtime cost. Verification without ceremony.',
      logo: {
        src: './src/assets/verum-logo.png',
        replacesTitle: false,
      },
      // Native favicon support (Starlight 0.37+)
      favicon: '/favicon.png',
      social: [
        { icon: 'github', label: 'GitHub', href: 'https://github.com/luxquant/verum' },
      ],
      editLink: {
        baseUrl: 'https://github.com/luxquant/verum/edit/main/website/',
      },
      // Last updated timestamps
      lastUpdated: true,
      // Pagination between pages
      pagination: true,
      // Table of contents configuration
      tableOfContents: {
        minHeadingLevel: 2,
        maxHeadingLevel: 4,
      },
      // Expressive Code for syntax highlighting (Astro 5.x / Starlight 0.37+)
      expressiveCode: {
        themes: ['github-dark', 'github-light'],
        styleOverrides: {
          borderRadius: '8px',
          codePaddingBlock: '1rem',
          codePaddingInline: '1rem',
          codeFontFamily: "'JetBrains Mono', 'Fira Code', 'Cascadia Code', ui-monospace, monospace",
        },
        defaultProps: {
          wrap: true,
          preserveIndent: true,
        },
      },
      customCss: ['./src/styles/custom.css'],
      sidebar: [
        {
          label: 'Start Here',
          items: [
            { label: 'Introduction', slug: 'introduction' },
            { label: 'Installation', slug: 'installation' },
            { label: 'Hello World', slug: 'hello-world' },
          ],
        },
        {
          label: 'Language Guide',
          items: [
            { label: 'Types', slug: 'guide/types' },
            { label: 'Functions', slug: 'guide/functions' },
            { label: 'Protocols', slug: 'guide/protocols' },
            { label: 'Memory Model', slug: 'guide/memory' },
            { label: 'Contexts', slug: 'guide/contexts' },
            { label: 'Refinement Types', slug: 'guide/refinements' },
            { label: 'Gradual Verification', slug: 'guide/verification' },
          ],
        },
        {
          label: 'Core Library',
          items: [
            { label: 'Overview', slug: 'core/overview' },
            { label: 'Collections', slug: 'core/collections' },
            { label: 'Concurrency', slug: 'core/concurrency' },
            { label: 'IO', slug: 'core/io' },
          ],
        },
        {
          label: 'Cogs',
          items: [
            { label: 'What are Cogs?', slug: 'cogs/overview' },
            { label: 'Creating a Cog', slug: 'cogs/creating' },
            { label: 'Publishing', slug: 'cogs/publishing' },
          ],
        },
        {
          label: 'Reference',
          items: [
            { label: 'Grammar', slug: 'reference/grammar' },
            { label: 'Keywords', slug: 'reference/keywords' },
            { label: 'Operators', slug: 'reference/operators' },
          ],
        },
      ],
      head: [
        // Additional favicon sizes
        {
          tag: 'link',
          attrs: {
            rel: 'icon',
            href: '/favicon-32.png',
            type: 'image/png',
            sizes: '32x32',
          },
        },
        {
          tag: 'link',
          attrs: {
            rel: 'icon',
            href: '/favicon-48.png',
            type: 'image/png',
            sizes: '48x48',
          },
        },
        // Apple Touch Icon
        {
          tag: 'link',
          attrs: {
            rel: 'apple-touch-icon',
            href: '/apple-touch-icon.png',
            sizes: '180x180',
          },
        },
        // Android/PWA Icons
        {
          tag: 'link',
          attrs: {
            rel: 'icon',
            href: '/android-chrome-192.png',
            type: 'image/png',
            sizes: '192x192',
          },
        },
        {
          tag: 'link',
          attrs: {
            rel: 'icon',
            href: '/android-chrome-384.png',
            type: 'image/png',
            sizes: '384x384',
          },
        },
        // Open Graph
        {
          tag: 'meta',
          attrs: {
            property: 'og:image',
            content: 'https://verum-lang.org/verum-logo-512.png',
          },
        },
        {
          tag: 'meta',
          attrs: {
            property: 'og:image:width',
            content: '512',
          },
        },
        {
          tag: 'meta',
          attrs: {
            property: 'og:image:height',
            content: '512',
          },
        },
        // Twitter Card
        {
          tag: 'meta',
          attrs: {
            name: 'twitter:card',
            content: 'summary',
          },
        },
        {
          tag: 'meta',
          attrs: {
            name: 'twitter:image',
            content: 'https://verum-lang.org/verum-logo-512.png',
          },
        },
        // PWA Manifest
        {
          tag: 'link',
          attrs: {
            rel: 'manifest',
            href: '/manifest.json',
          },
        },
        // Theme color for mobile browsers
        {
          tag: 'meta',
          attrs: {
            name: 'theme-color',
            content: '#4361ee',
          },
        },
      ],
      components: {
        // Override default components for custom landing page
        Hero: './src/components/Hero.astro',
      },
    }),
    mdx(),
    sitemap(),
  ],
  // Astro 5.x build optimizations
  build: {
    inlineStylesheets: 'auto',
  },
  // Astro 5.16+ improved image handling
  image: {
    experimentalLayout: 'responsive',
  },
});
