import { type ElementType } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";

const iconSwapTransition = { type: "spring", duration: 0.3, bounce: 0 } as const;

export function AnimatedActionIcon({
  activeKey,
  icons,
}: {
  activeKey: string;
  icons: Record<string, ElementType>;
}) {
  const reducedMotion = useReducedMotion() ?? false;
  const Icon = icons[activeKey];

  if (reducedMotion) {
    return (
      <span
        className="relative inline-flex size-4 shrink-0 items-center justify-center"
        data-icon="inline-start"
      >
        <Icon />
      </span>
    );
  }

  return (
    <span className="relative inline-flex size-4 shrink-0 items-center justify-center" data-icon="inline-start">
      <AnimatePresence initial={false} mode="wait">
        <motion.span
          animate={{ opacity: 1, scale: 1, filter: "blur(0px)" }}
          className="absolute inset-0 flex items-center justify-center"
          exit={{ opacity: 0, scale: 0.25, filter: "blur(4px)" }}
          initial={{ opacity: 0, scale: 0.25, filter: "blur(4px)" }}
          key={activeKey}
          transition={iconSwapTransition}
        >
          <Icon />
        </motion.span>
      </AnimatePresence>
    </span>
  );
}
