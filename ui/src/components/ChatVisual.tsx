import { save } from "@tauri-apps/plugin-dialog";
import DOMPurify from "dompurify";
import { Download, Maximize2, X, ZoomIn, ZoomOut } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { api, errorMessage } from "../lib/api";
import { useIsDark } from "../lib/theme";
import { toast } from "../lib/toast";

/** Code block languages the chat draws instead of showing as code. */
export const VISUAL_LANGUAGES = ["mermaid", "svg", "html"];

/** A diagram or drawing from the agent's answer. Shows the source until it's complete enough to
 * draw, and when it can't be drawn. */
export function ChatVisual({ language, source }: { language: string; source: string }) {
  if (language === "mermaid") return <MermaidDiagram source={source} />;
  if (language === "html") return <HtmlVisual source={source} />;
  return <SvgDrawing source={source} />;
}

/** The theme colors and font, for HTML visuals to match worki. */
const THEME_VARIABLES = ["panel", "elevated", "fg", "muted", "faint", "line", "hover", "accent", "accent-fg", "added", "removed", "warn"];

/** A complete page around the agent's HTML: worki's theme as CSS variables, and a script that
 * reports the content's height so the frame can grow with it. */
function htmlPage(source: string, dark: boolean): string {
  const style = getComputedStyle(document.documentElement);
  const vars = THEME_VARIABLES.map((v) => `--${v}: ${style.getPropertyValue(`--${v}`).trim()};`).join(" ");
  return `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width">
<style>
:root { ${vars} color-scheme: ${dark ? "dark" : "light"}; }
html, body { margin: 0; background: transparent; color: var(--fg); font: 13px/1.5 ${FONT}; }
body { display: flow-root; }
*, *::before, *::after { box-sizing: border-box; }
</style></head><body>${source}
<script>
const report = () => parent.postMessage({ worki: "height", height: Math.ceil(document.body.getBoundingClientRect().height) }, "*");
new ResizeObserver(report).observe(document.body);
addEventListener("load", report);
report();
</script></body></html>`;
}

const MAX_HTML_HEIGHT = 1600;

/** An HTML visual (charts, widgets) in a sandboxed frame: its scripts run, but it can't reach worki,
 * the page around it, or open anything. */
function HtmlVisual({ source }: { source: string }) {
  const dark = useIsDark();
  const [page, setPage] = useState<string | null>(null);
  const [open, setOpen] = useState(false);

  // Waits for the agent to pause writing, so the frame isn't reloaded with every few characters.
  useEffect(() => {
    const timer = setTimeout(() => setPage(htmlPage(source, dark)), 400);
    return () => clearTimeout(timer);
  }, [source, dark]);

  if (!page) return <Source source={source} />;
  return (
    <div className="group relative my-2">
      <HtmlFrame page={page} className="" />
      <div className="absolute top-1.5 right-1.5 flex gap-1 opacity-0 transition-opacity group-hover:opacity-100">
        <ToolButton title="Open full-window" onClick={() => setOpen(true)}>
          <Maximize2 size={11} />
        </ToolButton>
        <ToolButton title="Save as HTML" onClick={() => saveHtml(page)}>
          <Download size={11} />
          HTML
        </ToolButton>
      </div>
      {open &&
        createPortal(
          <Overlay onClose={() => setOpen(false)} tools={null}>
            <div className="h-full overflow-auto rounded-lg bg-panel p-4">
              <HtmlFrame page={page} className="" />
            </div>
          </Overlay>,
          document.body,
        )}
    </div>
  );
}

function HtmlFrame({ page, className }: { page: string; className: string }) {
  const frame = useRef<HTMLIFrameElement>(null);
  const [height, setHeight] = useState(120);
  useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      if (e.source === frame.current?.contentWindow && e.data?.worki === "height")
        setHeight(Math.min(MAX_HTML_HEIGHT, Math.max(40, Number(e.data.height) || 0)));
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);
  return (
    <iframe
      ref={frame}
      title="Visual"
      sandbox="allow-scripts"
      srcDoc={page}
      style={{ height }}
      className={`box-content block w-full bg-transparent ${className}`}
    />
  );
}

async function saveHtml(page: string) {
  try {
    const path = await save({ defaultPath: "visual.html", filters: [{ name: "HTML", extensions: ["html"] }] });
    if (!path) return;
    await api.saveFile(path, new TextEncoder().encode(page));
    toast(`Saved ${path.split("/").pop()}`, "info");
  } catch (e) {
    toast(errorMessage(e));
  }
}

