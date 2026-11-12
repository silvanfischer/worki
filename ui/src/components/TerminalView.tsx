import "@xterm/xterm/css/xterm.css";

import { openUrl } from "@tauri-apps/plugin-opener";
import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import { type ITheme, Terminal } from "@xterm/xterm";
import { useEffect, useRef } from "react";

import { api, errorMessage } from "../lib/api";
import { useUi } from "../lib/store";
import { useIsDark } from "../lib/theme";
import { toast } from "../lib/toast";

const darkTheme: ITheme = {
  background: "#1c1c1e",
  foreground: "#e5e5ea",
  cursor: "#e5e5ea",
  cursorAccent: "#1c1c1e",
  selectionBackground: "#3a3a5c",
  black: "#1c1c1e",
  red: "#ff6b6b",
  green: "#3fb950",
  yellow: "#e3b341",
  blue: "#6cb6ff",
  magenta: "#c297ff",
  cyan: "#56d4dd",
  white: "#e5e5ea",
  brightBlack: "#6e6e73",
  brightRed: "#ff8787",
  brightGreen: "#56d364",
  brightYellow: "#f2cc60",
  brightBlue: "#91cbff",
  brightMagenta: "#d2a8ff",
  brightCyan: "#76e3ea",
  brightWhite: "#ffffff",
};

const lightTheme: ITheme = {
  background: "#ffffff",
  foreground: "#1d1d1f",
  cursor: "#1d1d1f",
  cursorAccent: "#ffffff",
  selectionBackground: "#c7d2fe",
  black: "#1d1d1f",
  red: "#cf222e",
  green: "#1a7f37",
  yellow: "#9a6700",
  blue: "#0969da",
  magenta: "#8250df",
  cyan: "#1b7c83",
  white: "#6e7781",
  brightBlack: "#57606a",
  brightRed: "#a40e26",
  brightGreen: "#2da44e",
  brightYellow: "#bf8700",
  brightBlue: "#218bff",
  brightMagenta: "#a475f9",
  brightCyan: "#3192aa",
  brightWhite: "#8c959f",
};

export function TerminalView({ sessionId, active }: { sessionId: string; active: boolean }) {
  const container = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const dark = useIsDark();
  const fontSize = useUi((s) => s.terminalFontSize);
  const initial = useRef({ dark, fontSize });

  useEffect(() => {
    const term = new Terminal({
      allowProposedApi: true,
      fontFamily: '"SF Mono", Menlo, Monaco, monospace',
      fontSize: initial.current.fontSize,
      lineHeight: 1.15,
      cursorBlink: true,
      scrollback: 10_000,
      theme: initial.current.dark ? darkTheme : lightTheme,
      macOptionClickForcesSelection: true,
      // Links a program marks itself (OSC 8), such as login links; xterm would ask and then
      // call window.open, which opens nothing in the app.
      linkHandler: { activate: (_e, uri) => /^https?:\/\//i.test(uri) && openUrl(uri) },
    });
    termRef.current = term;
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = "11";
    term.loadAddon(new WebLinksAddon((_e, uri) => openUrl(uri)));
    term.attachCustomKeyEventHandler((e) => !(e.ctrlKey && e.key === "`"));
    term.open(container.current!);
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose());
      term.loadAddon(webgl);
    } catch {
      // Falls back to the DOM renderer.
    }

    let disposed = false;
    let lastSize = "";
    const fitNow = () => {
      if (disposed || !container.current?.offsetWidth || !fit.proposeDimensions()) return false;
      fit.fit();
      const size = `${term.cols}x${term.rows}`;
      if (size !== lastSize) {
        lastSize = size;
        api.resizeSession(sessionId, term.cols, term.rows).catch(() => {});
      }
      return true;
    };
    // Right after mounting, the renderer may not have measured the font yet; a fit then does
    // nothing, and no later resize comes to retry it.
    fitNow();
    let frame = 0;
    let tries = 0;
    const settle = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        fitNow();
        if (++tries < 10) settle();
      });
    };
    settle();
    document.fonts?.ready.then(() => fitNow());

    let sinkId: number | null = null;
    api
      .attachSession(sessionId, (bytes) => term.write(bytes))
      .then((id) => {
        if (disposed) api.detachSession(sessionId, id);
        else sinkId = id;
      })
      .catch((e) => term.write(`\r\n\x1b[31m${String(e)}\x1b[0m\r\n`));

    const osc52 = term.parser.registerOscHandler(52, (data) => {
      const payload = data.slice(data.indexOf(";") + 1);
      if (!payload || payload === "?") return true;
      let text: string;
      try {
        text = new TextDecoder().decode(Uint8Array.from(atob(payload), (c) => c.charCodeAt(0)));
      } catch {
        return true;
      }
      api.writeClipboard(text).catch((e) => toast(errorMessage(e)));
      return true;
    });

    const input = term.onData((data) => api.writeSession(sessionId, data).catch(() => {}));

    let resizeFrame = 0;
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(resizeFrame);
      resizeFrame = requestAnimationFrame(fitNow);
    });
    observer.observe(container.current!);
    const el = container.current!;
    const refit = () => {
      cancelAnimationFrame(resizeFrame);
      resizeFrame = requestAnimationFrame(fitNow);
    };
    el.addEventListener("worki-refit", refit);
    // WebKit enables Copy only for a real text selection, and the terminal draws its own: keep
    // the selected text selected in xterm's hidden textarea so Cmd+C has something to copy.
    const mirror = term.onSelectionChange(() => {
      const area = term.textarea;
      if (!area) return;
      area.value = term.hasSelection() ? term.getSelection() : "";
      if (area.value) area.select();
    });
    const copy = (e: ClipboardEvent) => {
      if (!term.hasSelection() || !el.contains(document.activeElement)) return;
      e.clipboardData?.setData("text/plain", term.getSelection());
      e.preventDefault();
    };
    document.addEventListener("copy", copy, true);

    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      cancelAnimationFrame(resizeFrame);
      observer.disconnect();
      el.removeEventListener("worki-refit", refit);
      document.removeEventListener("copy", copy, true);
      mirror.dispose();
      osc52.dispose();
      input.dispose();
      if (sinkId !== null) api.detachSession(sessionId, sinkId);
      term.dispose();
      termRef.current = null;
    };
  }, [sessionId]);

  useEffect(() => {
    if (active) termRef.current?.focus();
  }, [active]);

  useEffect(() => {
    const term = termRef.current;
    if (term) term.options.theme = dark ? darkTheme : lightTheme;
  }, [dark]);

  useEffect(() => {
    const term = termRef.current;
    if (!term || term.options.fontSize === fontSize) return;
    term.options.fontSize = fontSize;
    container.current?.dispatchEvent(new Event("worki-refit"));
  }, [fontSize]);

  return (
    <div className="h-full w-full bg-panel pt-1.5 pl-2">
      <div ref={container} className="h-full w-full" />
    </div>
  );
}
