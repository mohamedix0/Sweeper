import tsParser from '@typescript-eslint/parser';
import tsPlugin from '@typescript-eslint/eslint-plugin';
import reactHooks from 'eslint-plugin-react-hooks';

export default [
  {
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      parser: tsParser,
      parserOptions: {
        ecmaVersion: 'latest',
        sourceType: 'module',
        ecmaFeatures: { jsx: true },
      },
    },
    plugins: {
      '@typescript-eslint': tsPlugin,
      'react-hooks': reactHooks,
    },
    rules: {
      'no-eval': 'error',
      'no-implied-eval': 'error',
      'no-new-func': 'error',
      'no-restricted-imports': [
        'error',
        {
          paths: [
            { name: '@tauri-apps/plugin-fs', message: 'Tauri fs plugin is forbidden in Sweeper frontend.' },
            { name: '@tauri-apps/plugin-shell', message: 'Tauri shell plugin is forbidden in Sweeper frontend.' },
            { name: '@tauri-apps/api/fs', message: 'Tauri fs API is forbidden in Sweeper frontend.' },
            { name: '@tauri-apps/api/shell', message: 'Tauri shell API is forbidden in Sweeper frontend.' },
          ],
        },
      ],
      'no-restricted-properties': [
        'error',
        {
          object: 'React',
          property: 'createElement',
          message: 'Use JSX; dynamic element creation is restricted in this codebase.',
        },
      ],
      'no-restricted-syntax': [
        'error',
        {
          selector: "JSXAttribute[name.name='dangerouslySetInnerHTML']",
          message: 'dangerouslySetInnerHTML is forbidden by Static Gate A frontend policy.',
        },
      ],
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'warn',
      '@typescript-eslint/no-explicit-any': 'off',
    },
  },
  {
    ignores: ['dist/**', 'node_modules/**', 'playwright-report/**', 'test-results/**'],
  },
];
