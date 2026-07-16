export function seekRatioFromBounds(
  clientX: number,
  bounds: Pick<DOMRect, "left" | "width">,
) {
  if (bounds.width <= 0) return undefined;
  return Math.max(0, Math.min(1, (clientX - bounds.left) / bounds.width));
}

export function roundedMediaSecond(seconds: number | undefined) {
  return seconds !== undefined && Number.isFinite(seconds)
    ? Math.max(0, Math.floor(seconds))
    : 0;
}
