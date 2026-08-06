/** @type {import('tailwindcss').Config} */
// Mirrors app/tailwind.config.js so dashboard + desktop share the
// spectyn-* color tokens. Any token added here MUST be back-ported
// to app/tailwind.config.js (and vice versa) — otherwise the F201+
// component reuse story breaks.
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        spectyn: {
          bg: "#0f0f1a",
          card: "#1a1a2e",
          border: "#2a2a3e",
          primary: "#8ab4f8",
          secondary: "#bb86fc",
          success: "#4caf50",
          warning: "#ff9800",
          danger: "#dc3545",
          text: "#e0e0e0",
          muted: "#8888aa",
        },
      },
    },
  },
  plugins: [],
};
