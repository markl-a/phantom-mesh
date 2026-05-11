/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        phantom: {
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
