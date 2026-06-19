/** Tailwind scans the Maud templates (class strings live in .rs files). */
module.exports = {
  content: ["./crates/hub/src/**/*.rs"],
  theme: {
    extend: {
      colors: {
        // Last Monitor identity: deep navy-black ground, teal/aqua accent.
        ink: "#0B0E14",
        panel: "#10151F",
        panel2: "#141A26",
        line: "#1E2632",
        teal: {
          DEFAULT: "#34E1C4",
          fg: "#06231F", // text on teal
        },
      },
      fontFamily: {
        mono: [
          "ui-monospace",
          "SF Mono",
          "JetBrains Mono",
          "Cascadia Code",
          "Menlo",
          "monospace",
        ],
      },
    },
  },
  plugins: [],
};
