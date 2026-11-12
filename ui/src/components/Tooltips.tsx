import { useEffect, useLayoutEffect, useRef, useState } from "react";

const DELAY_MS = 450;
const GAP = 6;
const EDGE = 8;

interface Tip {
  text: string;
  x: number;
  y: number;
  above: boolean;
}

/** Shows any element's `title` as a styled hint on hover, in place of the slow native tooltip. */
export function Tooltips() {
  const [tip, setTip] = useState<Tip | null>(null);

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let current: Element | null = null;
    let borrowedTitle: string | null = null;

    const hide = () => {
      clearTimeout(timer);
      setTip(null);
    };

    // The title is only borrowed while hovering, so screen readers keep it as the accessible name
    // and React keeps owning the attribute.
    const release = () => {
      hide();
      if (current && borrowedTitle !== null) {
        current.removeAttribute("data-tip");
        if (!current.hasAttribute("title")) current.setAttribute("title", borrowedTitle);
      }
      current = null;
      borrowedTitle = null;
    };

    const over = (e: PointerEvent) => {
      const el = (e.target as Element | null)?.closest?.("[title], [data-tip]") ?? null;
      if (el === current) return;
      release();
      if (!el) return;
      const title = el.getAttribute("title");
      if (title) {
        borrowedTitle = title;
        el.setAttribute("data-tip", title);
        el.removeAttribute("title");
      }
      const text = el.getAttribute("data-tip");
      current = el;
      if (!text) return;
      timer = setTimeout(() => {
        const r = el.getBoundingClientRect();
        const above = r.bottom + 40 > window.innerHeight;
        setTip({ text, x: r.left + r.width / 2, y: above ? r.top - GAP : r.bottom + GAP, above });
      }, DELAY_MS);
    };

    const root = document.documentElement;
    document.addEventListener("pointerover", over);
    root.addEventListener("pointerleave", release);
    document.addEventListener("pointerdown", hide, true);
    document.addEventListener("keydown", hide, true);
    document.addEventListener("scroll", hide, true);
    window.addEventListener("blur", release);
    return () => {
      release();
      document.removeEventListener("pointerover", over);
      root.removeEventListener("pointerleave", release);
      document.removeEventListener("pointerdown", hide, true);
      document.removeEventListener("keydown", hide, true);
      document.removeEventListener("scroll", hide, true);
      window.removeEventListener("blur", release);
    };
  }, []);

  if (!tip) return null;
  return <TipBox tip={tip} />;
}

/** Centred on the element, but kept inside the window. */
function TipBox({ tip }: { tip: Tip }) {
  const ref = useRef<HTMLDivElement>(null);
  const [left, setLeft] = useState<number | null>(null);
  useLayoutEffect(() => {
    const width = ref.current?.offsetWidth ?? 0;
    setLeft(Math.min(Math.max(tip.x - width / 2, EDGE), window.innerWidth - width - EDGE));
  }, [tip]);
  return (
    <div
      ref={ref}
      role="tooltip"
      className="pointer-events-none fixed z-[100] max-w-[260px] rounded-md border border-line bg-elevated px-2 py-1 text-center text-[11.5px] leading-snug whitespace-pre-line text-fg shadow-lg"
      style={{
        left: left ?? 0,
        top: tip.y,
        transform: tip.above ? "translateY(-100%)" : undefined,
        visibility: left === null ? "hidden" : undefined,
      }}
    >
      {tip.text}
    </div>
  );
}
