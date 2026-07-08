// T-D-F-02: ESLint flat config for Nebula
// @typescript-eslint + react + react-hooks + eslint-config-prettier
import tseslint from "@typescript-eslint/eslint-plugin";
import tsparser from "@typescript-eslint/parser";
import react from "eslint-plugin-react";
import reactHooks from "eslint-plugin-react-hooks";
import prettier from "eslint-config-prettier";

export default [
  // Global ignores
  {
    ignores: ["dist/", "src-tauri/", "coverage/", "node_modules/"],
  },

  // Base TypeScript + React files
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      parser: tsparser,
      parserOptions: {
        ecmaVersion: "latest",
        sourceType: "module",
        ecmaFeatures: { jsx: true },
      },
    },
    plugins: {
      "@typescript-eslint": tseslint,
      react,
      "react-hooks": reactHooks,
    },
    settings: {
      react: {
        // Preact uses react-compat; tell eslint-plugin-react to recognize it
        version: "detect",
      },
    },
    rules: {
      // ---- TypeScript rules ----
      ...tseslint.configs.recommended.rules,
      "@typescript-eslint/no-unused-vars": [
        "warn",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
      "@typescript-eslint/no-explicit-any": "warn",
      "@typescript-eslint/explicit-function-return-type": "off",
      "@typescript-eslint/no-non-null-assertion": "off",

      // ---- React rules ----
      ...react.configs.recommended.rules,
      ...react.configs["jsx-runtime"].rules,
      "react/react-in-jsx-scope": "off", // Preact + jsx-runtime: no React import needed
      "react/prop-types": "off", // TypeScript handles prop types
      "react/no-unknown-property": "off", // Preact uses `class` instead of `className`

      // ---- React Hooks rules ----
      // Only enable rules-of-hooks (call order) + exhaustive-deps;
      // disable immutability/refs which conflict with Preact Signals (.value assignment).
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "warn",
      "react-hooks/immutability": "off", // Preact Signals: .value assignment is standard
      "react-hooks/refs": "off", // Preact: ref writes during render for imperative values

      // ---- Prettier compatibility (disables formatting rules) ----
      ...prettier.rules,
    },
  },

  // Test files — relax some rules
  {
    files: ["src/**/*.{test,spec}.{ts,tsx}"],
    rules: {
      "@typescript-eslint/no-explicit-any": "off",
    },
  },
];
