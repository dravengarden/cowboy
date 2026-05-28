import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { CssBaseline, ThemeProvider } from "@mui/material";
import { App } from "./App";
import { useThemeMode } from "./theme";

function Root(): React.JSX.Element {
  const { theme, mode, cycle } = useThemeMode();
  return (
    <ThemeProvider theme={theme}>
      <CssBaseline />
      <App themeMode={mode} onToggleTheme={cycle} />
    </ThemeProvider>
  );
}

const el = document.getElementById("root");
if (el) {
  createRoot(el).render(
    <StrictMode>
      <Root />
    </StrictMode>,
  );
}
