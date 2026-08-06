import js from '@eslint/js'
import globals from 'globals'
import reactHooks from 'eslint-plugin-react-hooks'
import tseslint from 'typescript-eslint'

export default tseslint.config(
    { ignores: ['dist', 'src-tauri/target', 'node_modules'] },
    {
        extends: [js.configs.recommended, ...tseslint.configs.recommendedTypeChecked],
        files: ['src/**/*.{ts,tsx}'],
        languageOptions: {
            ecmaVersion: 2022,
            globals: globals.browser,
            parserOptions: {
                project: ['./tsconfig.json'],
                tsconfigRootDir: import.meta.dirname,
            },
        },
        plugins: { 'react-hooks': reactHooks },
        rules: {
            ...reactHooks.configs.recommended.rules,

            /*
             * `any` disables the type checking that the zod boundary layers
             * exist to provide. `unknown` plus a parse is always available and
             * is what the IPC and API clients already do.
             */
            '@typescript-eslint/no-explicit-any': 'error',

            /*
             * A floating promise in this app is a silently swallowed IPC
             * failure — an install that reported nothing, a setting that never
             * saved. `void` marks the deliberate fire-and-forget ones.
             */
            '@typescript-eslint/no-floating-promises': 'error',
            '@typescript-eslint/no-misused-promises': [
                'error',
                { checksVoidReturn: { attributes: false } },
            ],

            '@typescript-eslint/no-unused-vars': [
                'error',
                { argsIgnorePattern: '^_', varsIgnorePattern: '^_' },
            ],

            /*
             * The frontend must never reach the network directly: requests go
             * through Rust so the bearer token stays out of the webview. This
             * rule is the mechanical half of that guarantee.
             */
            'no-restricted-globals': [
                'error',
                {
                    name: 'fetch',
                    message:
                        'Use the api client (src/lib/api/client.ts) — HTTP goes through Rust so the token never enters the webview.',
                },
            ],
            'no-restricted-properties': [
                'error',
                {
                    object: 'window',
                    property: 'fetch',
                    message: 'Use the api client (src/lib/api/client.ts).',
                },
            ],
            'no-restricted-imports': [
                'error',
                {
                    paths: [
                        {
                            name: '@tauri-apps/api/core',
                            importNames: ['invoke'],
                            message:
                                'Use src/lib/ipc — every command is parsed through a schema there.',
                        },
                    ],
                },
            ],
        },
    },
    {
        // The IPC layer is the one place `invoke` is legitimate.
        files: ['src/lib/ipc/index.ts'],
        rules: { 'no-restricted-imports': 'off' },
    }
)
