import { useState } from "react";

import { errorMessage } from "./api";
import { toast } from "./toast";

/** Copies text to the clipboard; `copied` is true for a moment afterwards. */
export function useCopy() {
  const [copied, setCopied] = useState(false);
  const copy = (text: string) =>
    navigator.clipboard.writeText(text).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      },
      (e) => toast(errorMessage(e)),
    );
  return { copied, copy };
}
