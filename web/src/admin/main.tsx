import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { CssBaseline, ThemeProvider } from "@mui/material";
import { AppErrorBoundary } from "../AppErrorBoundary";
import { useThemeMode } from "../theme";
import { AdminApp } from "./AdminApp";

function Root(): React.JSX.Element {
  const { theme } = useThemeMode();
  return (
    <ThemeProvider theme={theme}>
      <CssBaseline enableColorScheme />
      <AppErrorBoundary>
        <AdminApp />
      </AppErrorBoundary>
    </ThemeProvider>
  );
}

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <Root />
    </StrictMode>,
  );
}