/** Full-window layer with a toolbar; closes on Esc or a click outside its content. */
function Overlay({ tools, onClose, children }: { tools: ReactNode; onClose: () => void; children: ReactNode }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="fixed inset-0 z-50 flex flex-col bg-black/70 backdrop-blur-sm" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="flex shrink-0 items-center justify-end gap-1 p-2">
        {tools}
        <ToolButton title="Close (Esc)" onClick={onClose}>
          <X size={12} />
        </ToolButton>
      </div>
      <div className="min-h-0 flex-1 px-6 pb-6">{children}</div>
    </div>
  );
}

function SvgDrawing({ source }: { source: string }) {
  const clean = DOMPurify.sanitize(source, { USE_PROFILES: { svg: true, svgFilters: true } });
  if (!/<svg[\s>]/i.test(clean) || !/<\/svg>\s*$/i.test(source.trim())) return <Source source={source} />;
  return <Drawing svg={clean} name="drawing" className="bg-white" background="#ffffff" />;
}

const FONT = '-apple-system, BlinkMacSystemFont, "SF Pro Text", "Inter", system-ui, sans-serif';

/** Mermaid's base theme in worki's colors: soft accent-tinted boxes, quiet lines. */
const LIGHT = {
  darkMode: false,
  fontFamily: FONT,
  fontSize: "14px",
  background: "#ffffff",
  primaryColor: "#eaf2fd",
  primaryBorderColor: "#93b9ec",
  primaryTextColor: "#1d1d1f",
  secondaryColor: "#f4f4f5",
  secondaryBorderColor: "#d4d4d8",
  tertiaryColor: "#fafafa",
  tertiaryBorderColor: "#e4e4e7",
  lineColor: "#9ca3af",
  textColor: "#1d1d1f",
  edgeLabelBackground: "#ffffff",
  clusterBkg: "#f7f7f9",
  clusterBorder: "#e4e4e7",
  noteBkgColor: "#fff7e6",
  noteBorderColor: "#f5d38a",
  actorBkg: "#eaf2fd",
  actorBorder: "#93b9ec",
  signalColor: "#6e6e73",
  labelBoxBkgColor: "#eaf2fd",
};

const DARK = {
  ...LIGHT,
  darkMode: true,
  background: "#1c1c1e",
  primaryColor: "#1f2d42",
  primaryBorderColor: "#4c97f0",
  primaryTextColor: "#f5f5f7",
  secondaryColor: "#2c2c2e",
  secondaryBorderColor: "#48484a",
  tertiaryColor: "#242426",
  tertiaryBorderColor: "#3a3a3c",
  lineColor: "#8e8e93",
  textColor: "#f5f5f7",
  edgeLabelBackground: "#1c1c1e",
  clusterBkg: "#232325",
  clusterBorder: "#3a3a3c",
  noteBkgColor: "#3a3220",
  noteBorderColor: "#7a6224",
  actorBkg: "#1f2d42",
  actorBorder: "#4c97f0",
  signalColor: "#a1a1a6",
  labelBoxBkgColor: "#1f2d42",
};

const MERMAID_CSS = `
  .node rect, .node polygon, .node circle, .node ellipse, .node path { stroke-width: 1.25px; }
  .node rect { rx: 8px; ry: 8px; }
  .cluster rect { rx: 12px; ry: 12px; }
  .edgePath .path, .flowchart-link { stroke-width: 1.5px; }
  .edgeLabel rect { rx: 4px; ry: 4px; }
  .label, .nodeLabel, text { font-weight: 450; }
`;

function MermaidDiagram({ source }: { source: string }) {
  const dark = useIsDark();
  const id = useId().replace(/[^a-zA-Z0-9]/g, "");
  const [svg, setSvg] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const timer = setTimeout(async () => {
      const { default: mermaid } = await import("mermaid");
      mermaid.initialize({
        startOnLoad: false,
        securityLevel: "strict",
        theme: "base",
        themeVariables: dark ? DARK : LIGHT,
        themeCSS: MERMAID_CSS,
        // Plain SVG text instead of HTML labels, so the diagram also converts to PNG.
        htmlLabels: false,
        flowchart: { htmlLabels: false, curve: "basis", padding: 14, nodeSpacing: 36, rankSpacing: 48 },
        sequence: { actorMargin: 60, boxMargin: 8, mirrorActors: false },
      });
      try {
        if (!(await mermaid.parse(source, { suppressErrors: true }))) throw new Error("incomplete");
        const { svg } = await mermaid.render(`chat-mermaid-${id}`, source);
        if (!cancelled) {
          setSvg(svg);
          setFailed(false);
        }
      } catch {
        if (!cancelled) setFailed(true);
      }
    }, 150);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [source, dark, id]);

  if (!svg) return <Source source={source} note={failed ? "Couldn't draw this diagram yet." : undefined} />;
  return <Drawing svg={svg} name="diagram" className="border border-line bg-panel" background={dark ? DARK.background : LIGHT.background} />;
}

