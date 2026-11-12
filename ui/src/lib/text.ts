/** "1 file", "3 files". */
export const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;
