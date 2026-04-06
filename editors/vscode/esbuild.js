const esbuild = require('esbuild');

const production = process.argv.includes('--production');

async function main() {
    const ctx = await esbuild.context({
        entryPoints: ['src/extension.ts'],
        bundle: true,
        format: 'cjs',
        minify: production,
        sourcemap: !production,
        sourcesContent: false,
        platform: 'node',
        outfile: 'out/extension.js',
        external: ['vscode'],
        logLevel: 'silent',
        plugins: [
            {
                name: 'esbuild-problem-matcher',
                setup(build) {
                    build.onStart(() => {
                        console.log('[esbuild] build started');
                    });
                    build.onEnd((result) => {
                        for (const { text, location } of result.errors) {
                            console.error(`> ${location.file}:${location.line}:${location.column}: error: ${text}`);
                        }
                        console.log('[esbuild] build finished');
                    });
                },
            },
        ],
    });

    if (process.argv.includes('--watch')) {
        await ctx.watch();
    } else {
        await ctx.rebuild();
        await ctx.dispose();
    }
}

main().catch((e) => {
    console.error(e);
    process.exit(1);
});