function Drawing({ svg, name, className, background }: { svg: string; name: string; className: string; background: string }) {
  const box = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  return (
    <div className="group relative my-2">
      <div
        ref={box}
        onClick={() => setOpen(true)}
        title="Click to open"
        className={`flex cursor-zoom-in justify-center overflow-x-auto rounded-md p-3 [&_svg]:h-auto [&_svg]:max-w-full ${className}`}
        dangerouslySetInnerHTML={{ __html: svg }}
      />
      <div className="absolute top-1.5 right-1.5 flex gap-1 opacity-0 transition-opacity group-hover:opacity-100">
        <ToolButton title="Open and zoom" onClick={() => setOpen(true)}>
          <Maximize2 size={11} />
        </ToolButton>
        <DownloadButtons svg={() => box.current?.querySelector("svg") ?? null} name={name} background={background} />
      </div>
      {open && <Viewer svg={svg} name={name} className={className} background={background} onClose={() => setOpen(false)} />}
    </div>
  );
}

function ToolButton({ title, onClick, children }: { title: string; onClick: () => void; children: ReactNode }) {
  return (
    <button
      onClick={onClick}
      title={title}
      className="flex h-6 items-center gap-1 rounded-md border border-line bg-elevated px-1.5 text-[11px] text-muted shadow-sm hover:text-fg"
    >
      {children}
    </button>
  );
}

function DownloadButtons({ svg, name, background }: { svg: () => SVGSVGElement | null; name: string; background: string }) {
  return (
    <>
      {(["svg", "png"] as const).map((format) => (
        <ToolButton key={format} title={`Save as ${format.toUpperCase()}`} onClick={() => download(svg(), name, format, background)}>
          <Download size={11} />
          {format.toUpperCase()}
        </ToolButton>
      ))}
    </>
  );
}

const MIN_ZOOM = 0.1;
const MAX_ZOOM = 20;

