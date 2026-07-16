import { useEffect, useState } from "react";

const reducedMotionQuery = "(prefers-reduced-motion: reduce)";

type MatchMedia = (query: string) => Pick<MediaQueryList, "matches">;

export function readReducedMotionPreference(matchMedia?: MatchMedia) {
  const query = matchMedia
    ?? (typeof window === "undefined" ? undefined : window.matchMedia.bind(window));
  return query?.(reducedMotionQuery).matches ?? false;
}

export function usePrefersReducedMotion() {
  const [prefersReducedMotion, setPrefersReducedMotion] = useState(
    () => readReducedMotionPreference(),
  );

  useEffect(() => {
    const media = window.matchMedia(reducedMotionQuery);
    setPrefersReducedMotion(media.matches);

    function handleChange(event: MediaQueryListEvent) {
      setPrefersReducedMotion(event.matches);
    }

    media.addEventListener("change", handleChange);
    return () => media.removeEventListener("change", handleChange);
  }, []);

  return prefersReducedMotion;
}
