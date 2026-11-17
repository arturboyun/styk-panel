/** Rough count of added and removed lines, for a "+3 -1" badge. */
export function changedLines(before: string, after: string): { added: number; removed: number } {
  const a = before.split("\n");
  const b = after.split("\n");
  const setA = new Map<string, number>();
  a.forEach((l) => setA.set(l, (setA.get(l) ?? 0) + 1));
  let common = 0;
  b.forEach((l) => {
    const n = setA.get(l) ?? 0;
    if (n > 0) {
      common++;
      setA.set(l, n - 1);
    }
  });
  return { added: b.length - common, removed: a.length - common };
}