/** The drawing full-window: zoom with the buttons, the scroll wheel or a pinch, drag to pan. */
function Viewer({
  svg,
  name,
  className,
  background,
  onClose,
}: {
  svg: string;
  name: string;
  className: string;
  background: string;
  onClose: () => void;
}) {
  const stage = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ zoom: 1, x: 0, y: 0 });
  const drag = useRef<{ x: number; y: number } | null>(null);
  const sized = useMemo(() => withSize(svg), [svg]);

  const fit = useCallback(() => {
    const area = stage.current?.getBoundingClientRect();
    const drawing = content.current;
    if (!area || !drawing?.offsetWidth || !drawing.offsetHeight) return;
    const zoom = Math.min((area.width - 64) / drawing.offsetWidth, (area.height - 64) / drawing.offsetHeight, MAX_ZOOM);
    setView({ zoom, x: 0, y: 0 });
  }, []);

  useLayoutEffect(fit, [fit]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      else if (e.key === "+" || e.key === "=") zoomBy(1.25);
      else if (e.key === "-") zoomBy(0.8);
      else if (e.key === "0") fit();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  /** Zooms keeping the point under `at` (relative to the stage's center) in place. */
  const zoomBy = (factor: number, at = { x: 0, y: 0 }) =>
    setView((v) => {
      const zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, v.zoom * factor));
      const k = zoom / v.zoom;
      return { zoom, x: at.x - (at.x - v.x) * k, y: at.y - (at.y - v.y) * k };
    });

  useEffect(() => {
    const el = stage.current;
    if (!el) return;
    // Not a React handler: wheel listeners must be non-passive to stop the page from scrolling.
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const rect = el.getBoundingClientRect();
      const at = { x: e.clientX - rect.left - rect.width / 2, y: e.clientY - rect.top - rect.height / 2 };
      zoomBy(Math.exp(-e.deltaY * (e.ctrlKey ? 0.01 : 0.002)), at);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  return createPortal(
    <div className="fixed inset-0 z-50 flex flex-col bg-black/70 backdrop-blur-sm" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="flex shrink-0 items-center justify-end gap-1 p-2">
        <ToolButton title="Zoom out (−)" onClick={() => zoomBy(0.8)}>
          <ZoomOut size={12} />
        </ToolButton>
        <span className="w-12 text-center text-[11px] text-white/80 tabular-nums">{Math.round(view.zoom * 100)}%</span>
        <ToolButton title="Zoom in (+)" onClick={() => zoomBy(1.25)}>
          <ZoomIn size={12} />
        </ToolButton>
        <ToolButton title="Fit to window (0)" onClick={fit}>
          Fit
        </ToolButton>
        <span className="w-2" />
        <DownloadButtons svg={() => content.current?.querySelector("svg") ?? null} name={name} background={background} />
        <span className="w-2" />
        <ToolButton title="Close (Esc)" onClick={onClose}>
          <X size={12} />
        </ToolButton>
      </div>
      <div
        ref={stage}
        className="relative min-h-0 flex-1 cursor-grab overflow-hidden active:cursor-grabbing"
        onPointerDown={(e) => {
          drag.current = { x: e.clientX - view.x, y: e.clientY - view.y };
          e.currentTarget.setPointerCapture(e.pointerId);
        }}
        onPointerMove={(e) => {
          const start = drag.current;
          if (start) setView((v) => ({ ...v, x: e.clientX - start.x, y: e.clientY - start.y }));
        }}
        onPointerUp={() => (drag.current = null)}
        onDoubleClick={fit}
      >
        <div
          className="absolute top-1/2 left-1/2 w-max"
          style={{ transform: `translate(-50%, -50%) translate(${view.x}px, ${view.y}px) scale(${view.zoom})` }}
        >
          <div
            ref={content}
            className={`rounded-md p-3 select-none ${className} ${className.includes("bg-") ? "" : "bg-panel"}`}
            dangerouslySetInnerHTML={{ __html: sized }}
          />
        </div>
      </div>
    </div>,
    document.body,
  );
}

/** The drawing with its natural size in pixels: drawings that stretch to their container's
 * width (like Mermaid's) would otherwise collapse where nothing gives them one. */
function withSize(svg: string): string {
  const doc = new DOMParser().parseFromString(svg, "image/svg+xml");
  const root = doc.documentElement;
  if (root.nodeName.toLowerCase() !== "svg") return svg;
  const box = root.getAttribute("viewBox")?.split(/[\s,]+/).map(Number);
  if (box?.length === 4 && box[2] > 0 && box[3] > 0) {
    root.setAttribute("width", String(box[2]));
    root.setAttribute("height", String(box[3]));
  }
  root.style.removeProperty("max-width");
  return new XMLSerializer().serializeToString(root);
}

/** The drawing as a standalone SVG file, with an explicit size so other apps open it right. */
function standalone(svg: SVGSVGElement): { text: string; width: number; height: number } {
  const copy = svg.cloneNode(true) as SVGSVGElement;
  const box = svg.viewBox.baseVal;
  const rect = svg.getBoundingClientRect();
  const width = Math.ceil(box?.width || rect.width || 800);
  const height = Math.ceil(box?.height || rect.height || 600);
  copy.setAttribute("xmlns", "http://www.w3.org/2000/svg");
  copy.setAttribute("width", String(width));
  copy.setAttribute("height", String(height));
  copy.style.maxWidth = "";
  return { text: new XMLSerializer().serializeToString(copy), width, height };
}

function png(svg: { text: string; width: number; height: number }, background: string): Promise<Uint8Array> {
  const scale = 2;
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onerror = () => reject(new Error("Couldn't convert the drawing to PNG"));
    image.onload = () => {
      const canvas = document.createElement("canvas");
      canvas.width = svg.width * scale;
      canvas.height = svg.height * scale;
      const ctx = canvas.getContext("2d");
      if (!ctx) return reject(new Error("Couldn't convert the drawing to PNG"));
      ctx.fillStyle = background;
      ctx.fillRect(0, 0, canvas.width, canvas.height);
      ctx.drawImage(image, 0, 0, canvas.width, canvas.height);
      canvas.toBlob(async (blob) => {
        if (!blob) return reject(new Error("Couldn't convert the drawing to PNG"));
        resolve(new Uint8Array(await blob.arrayBuffer()));
      }, "image/png");
    };
    image.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg.text)}`;
  });
}

async function download(svg: SVGSVGElement | null, name: string, format: "svg" | "png", background: string) {
  if (!svg) return;
  try {
    const path = await save({
      defaultPath: `${name}.${format}`,
      filters: [{ name: format.toUpperCase(), extensions: [format] }],
    });
    if (!path) return;
    const file = standalone(svg);
    const contents = format === "svg" ? new TextEncoder().encode(file.text) : await png(file, background);
    await api.saveFile(path, contents);
    toast(`Saved ${path.split("/").pop()}`, "info");
  } catch (e) {
    toast(errorMessage(e));
  }
}

function Source({ source, note }: { source: string; note?: string }) {
  return (
    <div className="my-2">
      <pre className="overflow-x-auto rounded-md bg-hover px-3 py-2 font-mono fs-chat-xs">{source}</pre>
      {note && <p className="mt-1 fs-chat-xs text-faint">{note}</p>}
    </div>
  );
}
