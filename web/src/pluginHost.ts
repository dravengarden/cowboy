import * as React from "react";
import Visibility from "@mui/icons-material/Visibility";
import VisibilityOff from "@mui/icons-material/VisibilityOff";
import {
  Alert,
  Box,
  Button,
  Divider,
  IconButton,
  InputAdornment,
  LinearProgress,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import type { CowboyPluginHost } from "@cowboy/plugin-api";
import { authApi } from "./auth/authApi";
import { PasswordStrength } from "./admin/PasswordStrength";
import { ProductPasskeysPanel } from "./auth/ProductPasskeysPanel";
import { ProviderSurface } from "./ProviderSurface";
import { DeepSeekDetails } from "../../plugins/claude-deepseek/ui/DeepSeekDetails";
import { ProviderUsage } from "./pluginUsage";

declare global {
  interface Window {
    __COWBOY_PLUGIN_HOST?: CowboyPluginHost;
  }
}

export function installCowboyPluginHost(
  components: CowboyPluginHost["components"] = {},
): void {
  globalThis.__COWBOY_PLUGIN_HOST = {
    version: "1.0.0",
    React,
    ui: {
      Alert,
      Box,
      Button,
      Divider,
      IconButton,
      InputAdornment,
      LinearProgress,
      Stack,
      TextField,
      Typography,
    },
    icons: {
      Visibility,
      VisibilityOff,
    },
    components: {
      PasswordStrength,
      PasskeysPanel: ProductPasskeysPanel,
      ProviderSurface,
      ProviderUsage,
      DeepSeekDetails,
      ...components,
    },
    auth: {
      login: (account, password) => authApi.login(account, password),
      register: (account, password) => authApi.register(account, password),
      setup: (token) => authApi.setup(token),
    },
    call: async (pluginId, body) => {
      const response = await fetch(
        `/api/plugins/${encodeURIComponent(pluginId)}/call`,
        {
          method: "POST",
          credentials: "same-origin",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(body ?? {}),
        },
      );
      if (!response.ok) {
        throw new Error(`plugin call failed: HTTP ${String(response.status)}`);
      }
      const contentType = response.headers.get("content-type") ?? "";
      if (contentType.includes("application/json")) {
        return await response.json();
      }
      return await response.text();
    },
  };
}
