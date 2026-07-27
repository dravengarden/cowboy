import {
  LanguageDescription,
  type LanguageSupport,
} from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { languageFromPath } from "../../syntaxLanguages";

const loadedLanguages = new Map<string, Promise<LanguageSupport>>();
const CODEMIRROR_LANGUAGE_ALIASES: Record<string, string> = {
  // CodeMirror has no dedicated systemd grammar. Its properties/INI mode
  // correctly recognizes sections, assignments, comments, and escapes without
  // pretending to understand unit-specific directives.
  systemd: "ini",
};

export function languageDescriptionForPath(
  path: string,
): LanguageDescription | null {
  return LanguageDescription.matchFilename(languages, path) ??
    LanguageDescription.matchLanguageName(
      languages,
      CODEMIRROR_LANGUAGE_ALIASES[languageFromPath(path)] ??
        languageFromPath(path),
      false,
    );
}

export function loadCodeLanguage(
  path: string,
): Promise<LanguageSupport | null> {
  const description = languageDescriptionForPath(path);
  if (!description) return Promise.resolve(null);
  let loaded = loadedLanguages.get(description.name);
  if (!loaded) {
    loaded = description.load();
    loadedLanguages.set(description.name, loaded);
  }
  return loaded;
}
