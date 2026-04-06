import * as path from 'path';
import * as glob from 'glob';

// Simple test runner for VS Code integration tests (vitest-style syntax)
interface TestCase {
    name: string;
    fn: () => void | Promise<void>;
}

interface TestSuite {
    name: string;
    tests: TestCase[];
    beforeAll?: () => void | Promise<void>;
    afterAll?: () => void | Promise<void>;
}

const suites: TestSuite[] = [];
let currentSuite: TestSuite | null = null;

// Global test functions (vitest-style)
export function describe(name: string, fn: () => void): void {
    const suite: TestSuite = { name, tests: [] };
    currentSuite = suite;
    fn();
    suites.push(suite);
    currentSuite = null;
}

export function it(name: string, fn: () => void | Promise<void>): void {
    if (currentSuite) {
        currentSuite.tests.push({ name, fn });
    }
}

export function beforeAll(fn: () => void | Promise<void>): void {
    if (currentSuite) {
        currentSuite.beforeAll = fn;
    }
}

export function afterAll(fn: () => void | Promise<void>): void {
    if (currentSuite) {
        currentSuite.afterAll = fn;
    }
}

// Simple expect implementation
export function expect<T>(actual: T) {
    return {
        toBe(expected: T) {
            if (actual !== expected) {
                throw new Error(`Expected ${actual} to be ${expected}`);
            }
        },
        toEqual(expected: T) {
            if (JSON.stringify(actual) !== JSON.stringify(expected)) {
                throw new Error(`Expected ${JSON.stringify(actual)} to equal ${JSON.stringify(expected)}`);
            }
        },
        toBeTruthy() {
            if (!actual) {
                throw new Error(`Expected ${actual} to be truthy`);
            }
        },
        toBeFalsy() {
            if (actual) {
                throw new Error(`Expected ${actual} to be falsy`);
            }
        },
        toBeDefined() {
            if (actual === undefined) {
                throw new Error(`Expected value to be defined`);
            }
        },
        toBeUndefined() {
            if (actual !== undefined) {
                throw new Error(`Expected ${actual} to be undefined`);
            }
        },
        toBeGreaterThanOrEqual(expected: number) {
            if (typeof actual !== 'number' || actual < expected) {
                throw new Error(`Expected ${actual} to be >= ${expected}`);
            }
        },
        toBeLessThanOrEqual(expected: number) {
            if (typeof actual !== 'number' || actual > expected) {
                throw new Error(`Expected ${actual} to be <= ${expected}`);
            }
        },
        toContain(item: unknown) {
            if (Array.isArray(actual)) {
                if (!actual.includes(item)) {
                    throw new Error(`Expected array to contain ${item}`);
                }
            } else if (typeof actual === 'string') {
                if (!actual.includes(item as string)) {
                    throw new Error(`Expected string to contain ${item}`);
                }
            } else {
                throw new Error(`toContain only works with arrays and strings`);
            }
        },
        toMatch(pattern: RegExp | string) {
            if (typeof actual !== 'string') {
                throw new Error(`toMatch requires a string`);
            }
            const regex = typeof pattern === 'string' ? new RegExp(pattern) : pattern;
            if (!regex.test(actual)) {
                throw new Error(`Expected "${actual}" to match ${pattern}`);
            }
        },
    };
}

// Test runner
async function runSuites(): Promise<{ passed: number; failed: number }> {
    let passed = 0;
    let failed = 0;

    for (const suite of suites) {
        console.log(`\n  ${suite.name}`);

        if (suite.beforeAll) {
            try {
                await suite.beforeAll();
            } catch (err) {
                console.error(`    ✗ beforeAll failed: ${err}`);
                failed += suite.tests.length;
                continue;
            }
        }

        for (const test of suite.tests) {
            try {
                await test.fn();
                console.log(`    ✓ ${test.name}`);
                passed++;
            } catch (err) {
                console.error(`    ✗ ${test.name}`);
                console.error(`      ${err}`);
                failed++;
            }
        }

        if (suite.afterAll) {
            try {
                await suite.afterAll();
            } catch (err) {
                console.error(`    afterAll failed: ${err}`);
            }
        }
    }

    return { passed, failed };
}

export async function run(): Promise<void> {
    const testsRoot = path.resolve(__dirname, '.');

    // Import all test files
    const files = await glob.glob('**/**.test.js', { cwd: testsRoot });

    for (const f of files) {
        await import(path.resolve(testsRoot, f));
    }

    console.log('\nRunning VS Code Extension Tests\n');

    const { passed, failed } = await runSuites();

    console.log(`\n  ${passed} passing`);
    if (failed > 0) {
        console.log(`  ${failed} failing`);
        throw new Error(`${failed} tests failed.`);
    }
}
